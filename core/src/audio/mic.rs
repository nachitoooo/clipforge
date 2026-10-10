//! Microphone capture (macOS and Windows) on top of `cpal`.
//!
//! [`CpalMicSource`] implements [`AudioSource`]: it opens the chosen input
//! device (or the system default), and delivers **mono 48 kHz** `f32` chunks
//! stamped on the same clock as the video — `cpal` reports capture instants
//! on `mach_absolute_time` (macOS) / `QueryPerformanceCounter` (Windows), the
//! very clocks ScreenCaptureKit and WASAPI/Graphics Capture use, so
//! [`SharedEpoch`] maps them onto the clip timeline (verified on macOS: the
//! capture instant trails "now" by ~24 ms, constantly).
//!
//! Everything a settings screen needs to change *live* — volume, mute, the
//! level meter — goes through [`MicControls`], shared with the audio thread
//! through atomics, so none of it restarts the stream.
//!
//! **Status:** verified on macOS with a real microphone; the Windows build is
//! compiled and linted but has not been run.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, Sample, SampleFormat, SizedSample, StreamConfig, StreamInstant};
use tokio::sync::mpsc;

use super::level::LevelTap;
use super::resample::{to_mono, LinearResampler};
use super::{AudioChunk, AudioConfig, AudioError};
use crate::capture::clock::SharedEpoch;
use crate::capture::{AudioSource, CaptureError};

pub const MIC_SAMPLE_RATE: u32 = 48_000;
pub const MIC_CHANNELS: u16 = 1;
/// Chunks buffered between the audio callback and the consumer (~1.3 s).
const CHANNEL_CAPACITY: usize = 128;

/// The format the microphone lane is encoded in: mono voice at 96 kb/s (a
/// bitrate both the AudioToolbox and Media Foundation encoders accept).
pub fn mic_audio_config() -> AudioConfig {
    AudioConfig {
        sample_rate: MIC_SAMPLE_RATE,
        channels: MIC_CHANNELS,
        bitrate: 96_000,
    }
}

// ---- live controls ----------------------------------------------------------

/// State shared between the settings UI and the audio thread.
#[derive(Debug)]
pub struct MicControls {
    gain_bits: AtomicU32,
    muted: AtomicBool,
    /// Peak after gain and mute — what the meter shows.
    pub level: LevelTap,
    /// Raw input before gain: tells "no signal at all" (permission denied,
    /// dead device) apart from "muted".
    pub input: LevelTap,
    last_error: Mutex<Option<String>>,
}

impl MicControls {
    pub fn new(gain: f32, muted: bool) -> Arc<Self> {
        Arc::new(Self {
            gain_bits: AtomicU32::new(gain.max(0.0).to_bits()),
            muted: AtomicBool::new(muted),
            level: LevelTap::default(),
            input: LevelTap::default(),
            last_error: Mutex::new(None),
        })
    }

    /// Linear gain (1.0 = unchanged).
    pub fn set_gain(&self, gain: f32) {
        self.gain_bits
            .store(gain.max(0.0).to_bits(), Ordering::Relaxed);
    }

    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_error(&self, error: Option<String>) {
        *self.last_error.lock().expect("mic error lock") = error;
    }

    /// The last stream error (device unplugged, ...), if any.
    pub fn error(&self) -> Option<String> {
        self.last_error.lock().expect("mic error lock").clone()
    }

    /// Forget meter history and errors before a (re)start.
    pub fn reset(&self) {
        self.level.reset();
        self.input.reset();
        self.set_error(None);
    }
}

