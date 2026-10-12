//! Windows H.264 encoding with the Media Foundation encoder MFT.
//!
//! [`MediaFoundationEncoder`] implements [`Encoder`]: each BGRA frame is turned
//! into NV12 (see [`super::nv12`]), handed to the transform, and the Annex B
//! access units that come out are converted to the length-prefixed packets the
//! ring buffer and the mp4 muxer use ([`super::annexb`]).
//!
//! It drives Microsoft's own H.264 encoder (`CMSH264EncoderMFT`): a
//! synchronous transform that every Windows 10/11 has and that needs no GPU.
//! Hardware encoders (NVENC, AMF, Quick Sync) are asynchronous MFTs with their
//! own quirks; they are not used yet, so on Windows this is software encoding —
//! tuned for speed and low latency, no B-frames, so packets come out in the
//! order they went in.

use std::mem::ManuallyDrop;
use std::time::Duration;

use windows::core::{Interface, GUID, HRESULT};
use windows::Win32::Media::MediaFoundation::{
    eAVEncCommonRateControlMode_CBR, eAVEncH264VProfile_High, CMSH264EncoderMFT,
    CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonQualityVsSpeed,
    CODECAPI_AVEncCommonRateControlMode, CODECAPI_AVEncMPVDefaultBPictureCount,
    CODECAPI_AVEncMPVGOPSize, CODECAPI_AVLowLatencyMode, ICodecAPI, IMFMediaType, IMFSample,
    IMFTransform, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
    MFShutdown, MFStartup, MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
    MFVideoPrimaries_BT709, MFVideoTransFunc_709, MFVideoTransferMatrix_BT709, MFSTARTUP_FULL,
    MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
    MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_MT_TRANSFER_FUNCTION, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES, MF_MT_YUV_MATRIX,
    MF_VERSION,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Variant::VARIANT;

use super::annexb::{parse_access_unit, AccessUnit};
use super::nv12::{bgra_to_nv12, nv12_len};
use super::{Codec, EncodeError, EncodedPacket, Encoder, EncoderConfig, ParameterSets};
use crate::capture::RawFrame;

/// Media Foundation times are in 100 ns units.
const HNS_PER_SECOND: u64 = 10_000_000;
/// `MF_E_NOTACCEPTING`: the transform wants its output taken before more input.
const MF_E_NOTACCEPTING: HRESULT = HRESULT(0xC00D36B5_u32 as _);
/// `MFNominalRange_16_235` — "TV" range, what [`super::nv12`] produces.
const NOMINAL_RANGE_16_235: u32 = 2;
/// Encoder effort, 0 (fastest) to 100 (best quality). A screen recorder that
/// must keep up with the game wants speed.
const QUALITY_VS_SPEED: u32 = 20;

fn backend(context: &str, e: windows::core::Error) -> EncodeError {
    EncodeError::Backend(format!("{context}: {e}"))
}

fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

fn set_u32(media: &IMFMediaType, key: &GUID, value: u32) -> Result<(), EncodeError> {
    // SAFETY: `media` is a live COM object.
    unsafe { media.SetUINT32(key, value) }.map_err(|e| backend("set media type attribute", e))
}

fn set_u64(media: &IMFMediaType, key: &GUID, value: u64) -> Result<(), EncodeError> {
    // SAFETY: `media` is a live COM object.
    unsafe { media.SetUINT64(key, value) }.map_err(|e| backend("set media type attribute", e))
}

/// The media type common to both ends: video of this size and rate.
fn video_type(subtype: &GUID, config: &EncoderConfig) -> Result<IMFMediaType, EncodeError> {
    // SAFETY: plain Media Foundation calls on live objects.
    let media = unsafe { MFCreateMediaType() }.map_err(|e| backend("create media type", e))?;
    // SAFETY: as above.
    unsafe {
        media
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| backend("major type", e))?;
        media
            .SetGUID(&MF_MT_SUBTYPE, subtype)
            .map_err(|e| backend("subtype", e))?;
    }
    set_u64(&media, &MF_MT_FRAME_SIZE, pack(config.width, config.height))?;
    set_u64(&media, &MF_MT_FRAME_RATE, pack(config.fps, 1))?;
    set_u64(&media, &MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
    set_u32(
        &media,
        &MF_MT_INTERLACE_MODE,
        MFVideoInterlace_Progressive.0 as u32,
    )?;
    Ok(media)
}

pub struct MediaFoundationEncoder {
    /// Released by hand in `Drop`, *before* Media Foundation is shut down: the
    /// encoder runs on the platform's work-queue threads, which `MFShutdown`
    /// tears down.
    transform: ManuallyDrop<IMFTransform>,
    config: EncoderConfig,
    parameter_sets: Option<ParameterSets>,
    /// Reused between frames.
    nv12: Vec<u8>,
    output_buffer_size: u32,
    /// The transform allocates its own output samples.
    provides_samples: bool,
    frames_in: u64,
    packets_out: u64,
    finished: bool,
}

// SAFETY: the transform is only used through `&mut self` (one thread at a
// time) and Microsoft's H.264 encoder MFT is free-threaded.
unsafe impl Send for MediaFoundationEncoder {}

impl MediaFoundationEncoder {
    pub fn new(config: EncoderConfig) -> Result<Self, EncodeError> {
        if config.codec != Codec::H264 {
            return Err(EncodeError::HardwareUnavailable(config.codec));
        }
        if config.width < 2
            || config.height < 2
            || !config.width.is_multiple_of(2)
            || !config.height.is_multiple_of(2)
        {
            return Err(EncodeError::BadFrame(format!(
                "{}x{}: the encoder needs even, non-zero sides",
                config.width, config.height
            )));
        }
        if config.fps == 0 {
            return Err(EncodeError::Backend("fps must be non-zero".into()));
        }

        // SAFETY: COM/MF initialization; failures are reported, and the matching
        // MFShutdown is in `Drop` (only after a successful startup).
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

    fn build(config: EncoderConfig) -> Result<Self, EncodeError> {
        // SAFETY: plain Media Foundation calls on live objects.
        unsafe {
            let transform: IMFTransform =
                CoCreateInstance(&CMSH264EncoderMFT, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| EncodeError::Backend(format!("no H.264 encoder MFT: {e}")))?;

            // Output first: the encoder derives what it accepts as input from it.
            let output = video_type(&MFVideoFormat_H264, &config)?;
            set_u32(&output, &MF_MT_AVG_BITRATE, config.bitrate)?;
            set_u32(
                &output,
                &MF_MT_MPEG2_PROFILE,
                eAVEncH264VProfile_High.0 as u32,
            )?;
            transform
                .SetOutputType(0, &output, 0)
                .map_err(|e| backend("set output type", e))?;

            // Tuning is best effort: an encoder that refuses one knob still works.
            if let Ok(api) = transform.cast::<ICodecAPI>() {
                let knobs: [(&GUID, VARIANT); 6] = [
                    (
                        &CODECAPI_AVEncCommonRateControlMode,
                        VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32),
                    ),
                    (
                        &CODECAPI_AVEncCommonMeanBitRate,
                        VARIANT::from(config.bitrate),
                    ),
                    (
                        &CODECAPI_AVEncMPVGOPSize,
                        VARIANT::from(config.keyframe_interval),
                    ),
                    (&CODECAPI_AVLowLatencyMode, VARIANT::from(true)),
                    (&CODECAPI_AVEncMPVDefaultBPictureCount, VARIANT::from(0u32)),
                    (
                        &CODECAPI_AVEncCommonQualityVsSpeed,
                        VARIANT::from(QUALITY_VS_SPEED),
                    ),
                ];
                for (key, value) in &knobs {
                    if let Err(e) = api.SetValue(*key, value) {
                        eprintln!("clipforge: H.264 encoder setting refused ({key:?}): {e}");
                    }
                }
            }

            let input = video_type(&MFVideoFormat_NV12, &config)?;
            set_u32(&input, &MF_MT_DEFAULT_STRIDE, config.width)?;
            // What `nv12::bgra_to_nv12` produces, so players read the colors right.
            set_u32(
                &input,
                &MF_MT_YUV_MATRIX,
                MFVideoTransferMatrix_BT709.0 as u32,
            )?;
            set_u32(
                &input,
                &MF_MT_VIDEO_PRIMARIES,
                MFVideoPrimaries_BT709.0 as u32,
            )?;
            set_u32(
                &input,
                &MF_MT_TRANSFER_FUNCTION,
                MFVideoTransFunc_709.0 as u32,
            )?;
            set_u32(&input, &MF_MT_VIDEO_NOMINAL_RANGE, NOMINAL_RANGE_16_235)?;
            transform
                .SetInputType(0, &input, 0)
                .map_err(|e| backend("set input type", e))?;

            let info = transform
                .GetOutputStreamInfo(0)
                .map_err(|e| backend("output stream info", e))?;
            let provides_samples = info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0)
                    as u32
                != 0;

            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(|e| backend("begin streaming", e))?;
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(|e| backend("start of stream", e))?;

            Ok(Self {
                transform: ManuallyDrop::new(transform),
                nv12: vec![0; nv12_len(config.width as usize, config.height as usize)],
                config,
                parameter_sets: None,
                output_buffer_size: info.cbSize.max(1 << 20),
                provides_samples,
                frames_in: 0,
                packets_out: 0,
                finished: false,
            })
        }
    }

    fn input_sample(&self, pts: Duration) -> Result<IMFSample, EncodeError> {
        // SAFETY: the buffer is locked only while copying `self.nv12` into it.
        unsafe {
            let buffer = MFCreateMemoryBuffer(self.nv12.len() as u32)
                .map_err(|e| backend("create input buffer", e))?;
            let mut data: *mut u8 = std::ptr::null_mut();
            buffer
                .Lock(&mut data, None, None)
                .map_err(|e| backend("lock input buffer", e))?;
            std::ptr::copy_nonoverlapping(self.nv12.as_ptr(), data, self.nv12.len());
            buffer
                .Unlock()
                .map_err(|e| backend("unlock input buffer", e))?;
            buffer
                .SetCurrentLength(self.nv12.len() as u32)
                .map_err(|e| backend("input length", e))?;

            let sample = MFCreateSample().map_err(|e| backend("create sample", e))?;
            sample
                .AddBuffer(&buffer)
                .map_err(|e| backend("add buffer", e))?;
            sample
                .SetSampleTime((pts.as_nanos() / 100) as i64)
                .map_err(|e| backend("sample time", e))?;
            sample
                .SetSampleDuration((HNS_PER_SECOND / u64::from(self.config.fps)) as i64)
                .map_err(|e| backend("sample duration", e))?;
            Ok(sample)
        }
    }

    /// Take every access unit the transform has ready.
    fn collect_output(&mut self) -> Result<Vec<EncodedPacket>, EncodeError> {
        let mut packets = Vec::new();
        loop {
            // SAFETY: the output sample is created here (or by the MFT) and lives
            // for exactly one `ProcessOutput` call.
            let (bytes, pts_hns) = unsafe {
                let supplied = if self.provides_samples {
                    None
                } else {
                    let buffer = MFCreateMemoryBuffer(self.output_buffer_size)
                        .map_err(|e| backend("create output buffer", e))?;
                    let sample =
                        MFCreateSample().map_err(|e| backend("create output sample", e))?;
                    sample
                        .AddBuffer(&buffer)
                        .map_err(|e| backend("add output buffer", e))?;
                    Some(sample)
                };
                let mut out = [MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: ManuallyDrop::new(supplied),
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
                    Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                        // The output type changed (a new sequence header): take
                        // the one the encoder now offers and ask again.
                        let media = self
                            .transform
                            .GetOutputAvailableType(0, 0)
                            .map_err(|e| backend("new output type", e))?;
                        self.transform
                            .SetOutputType(0, &media, 0)
                            .map_err(|e| backend("accept new output type", e))?;
                        continue;
                    }
                    Err(e) => return Err(backend("ProcessOutput", e)),
                }
                let Some(sample) = sample else {
                    return Ok(packets);
                };
                let pts_hns = sample.GetSampleTime().unwrap_or(0).max(0) as u64;
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
                (bytes, pts_hns)
            };
            if bytes.is_empty() {
                continue;
            }
            let unit: AccessUnit = parse_access_unit(&bytes)
                .map_err(|e| EncodeError::Backend(format!("unreadable encoder output: {e}")))?;
            if self.parameter_sets.is_none() && !unit.sps.is_empty() && !unit.pps.is_empty() {
                self.parameter_sets = Some(ParameterSets {
                    sps: unit.sps.clone(),
                    pps: unit.pps.clone(),
                });
            }
            if unit.data.is_empty() {
                continue; // headers only: nothing to play
            }
            let pts = Duration::from_nanos(pts_hns * 100);
            self.packets_out += 1;
            packets.push(EncodedPacket {
                data: unit.data,
                pts,
                dts: pts, // no B-frames
                is_keyframe: unit.is_keyframe,
            });
        }
    }
}

