//! Windows AAC-LC encoding with the Media Foundation AAC encoder MFT.
//!
//! [`MediaFoundationAacEncoder`] implements [`AudioEncoder`]: interleaved
//! `f32` PCM is converted to 16-bit (the only input the Microsoft encoder
//! accepts), fed to the transform, and raw AAC access units (payload type 0,
//! no ADTS header — what an mp4 `mp4a` track stores) come out, one per 1024
//! samples. Packet timestamps come from sample counting, not from the MFT.
//!
//! **Status:** compiled and cross-checked from macOS only. Two things need
//! calibrating on a Windows machine: that `ProcessOutput` yields exactly one
//! AAC frame per call as assumed, and [`MF_AAC_PRIMING_FRAMES`] (the
//! encoder's delay, used for the clip's edit list).

use std::mem::ManuallyDrop;
use std::time::Duration;

use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::{
    AACMFTEncoder, IMFMediaType, IMFSample, IMFTransform, MFAudioFormat_AAC, MFAudioFormat_PCM,
    MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Audio, MFShutdown,
    MFStartup, MFSTARTUP_FULL, MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_COMMAND_FLUSH,
    MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION,
    MF_MT_AAC_PAYLOAD_TYPE, MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
    MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_VERSION,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

use super::{
    frames_to_duration, AudioChunk, AudioConfig, AudioEncoder, AudioError, AudioPacket,
    AAC_FRAME_SAMPLES,
};

/// `MF_MT_AAC_PAYLOAD_TYPE` 0: raw AAC access units (no ADTS/LATM framing).
const MF_AAC_PAYLOAD_RAW: u32 = 0;
/// Encoder delay assumed for the Microsoft AAC-LC encoder, in samples. Not
/// documented by Microsoft; one AAC frame is the lower bound. If a clip's
/// audio sounds consistently early/late against its video on Windows,
/// calibrate this value.
pub const MF_AAC_PRIMING_FRAMES: u32 = 1024;

/// `HNSEC` per second: Media Foundation times are in 100 ns units.
const HNS_PER_SECOND: i64 = 10_000_000;

fn backend(context: &str, e: windows::core::Error) -> AudioError {
    AudioError::Backend(format!("{context}: {e}"))
}

pub struct MediaFoundationAacEncoder {
    transform: IMFTransform,
    config: AudioConfig,
    /// Timestamp of the first sample fed since the last (re)start.
    base: Option<Duration>,
    /// Frames submitted since the last (re)start (drives input timestamps).
    fed_frames: u64,
    /// AAC packets produced since the last (re)start.
    packets_out: u64,
    output_buffer_size: u32,
    finished: bool,
}

// SAFETY: the transform is only used through `&mut self` (one thread at a
// time) and the AAC encoder MFT is free-threaded.
unsafe impl Send for MediaFoundationAacEncoder {}

fn set_u32(media: &IMFMediaType, key: &GUID, value: u32) -> Result<(), AudioError> {
    // SAFETY: `media` is a live COM object.
    unsafe { media.SetUINT32(key, value) }.map_err(|e| backend("set media type attribute", e))
}

fn audio_type(
    subtype: &GUID,
    config: &AudioConfig,
    avg_bytes_per_second: u32,
    block_align: u32,
) -> Result<IMFMediaType, AudioError> {
    // SAFETY: plain Media Foundation calls on live objects.
    unsafe {
        let media = MFCreateMediaType().map_err(|e| backend("create media type", e))?;
        media
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .map_err(|e| backend("major type", e))?;
        media
            .SetGUID(&MF_MT_SUBTYPE, subtype)
            .map_err(|e| backend("subtype", e))?;
        set_u32(&media, &MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        set_u32(&media, &MF_MT_AUDIO_SAMPLES_PER_SECOND, config.sample_rate)?;
        set_u32(
            &media,
            &MF_MT_AUDIO_NUM_CHANNELS,
            u32::from(config.channels),
        )?;
        set_u32(
            &media,
            &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
            avg_bytes_per_second,
        )?;
        set_u32(&media, &MF_MT_AUDIO_BLOCK_ALIGNMENT, block_align)?;
        Ok(media)
    }
}

impl MediaFoundationAacEncoder {
    pub fn new(config: AudioConfig) -> Result<Self, AudioError> {
        if config.channels == 0 || config.channels > 2 {
            return Err(AudioError::Unsupported(format!(
                "{} channels (AAC-LC here is mono or stereo)",
                config.channels
            )));
        }
        // The encoder only offers these average rates.
        let bytes_per_second = config.bitrate / 8;
        if ![12_000, 16_000, 20_000, 24_000].contains(&bytes_per_second) {
            return Err(AudioError::Unsupported(format!(
                "{} b/s: the Media Foundation AAC encoder supports 96, 128, 160 or 192 kb/s",
                config.bitrate
            )));
        }

        // SAFETY: COM/MF initialization; failures are reported, and the
        // matching MFShutdown is in `Drop` (only after a successful startup).
        unsafe {
            // Ignore "already initialized with another mode".
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| backend("MFStartup", e))?;
        }
        let built = Self::build(config);
        if built.is_err() {
            // SAFETY: balances the successful MFStartup above.
            unsafe {
                let _ = MFShutdown();
            }
        }
        built
    }

    fn build(config: AudioConfig) -> Result<Self, AudioError> {
        // SAFETY: plain Media Foundation calls on live objects.
        unsafe {
            let transform: IMFTransform =
                CoCreateInstance(&AACMFTEncoder, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| AudioError::Unsupported(format!("no AAC encoder MFT: {e}")))?;

            // Output first: the AAC encoder derives its input needs from it.
            let output = audio_type(&MFAudioFormat_AAC, &config, config.bitrate / 8, 1)?;
            set_u32(&output, &MF_MT_AAC_PAYLOAD_TYPE, MF_AAC_PAYLOAD_RAW)?;
            set_u32(&output, &MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0x29)?; // AAC-LC L2
            transform
                .SetOutputType(0, &output, 0)
                .map_err(|e| backend("set output type", e))?;

            let block_align = u32::from(config.channels) * 2;
            let input = audio_type(
                &MFAudioFormat_PCM,
                &config,
                config.sample_rate * block_align,
                block_align,
            )?;
            transform
                .SetInputType(0, &input, 0)
                .map_err(|e| backend("set input type", e))?;

            let info = transform
                .GetOutputStreamInfo(0)
                .map_err(|e| backend("output stream info", e))?;

            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(|e| backend("begin streaming", e))?;
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(|e| backend("start of stream", e))?;

            Ok(Self {
                transform,
                config,
                base: None,
                fed_frames: 0,
                packets_out: 0,
                output_buffer_size: info.cbSize.max(1024),
                finished: false,
            })
        }
    }

    fn sample_from_pcm(&self, samples: &[f32], time_hns: i64) -> Result<IMFSample, AudioError> {
        let pcm: Vec<u8> = samples
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
            .collect();
        let frames = samples.len() / usize::from(self.config.channels);
        // SAFETY: the buffer is locked only while copying `pcm` into it.
        unsafe {
            let buffer = MFCreateMemoryBuffer(pcm.len() as u32)
                .map_err(|e| backend("create input buffer", e))?;
            let mut data: *mut u8 = std::ptr::null_mut();
            buffer
                .Lock(&mut data, None, None)
                .map_err(|e| backend("lock input buffer", e))?;
            std::ptr::copy_nonoverlapping(pcm.as_ptr(), data, pcm.len());
            buffer
                .Unlock()
                .map_err(|e| backend("unlock input buffer", e))?;
            buffer
                .SetCurrentLength(pcm.len() as u32)
                .map_err(|e| backend("input length", e))?;

            let sample = MFCreateSample().map_err(|e| backend("create sample", e))?;
            sample
                .AddBuffer(&buffer)
                .map_err(|e| backend("add buffer", e))?;
            sample
                .SetSampleTime(time_hns)
                .map_err(|e| backend("sample time", e))?;
            sample
                .SetSampleDuration(
                    frames as i64 * HNS_PER_SECOND / i64::from(self.config.sample_rate),
                )
                .map_err(|e| backend("sample duration", e))?;
            Ok(sample)
        }
    }

    /// Pull every packet the transform has ready.
    fn collect_output(&mut self) -> Result<Vec<AudioPacket>, AudioError> {
        let mut packets = Vec::new();
        loop {
            // SAFETY: the output sample/buffer are created here and handed to
            // the MFT for exactly one `ProcessOutput` call.
            let data = unsafe {
                let buffer = MFCreateMemoryBuffer(self.output_buffer_size)
                    .map_err(|e| backend("create output buffer", e))?;
                let sample = MFCreateSample().map_err(|e| backend("create output sample", e))?;
                sample
                    .AddBuffer(&buffer)
                    .map_err(|e| backend("add output buffer", e))?;

                let mut out = [MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: ManuallyDrop::new(Some(sample)),
                    dwStatus: 0,
                    pEvents: ManuallyDrop::new(None),
                }];
                let mut status = 0u32;
                let result = self.transform.ProcessOutput(0, &mut out, &mut status);

                // Take ownership back so the COM references are released.
                let sample = ManuallyDrop::into_inner(std::mem::replace(
                    &mut out[0].pSample,
                    ManuallyDrop::new(None),
                ));
                drop(ManuallyDrop::into_inner(std::mem::replace(
                    &mut out[0].pEvents,
                    ManuallyDrop::new(None),
                )));

                match result {
                    Ok(()) => {}
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(packets),
                    Err(e) => return Err(backend("ProcessOutput", e)),
                }
                let Some(sample) = sample else {
                    return Ok(packets);
                };
                let contiguous = sample
                    .ConvertToContiguousBuffer()
                    .map_err(|e| backend("output buffer", e))?;
                let mut ptr: *mut u8 = std::ptr::null_mut();
                let mut len = 0u32;
                contiguous
                    .Lock(&mut ptr, None, Some(&mut len))
                    .map_err(|e| backend("lock output", e))?;
                let bytes = std::slice::from_raw_parts(ptr, len as usize).to_vec();
                contiguous
                    .Unlock()
                    .map_err(|e| backend("unlock output", e))?;
                bytes
            };
            if data.is_empty() {
                continue;
            }
            let k = self.packets_out;
            self.packets_out += 1;
            packets.push(AudioPacket {
                data,
                pts: self.base.unwrap_or_default()
                    + frames_to_duration(k * u64::from(AAC_FRAME_SAMPLES), self.config.sample_rate),
                duration: frames_to_duration(u64::from(AAC_FRAME_SAMPLES), self.config.sample_rate),
            });
        }
    }
}