/// Multiply by a gain that moves linearly from `from` to `to` across the
/// buffer (a click-free fade for mute/unmute and slider jumps), then clamp.
pub fn apply_gain_ramp(samples: &mut [f32], from: f32, to: f32) {
    if samples.is_empty() {
        return;
    }
    if from == to {
        if to != 1.0 {
            for s in samples.iter_mut() {
                *s = (*s * to).clamp(-1.0, 1.0);
            }
        } else {
            for s in samples.iter_mut() {
                *s = s.clamp(-1.0, 1.0);
            }
        }
        return;
    }
    let n = samples.len() as f32;
    for (i, s) in samples.iter_mut().enumerate() {
        let gain = from + (to - from) * ((i + 1) as f32 / n);
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
}

// ---- the device-independent part -------------------------------------------

/// Everything between the raw device samples and a [`AudioChunk`]: downmix to
/// mono, resample to 48 kHz, apply volume/mute, feed the meters. Separate
/// from `cpal` so it is testable without a microphone.
pub struct MicProcessor {
    in_channels: usize,
    resampler: LinearResampler,
    current_gain: f32,
    controls: Arc<MicControls>,
}

impl MicProcessor {
    pub fn new(in_rate: u32, in_channels: usize, controls: Arc<MicControls>) -> Self {
        let initial = if controls.muted() {
            0.0
        } else {
            controls.gain()
        };
        Self {
            in_channels: in_channels.max(1),
            resampler: LinearResampler::new(in_rate, MIC_SAMPLE_RATE, 1),
            current_gain: initial,
            controls,
        }
    }

    /// Interleaved device samples in, mono 48 kHz samples out.
    pub fn process(&mut self, raw: &[f32]) -> Vec<f32> {
        let mono = to_mono(raw, self.in_channels);
        self.controls.input.record(&mono);
        let mut out = self.resampler.process(&mono);
        let target = if self.controls.muted() {
            0.0
        } else {
            self.controls.gain()
        };
        apply_gain_ramp(&mut out, self.current_gain, target);
        self.current_gain = target;
        self.controls.level.record(&out);
        out
    }
}

// ---- device listing --------------------------------------------------------

/// A microphone the user can pick in Settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// Stable identifier, safe to store and pass back to [`CpalMicSource`].
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn list_input_devices() -> Result<Vec<InputDeviceInfo>, AudioError> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|d| d.id().ok())
        .map(|id| id.to_string());
    let devices = host
        .input_devices()
        .map_err(|e| AudioError::Backend(format!("no se pudo listar los micrófonos: {e}")))?;
    let mut out = Vec::new();
    for device in devices {
        let Ok(id) = device.id().map(|id| id.to_string()) else {
            continue;
        };
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| id.clone());
        out.push(InputDeviceInfo {
            is_default: default_id.as_deref() == Some(id.as_str()),
            id,
            name,
        });
    }
    // Default first, then alphabetical: stable and easy to scan.
    out.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

// ---- the source -------------------------------------------------------------

fn map_error(e: cpal::Error) -> CaptureError {
    match e.kind() {
        ErrorKind::PermissionDenied => CaptureError::MicrophonePermissionDenied,
        ErrorKind::DeviceNotAvailable => {
            CaptureError::Backend("el micrófono elegido no está disponible".into())
        }
        ErrorKind::DeviceBusy => {
            CaptureError::Backend("el micrófono está ocupado por otra aplicación".into())
        }
        _ => CaptureError::Backend(format!("micrófono: {e}")),
    }
}

/// [`AudioSource`] for a microphone. `device_id: None` follows the system
/// default input.
pub struct CpalMicSource {
    device_id: Option<String>,
    controls: Arc<MicControls>,
    epoch: SharedEpoch,
    running: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CpalMicSource {
    /// `epoch` must be the video capture's [`SharedEpoch`]; chunks that
    /// predate it are discarded.
    pub fn new(device_id: Option<String>, controls: Arc<MicControls>, epoch: SharedEpoch) -> Self {
        Self {
            device_id,
            controls,
            epoch,
            running: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicU64::new(0)),
            thread: None,
        }
    }