impl Encoder for MediaFoundationEncoder {
    fn encode(&mut self, frame: &RawFrame) -> Result<Vec<EncodedPacket>, EncodeError> {
        if self.finished {
            return Err(EncodeError::Backend("encode after finish".into()));
        }
        if frame.width != self.config.width || frame.height != self.config.height {
            return Err(EncodeError::BadFrame(format!(
                "{}x{} frame for {}x{} encoder",
                frame.width, frame.height, self.config.width, self.config.height
            )));
        }
        if frame.data.len() != frame.expected_len() {
            return Err(EncodeError::BadFrame(format!(
                "{} bytes for a {}x{} BGRA frame",
                frame.data.len(),
                frame.width,
                frame.height
            )));
        }
        bgra_to_nv12(
            &frame.data,
            frame.width as usize,
            frame.height as usize,
            &mut self.nv12,
        );
        let sample = self.input_sample(frame.timestamp)?;
        // SAFETY: `sample` is a fully built, live sample.
        let mut submitted = unsafe { self.transform.ProcessInput(0, &sample, 0) };
        let mut packets = Vec::new();
        if matches!(&submitted, Err(e) if e.code() == MF_E_NOTACCEPTING) {
            // Its output queue is full: take what is ready and offer the frame again.
            packets = self.collect_output()?;
            // SAFETY: as above.
            submitted = unsafe { self.transform.ProcessInput(0, &sample, 0) };
        }
        submitted.map_err(|e| backend("ProcessInput", e))?;
        self.frames_in += 1;
        packets.extend(self.collect_output()?);
        Ok(packets)
    }

    fn finish(&mut self) -> Result<Vec<EncodedPacket>, EncodeError> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        // SAFETY: plain messages to a live transform.
        unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)
                .map_err(|e| backend("end of stream", e))?;
            self.transform
                .ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)
                .map_err(|e| backend("drain", e))?;
        }
        self.collect_output()
    }

    fn parameter_sets(&self) -> Option<ParameterSets> {
        self.parameter_sets.clone()
    }

    fn config(&self) -> &EncoderConfig {
        &self.config
    }
}

impl Drop for MediaFoundationEncoder {
    fn drop(&mut self) {
        // SAFETY: the transform is dropped exactly once, here, and never used
        // again; the MFShutdown balances the MFStartup in `new`.
        unsafe {
            ManuallyDrop::drop(&mut self.transform);
            let _ = MFShutdown();
        }
    }
}
