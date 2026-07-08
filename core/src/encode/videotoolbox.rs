//! macOS hardware H.264 encoding via VideoToolbox.
//!
//! [`VideoToolboxEncoder`] implements [`Encoder`] on top of a
//! `VTCompressionSession` (through the `videotoolbox` crate): BGRA frames are
//! copied into a reused `IOSurface`, encoded synchronously (real-time mode,
//! no B-frames, so pts == dts), and emitted as AVCC (4-byte length-prefixed)
//! packets. SPS/PPS are pulled from the first packet's format description.

use std::time::Duration;

use apple_cf::iosurface::IOSurface;
use videotoolbox::available_video_encoder_details;
use videotoolbox::compression::{CompressionSession, EncodedFrame};
use videotoolbox::error::VTError;
use videotoolbox::session::Codec as VTCodec;

use super::{Codec, EncodeError, EncodedPacket, Encoder, EncoderConfig, ParameterSets};
use crate::capture::RawFrame;

/// pts timescale handed to VideoToolbox: nanoseconds.
const TIMESCALE: i32 = 1_000_000_000;

pub struct VideoToolboxEncoder {
    session: CompressionSession,
    surface: IOSurface,
    config: EncoderConfig,
    parameter_sets: Option<ParameterSets>,
    finished: bool,
}

impl VideoToolboxEncoder {
    pub fn new(config: EncoderConfig) -> Result<Self, EncodeError> {
        let vt_codec = match config.codec {
            Codec::H264 => VTCodec::H264,
            Codec::Hevc => VTCodec::HEVC,
        };

        // Hardware-only policy: refuse to construct if the system has no
        // hardware encoder for this codec (VT would silently fall back to
        // software on such systems).
        let has_hardware = available_video_encoder_details()
            .map_err(|e| EncodeError::Backend(format!("encoder list: {e}")))?
            .iter()
            .any(|details| {
                details.base.codec_type == vt_codec.as_cm_codec_type()
                    && details.is_hardware_accelerated.unwrap_or(false)
            });
        if !has_hardware {
            return Err(EncodeError::HardwareUnavailable(config.codec));
        }

        let session =
            CompressionSession::builder(config.width as i32, config.height as i32, vt_codec)
                .with_real_time(true)
                .with_allow_frame_reordering(false)
                .with_average_bit_rate(config.bitrate as i32)
                .with_expected_frame_rate(f64::from(config.fps))
                .with_max_keyframe_interval(config.keyframe_interval as i32)
                .build()
                .map_err(map_vt_error)?;

        let surface = IOSurface::create(
            config.width as usize,
            config.height as usize,
            u32::from_be_bytes(*b"BGRA"),
            4,
        )
        .ok_or_else(|| EncodeError::Backend("IOSurface allocation failed".into()))?;

        Ok(Self {
            session,
            surface,
            config,
            parameter_sets: None,
            finished: false,
        })
    }

    /// Copy a tightly-packed BGRA frame into the (possibly row-padded)
    /// input surface.
    fn upload(&mut self, frame: &RawFrame) -> Result<(), EncodeError> {
        let mut guard = self
            .surface
            .lock_read_write()
            .map_err(|status| EncodeError::Backend(format!("IOSurfaceLock: {status}")))?;
        let stride = guard.bytes_per_row();
        let row_len = frame.width as usize * 4;
        let height = frame.height as usize;
        let base = guard
            .base_address_mut()
            .ok_or_else(|| EncodeError::Backend("IOSurface base address unavailable".into()))?;
        debug_assert!(stride >= row_len);
        for row in 0..height {
            // SAFETY: the surface was allocated as width x height BGRA with
            // `stride` bytes per row; both slices stay in bounds and the
            // pointer is valid while the lock guard is held.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    frame.data.as_ptr().add(row * row_len),
                    base.add(row * stride),
                    row_len,
                );
            }
        }
        Ok(())
    }

    fn packet_from(&mut self, encoded: &EncodedFrame) -> Result<EncodedPacket, EncodeError> {
        if self.parameter_sets.is_none() {
            self.parameter_sets = extract_parameter_sets(encoded);
        }
        let (value, timescale) = encoded.presentation_time;
        let pts = if timescale > 0 {
            Duration::from_nanos((value as i128 * 1_000_000_000 / timescale as i128) as u64)
        } else {
            Duration::ZERO
        };
        Ok(EncodedPacket {
            is_keyframe: avcc_contains_idr(&encoded.data),
            data: encoded.data.clone(),
            pts,
            dts: pts, // no B-frames (frame reordering disabled)
        })
    }
}