    /// Chunks the audio callback produced but could not hand over.
    pub fn dropped_chunks(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

fn open_stream(
    device_id: &Option<String>,
    controls: &Arc<MicControls>,
    epoch: &SharedEpoch,
    tx: &mpsc::Sender<AudioChunk>,
    dropped: &Arc<AtomicU64>,
) -> Result<cpal::Stream, CaptureError> {
    let host = cpal::default_host();
    let device = match device_id {
        Some(id) => {
            let parsed = id.parse::<cpal::DeviceId>().map_err(|_| {
                CaptureError::Backend(format!("identificador de micrófono no válido: {id}"))
            })?;
            host.device_by_id(&parsed).ok_or_else(|| {
                CaptureError::Backend("el micrófono elegido no está conectado".into())
            })?
        }
        None => host
            .default_input_device()
            .ok_or_else(|| CaptureError::Backend("no hay ningún micrófono disponible".into()))?,
    };
    let supported = device.default_input_config().map_err(map_error)?;
    let config: StreamConfig = supported.into();
    let args = (config, controls, epoch, tx, dropped);
    match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, args),
        SampleFormat::I16 => build::<i16>(&device, args),
        SampleFormat::U16 => build::<u16>(&device, args),
        SampleFormat::I32 => build::<i32>(&device, args),
        other => Err(CaptureError::Backend(format!(
            "formato de muestra del micrófono no soportado: {other:?}"
        ))),
    }
}

type BuildArgs<'a> = (
    StreamConfig,
    &'a Arc<MicControls>,
    &'a SharedEpoch,
    &'a mpsc::Sender<AudioChunk>,
    &'a Arc<AtomicU64>,
);

fn build<T>(device: &cpal::Device, args: BuildArgs<'_>) -> Result<cpal::Stream, CaptureError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let (config, controls, epoch, tx, dropped) = args;
    let mut processor = MicProcessor::new(
        config.sample_rate,
        usize::from(config.channels),
        Arc::clone(controls),
    );
    let epoch = epoch.clone();
    let tx = tx.clone();
    let dropped = Arc::clone(dropped);
    let error_controls = Arc::clone(controls);

    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                let capture = info.timestamp().capture;
                let capture_ns =
                    i64::try_from(capture.duration_since(StreamInstant::ZERO).as_nanos())
                        .unwrap_or(i64::MAX);
                // Audio from before the first video frame has no place in a clip.
                let Some(timestamp) = epoch.relative(capture_ns) else {
                    return;
                };
                let raw: Vec<f32> = data.iter().map(|s| f32::from_sample(*s)).collect();
                let samples = processor.process(&raw);
                if samples.is_empty() {
                    return;
                }
                let chunk = AudioChunk {
                    samples,
                    channels: MIC_CHANNELS,
                    sample_rate: MIC_SAMPLE_RATE,
                    timestamp,
                };
                if tx.try_send(chunk).is_err() {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            },
            move |e: cpal::Error| {
                eprintln!("clipforge: microphone stream error: {e}");
                error_controls.set_error(Some(match e.kind() {
                    ErrorKind::DeviceNotAvailable => "el micrófono se desconectó".to_string(),
                    _ => e.to_string(),
                }));
            },
            None,
        )
        .map_err(map_error)
}

