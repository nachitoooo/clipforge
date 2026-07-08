//! Hardware encoders behind the [`Encoder`] trait: VideoToolbox on macOS,
//! Media Foundation on Windows. Software encoding is never used — clips are
//! remuxed, not re-encoded.

pub mod mock;
#[cfg(target_os = "macos")]
pub mod videotoolbox;

use std::time::Duration;

use thiserror::Error;

use crate::capture::RawFrame;

/// Codec produced by an encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

/// Encoder configuration. `Default` gives the documented 1080p60 baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct EncoderConfig {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Average bitrate in bits/s. 12 Mb/s keeps 1080p60 game footage sharp
    /// while a 120 s buffer stays near ~180 MB of encoded data.
    pub bitrate: u32,
    /// Keyframe interval in frames (GOP length). Ring-buffer eviction is
    /// GOP-aligned, so this bounds extraction granularity: 2 s @ 60 fps.
    pub keyframe_interval: u32,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            codec: Codec::H264,
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate: 12_000_000,
            keyframe_interval: 120,
        }
    }
}

/// One encoded video packet (an access unit) leaving an encoder.
#[derive(Clone)]
pub struct EncodedPacket {
    /// Codec bitstream in length-prefixed (AVCC/HVCC) form, parameter sets
    /// excluded — those live in [`Encoder::parameter_sets`].
    pub data: Vec<u8>,
    pub pts: Duration,
    /// Decode timestamp; equals `pts` while B-frames are disabled (they are,
    /// for low-latency capture).
    pub dts: Duration,
    /// True when this packet starts with a keyframe (IDR). The ring buffer
    /// relies on this flag for GOP-aligned eviction.
    pub is_keyframe: bool,
}

impl std::fmt::Debug for EncodedPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodedPacket")
            .field("pts", &self.pts)
            .field("dts", &self.dts)
            .field("is_keyframe", &self.is_keyframe)
            .field("data_len", &self.data.len())
            .finish()
    }
}

/// Codec parameter sets needed by the muxer (e.g. for the mp4 `avcC` box).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParameterSets {
    /// H.264 SPS / HEVC VPS+SPS, without start codes.
    pub sps: Vec<Vec<u8>>,
    /// PPS, without start codes.
    pub pps: Vec<Vec<u8>>,
}

#[derive(Debug, Error)]
pub enum EncodeError {
    #[error("no hardware encoder available for {0:?}")]
    HardwareUnavailable(Codec),
    #[error("frame does not match encoder config: {0}")]
    BadFrame(String),
    #[error("encoder backend error: {0}")]
    Backend(String),
}

/// A hardware video encoder: raw frames in, encoded packets out.
///
/// Hardware encoders pipeline internally, so one input frame may yield zero
/// packets now and several later; `finish` drains whatever is still in
/// flight.
pub trait Encoder: Send {
    /// Submit one frame. Returns any packets that became ready, in decode
    /// order.
    fn encode(&mut self, frame: &RawFrame) -> Result<Vec<EncodedPacket>, EncodeError>;

    /// Flush the pipeline and return all remaining packets. The encoder is
    /// finished afterwards; submitting more frames is an error.
    fn finish(&mut self) -> Result<Vec<EncodedPacket>, EncodeError>;

    /// Parameter sets for muxing. Available once the first packet has been
    /// produced (`None` before that).
    fn parameter_sets(&self) -> Option<ParameterSets>;

    fn config(&self) -> &EncoderConfig;
}
