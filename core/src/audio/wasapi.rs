//! Windows system-audio capture: WASAPI loopback of the default render
//! device ("what you hear").
//!
//! [`WasapiLoopbackSource`] implements [`AudioSource`]. A dedicated thread
//! polls the loopback stream every 10 ms and pushes 32-bit float stereo
//! chunks at 48 kHz (the audio engine converts from the device format).
//!
//! Two properties of loopback capture shape the design:
//! * **No packets while nothing plays.** WASAPI loopback goes quiet when the
//!   device is idle, leaving gaps in the timestamps. That is fine: the
//!   engine's `ContinuityGuard` fills gaps with silence.
//! * **Timestamps** are QPC positions in 100 ns units. They are re-based
//!   onto the shared video epoch ([`SharedEpoch`]), which must be set from
//!   the *same* QPC clock (Windows Graphics Capture's `SystemRelativeTime`).
//!   Chunks that predate the epoch are dropped.
//!
//! **Status:** compiled and cross-checked from macOS only. Runtime behavior
//! needs verification on a Windows machine (`wasapi_loopback_smoke`).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

use super::{AudioChunk, AudioConfig};
use crate::capture::clock::SharedEpoch;
use crate::capture::{AudioSource, CaptureError};

/// Chunks buffered between the capture thread and the encoder (~1.3 s).
const CHANNEL_CAPACITY: usize = 128;
/// How often the capture thread drains the device.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// `WAVE_FORMAT_IEEE_FLOAT` (mmreg.h): 32-bit float PCM.
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
/// WASAPI shared-mode buffer: 200 ms, in 100 ns units.
const BUFFER_DURATION_HNS: i64 = 2_000_000;

fn backend(context: &str, e: windows::core::Error) -> CaptureError {
    CaptureError::Backend(format!("{context}: {e}"))
}

pub struct WasapiLoopbackSource {
    config: AudioConfig,
    epoch: SharedEpoch,
    running: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WasapiLoopbackSource {
    /// `epoch` must be the same [`SharedEpoch`] the video source uses.
    pub fn new(config: AudioConfig, epoch: SharedEpoch) -> Self {
        Self {
            config,
            epoch,
            running: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicU64::new(0)),
            thread: None,
        }
    }

    /// Chunks the capture thread produced but could not deliver.
    pub fn dropped_chunks(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Everything living on the capture thread: COM objects are not `Send`, so
/// they are created and used there only.
struct Session {
    client: IAudioClient,
    capture: IAudioCaptureClient,
}

impl Session {
    fn open(config: &AudioConfig) -> Result<Self, CaptureError> {
        // SAFETY: plain COM calls on the thread that initialized COM; the
        // format struct outlives `Initialize`.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| backend("device enumerator", e))?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| backend("default render device", e))?;
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| backend("activate audio client", e))?;

            let channels = config.channels;
            let bits: u16 = 32;
            let block_align = channels * bits / 8;
            let format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
                nChannels: channels,
                nSamplesPerSec: config.sample_rate,
                nAvgBytesPerSec: config.sample_rate * u32::from(block_align),
                nBlockAlign: block_align,
                wBitsPerSample: bits,
                cbSize: 0,
            };
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    BUFFER_DURATION_HNS,
                    0,
                    &format,
                    None,
                )
                .map_err(|e| backend("initialize loopback stream", e))?;
            let capture: IAudioCaptureClient = client
                .GetService()
                .map_err(|e| backend("capture service", e))?;
            client.Start().map_err(|e| backend("start stream", e))?;
            Ok(Self { client, capture })
        }
    }

    /// Drain every packet currently available into `out`.
    fn drain(
        &self,
        config: &AudioConfig,
        epoch: &SharedEpoch,
        mut out: impl FnMut(AudioChunk),
    ) -> Result<(), CaptureError> {
        // SAFETY: the pointers returned by `GetBuffer` are valid until the
        // matching `ReleaseBuffer`, and we copy out of them before releasing.
        unsafe {
            loop {
                let available = self
                    .capture
                    .GetNextPacketSize()
                    .map_err(|e| backend("packet size", e))?;
                if available == 0 {
                    return Ok(());
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                let mut qpc_100ns = 0u64;
                self.capture
                    .GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        None,
                        Some(&mut qpc_100ns),
                    )
                    .map_err(|e| backend("get buffer", e))?;

                let count = frames as usize * usize::from(config.channels);
                let samples =
                    if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                        vec![0.0f32; count]
                    } else {
                        std::slice::from_raw_parts(data.cast::<f32>(), count).to_vec()
                    };
                self.capture
                    .ReleaseBuffer(frames)
                    .map_err(|e| backend("release buffer", e))?;

                // QPC position (100 ns units) → ns on the video epoch's clock.
                if let Some(timestamp) = epoch.relative(qpc_100ns.saturating_mul(100) as i64) {
                    out(AudioChunk {
                        samples,
                        channels: config.channels,
                        sample_rate: config.sample_rate,
                        timestamp,
                    });
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the stream was started in `open`; stopping twice is harmless.
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

impl AudioSource for WasapiLoopbackSource {
    fn start(&mut self) -> Result<mpsc::Receiver<AudioChunk>, CaptureError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CaptureError::AlreadyStarted);
        }
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), CaptureError>>();

        let config = self.config;
        let epoch = self.epoch.clone();
        let running = Arc::clone(&self.running);
        let dropped = Arc::clone(&self.dropped);

        self.thread = Some(std::thread::spawn(move || {
            // SAFETY: balanced with `CoUninitialize` below, on this thread.
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let session = if com.is_ok() {
                Session::open(&config)
            } else {
                Err(CaptureError::Backend(format!("COM init: {com:?}")))
            };
            let session = match session {
                Ok(session) => {
                    let _ = ready_tx.send(Ok(()));
                    session
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    running.store(false, Ordering::SeqCst);
                    if com.is_ok() {
                        unsafe { CoUninitialize() };
                    }
                    return;
                }
            };

            while running.load(Ordering::SeqCst) {
                std::thread::sleep(POLL_INTERVAL);
                let result = session.drain(&config, &epoch, |chunk| {
                    if tx.try_send(chunk).is_err() {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                });
                if let Err(e) = result {
                    // Device removed / format changed: end the stream; the
                    // engine keeps recording video.
                    eprintln!("clipforge: WASAPI loopback stopped: {e}");
                    break;
                }
            }
            drop(session);
            // SAFETY: matches the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
            // `tx` drops here, which ends the stream for the consumer.
        }));

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(rx),
            Ok(Err(e)) => {
                self.stop();
                Err(e)
            }
            Err(_) => {
                self.stop();
                Err(CaptureError::Backend("audio thread exited early".into()))
            }
        }
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for WasapiLoopbackSource {
    fn drop(&mut self) {
        self.stop();
    }
}