impl AudioSource for CpalMicSource {
    fn start(&mut self) -> Result<mpsc::Receiver<AudioChunk>, CaptureError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CaptureError::AlreadyStarted);
        }
        self.controls.reset();
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), CaptureError>>();

        let device_id = self.device_id.clone();
        let controls = Arc::clone(&self.controls);
        let epoch = self.epoch.clone();
        let running = Arc::clone(&self.running);
        let dropped = Arc::clone(&self.dropped);

        // The stream is built, kept and dropped on this one thread: on some
        // platforms a `cpal::Stream` must not cross threads.
        self.thread = Some(std::thread::spawn(move || {
            let stream =
                match open_stream(&device_id, &controls, &epoch, &tx, &dropped).and_then(|stream| {
                    stream.play().map_err(map_error)?;
                    Ok(stream)
                }) {
                    Ok(stream) => {
                        let _ = ready_tx.send(Ok(()));
                        stream
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        running.store(false, Ordering::SeqCst);
                        return;
                    }
                };
            while running.load(Ordering::SeqCst) {
                std::thread::park_timeout(Duration::from_millis(100));
            }
            drop(stream);
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
                Err(CaptureError::Backend(
                    "el hilo del micrófono terminó de golpe".into(),
                ))
            }
        }
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

impl Drop for CpalMicSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(rate: u32, freq: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| 0.5 * (TAU * freq * n as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn controls_round_trip_and_clamp_negative_gain() {
        let c = MicControls::new(1.0, false);
        assert_eq!(c.gain(), 1.0);
        c.set_gain(1.5);
        assert_eq!(c.gain(), 1.5);
        c.set_gain(-3.0);
        assert_eq!(c.gain(), 0.0);
        c.set_muted(true);
        assert!(c.muted());
        c.set_error(Some("boom".into()));
        assert_eq!(c.error().as_deref(), Some("boom"));
        c.reset();
        assert_eq!(c.error(), None);
    }

    #[test]
    fn a_constant_gain_scales_and_clamps() {
        let mut s = vec![0.2, -0.4, 0.9];
        apply_gain_ramp(&mut s, 2.0, 2.0);
        assert_eq!(s, [0.4, -0.8, 1.0]);

        let mut unity = vec![0.3, -0.3];
        apply_gain_ramp(&mut unity, 1.0, 1.0);
        assert_eq!(unity, [0.3, -0.3]);
    }

    #[test]
    fn a_ramp_moves_smoothly_and_ends_at_the_target() {
        let mut s = vec![1.0; 10];
        apply_gain_ramp(&mut s, 1.0, 0.0);
        // Strictly decreasing, ending exactly at silence: no click.
        for pair in s.windows(2) {
            assert!(pair[1] < pair[0]);
        }
        assert_eq!(*s.last().unwrap(), 0.0);
        assert!(s[0] > 0.8);
    }

    #[test]
    fn processing_downmixes_resamples_and_applies_gain() {
        let controls = MicControls::new(0.5, false);
        let mut p = MicProcessor::new(44_100, 2, Arc::clone(&controls));
        // Stereo 44.1 kHz, both channels the same 440 Hz sine.
        let mono = sine(44_100, 440.0, 4_410);
        let stereo: Vec<f32> = mono.iter().flat_map(|s| [*s, *s]).collect();
        let out = p.process(&stereo);

        // ~100 ms at 48 kHz, half the original amplitude.
        assert!((4_795..=4_801).contains(&out.len()), "{} frames", out.len());
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((0.24..0.26).contains(&peak), "peak {peak}");
        // The meter saw the post-gain level, the raw tap the pre-gain one.
        assert!((0.24..0.26).contains(&controls.level.take_peak()));
        assert!((0.49..0.51).contains(&controls.input.take_peak()));
    }

    #[test]
    fn muting_fades_to_silence_but_the_raw_signal_is_still_seen() {
        let controls = MicControls::new(1.0, false);
        let mut p = MicProcessor::new(48_000, 1, Arc::clone(&controls));
        p.process(&sine(48_000, 300.0, 480)); // establish unity gain
        controls.set_muted(true);
        let faded = p.process(&sine(48_000, 300.0, 480));
        assert_eq!(*faded.last().unwrap(), 0.0, "ends silent");
        let silent = p.process(&sine(48_000, 300.0, 480));
        assert!(silent.iter().all(|s| *s == 0.0));

        controls.level.take_peak();
        p.process(&sine(48_000, 300.0, 480));
        assert_eq!(
            controls.level.take_peak(),
            0.0,
            "the meter shows muted as silence"
        );
        assert!(
            controls.input.heard_signal(),
            "but the device is clearly alive"
        );
    }

    #[test]
    fn a_dead_input_is_distinguishable_from_a_muted_one() {
        let controls = MicControls::new(1.0, false);
        let mut p = MicProcessor::new(48_000, 1, Arc::clone(&controls));
        p.process(&vec![0.0; 4_800]); // permission denied delivers exact zeros
        assert!(!controls.input.heard_signal());
    }

    #[test]
    fn a_starting_mute_does_not_fade_in_from_nothing() {
        // Created muted: the first buffer must be silent from sample zero.
        let controls = MicControls::new(1.0, true);
        let mut p = MicProcessor::new(48_000, 1, controls);
        let out = p.process(&sine(48_000, 300.0, 480));
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn the_mic_encoder_format_is_mono_48k() {
        let c = mic_audio_config();
        assert_eq!((c.sample_rate, c.channels), (48_000, 1));
    }
}