impl AudioEncoder for MediaFoundationAacEncoder {
    fn encode(&mut self, chunk: &AudioChunk) -> Result<Vec<AudioPacket>, AudioError> {
        if self.finished {
            return Err(AudioError::Backend("encode after finish".into()));
        }
        if chunk.sample_rate != self.config.sample_rate || chunk.channels != self.config.channels {
            return Err(AudioError::BadChunk(format!(
                "{} Hz x{} chunk for {} Hz x{} encoder",
                chunk.sample_rate, chunk.channels, self.config.sample_rate, self.config.channels
            )));
        }
        if self.base.is_none() {
            self.base = Some(chunk.timestamp);
        }
        let time_hns = self.fed_frames as i64 * HNS_PER_SECOND / i64::from(self.config.sample_rate);
        let sample = self.sample_from_pcm(&chunk.samples, time_hns)?;
        // SAFETY: `sample` is a fully built, live sample.
        unsafe { self.transform.ProcessInput(0, &sample, 0) }
            .map_err(|e| backend("ProcessInput", e))?;
        self.fed_frames += chunk.frames() as u64;
        self.collect_output()
    }

    fn finish(&mut self) -> Result<Vec<AudioPacket>, AudioError> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        // SAFETY: plain message to a live transform.
        unsafe { self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0) }
            .map_err(|e| backend("drain", e))?;
        self.collect_output()
    }

    fn resync(&mut self) {
        // SAFETY: plain message to a live transform.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);
        }
        self.base = None;
        self.fed_frames = 0;
        self.packets_out = 0;
    }

    fn priming_frames(&self) -> u32 {
        MF_AAC_PRIMING_FRAMES
    }

    fn config(&self) -> &AudioConfig {
        &self.config
    }
}

impl Drop for MediaFoundationAacEncoder {
    fn drop(&mut self) {
        // SAFETY: balances the MFStartup in `new`. The transform is released
        // by its own `Drop` after this.
        unsafe {
            let _ = MFShutdown();
        }
    }
}