impl Encoder for VideoToolboxEncoder {
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
                "frame data is {} bytes, expected {}",
                frame.data.len(),
                frame.expected_len()
            )));
        }

        self.upload(frame)?;
        let pts_value = frame.timestamp.as_nanos() as i64;
        let encoded = self
            .session
            .encode(&self.surface, (pts_value, TIMESCALE))
            .map_err(map_vt_error)?;
        if encoded.data.is_empty() {
            // Encoder dropped the frame (allowed in real-time mode).
            return Ok(vec![]);
        }
        Ok(vec![self.packet_from(&encoded)?])
    }

    fn finish(&mut self) -> Result<Vec<EncodedPacket>, EncodeError> {
        // `encode` is synchronous (one packet out per frame in), so there is
        // nothing buffered to drain.
        self.finished = true;
        Ok(vec![])
    }

    fn parameter_sets(&self) -> Option<ParameterSets> {
        self.parameter_sets.clone()
    }

    fn config(&self) -> &EncoderConfig {
        &self.config
    }
}

fn map_vt_error(err: VTError) -> EncodeError {
    EncodeError::Backend(err.to_string())
}

/// Walk AVCC (4-byte length-prefixed) NAL units; true if any is an H.264 IDR.
fn avcc_contains_idr(data: &[u8]) -> bool {
    avcc_nal_types(data).any(|t| t == 5)
}

fn avcc_nal_types(data: &[u8]) -> impl Iterator<Item = u8> + '_ {
    let mut offset = 0usize;
    std::iter::from_fn(move || {
        if offset + 4 > data.len() {
            return None;
        }
        let len = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        let nal_start = offset + 4;
        if len == 0 || nal_start + len > data.len() {
            return None;
        }
        offset = nal_start + len;
        Some(data[nal_start] & 0x1f)
    })
}

/// Read H.264 SPS/PPS from the sample buffer's format description.
///
/// The `videotoolbox` crate doesn't expose this, so we call CoreMedia
/// directly (already linked by our other Apple-framework dependencies).
fn extract_parameter_sets(encoded: &EncodedFrame) -> Option<ParameterSets> {
    use std::ffi::c_void;
    use std::ptr;

    extern "C" {
        fn CMSampleBufferGetFormatDescription(sbuf: *const c_void) -> *const c_void;
        fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            desc: *const c_void,
            index: usize,
            out_ptr: *mut *const u8,
            out_size: *mut usize,
            out_count: *mut usize,
            out_nal_header_len: *mut i32,
        ) -> i32;
    }

    let sbuf = encoded.cm_sample_buffer()?.as_ptr();
    // SAFETY: `sbuf` is a valid CMSampleBuffer owned by `encoded`; CoreMedia
    // returns a borrowed format description whose lifetime is tied to it. The
    // parameter-set pointers are copied to owned Vecs before returning.
    unsafe {
        let desc = CMSampleBufferGetFormatDescription(sbuf.cast());
        if desc.is_null() {
            return None;
        }
        let mut count = 0usize;
        let mut nal_header_len = 0i32;
        let status = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            desc,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut count,
            &mut nal_header_len,
        );
        if status != 0 || count < 2 || nal_header_len != 4 {
            return None;
        }
        let mut sets = ParameterSets::default();
        for index in 0..count {
            let mut data: *const u8 = ptr::null();
            let mut size = 0usize;
            if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                desc,
                index,
                &mut data,
                &mut size,
                ptr::null_mut(),
                ptr::null_mut(),
            ) != 0
                || data.is_null()
            {
                return None;
            }
            let bytes = std::slice::from_raw_parts(data, size).to_vec();
            match bytes.first().map(|b| b & 0x1f) {
                Some(7) => sets.sps.push(bytes),
                Some(8) => sets.pps.push(bytes),
                _ => {}
            }
        }
        (!sets.sps.is_empty() && !sets.pps.is_empty()).then_some(sets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avcc(nals: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            out.extend_from_slice(nal);
        }
        out
    }

    #[test]
    fn idr_detection_walks_all_nals() {
        // SEI (6) + IDR (5)
        assert!(avcc_contains_idr(&avcc(&[&[0x06, 0xaa], &[0x65, 0xbb]])));
        // SEI (6) + non-IDR slice (1)
        assert!(!avcc_contains_idr(&avcc(&[&[0x06, 0xaa], &[0x41, 0xbb]])));
        // Empty / truncated input never panics.
        assert!(!avcc_contains_idr(&[]));
        assert!(!avcc_contains_idr(&[0, 0, 0, 9, 0x65]));
    }
}
