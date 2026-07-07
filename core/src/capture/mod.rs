//! Frame sources: one per OS behind the [`FrameSource`] trait, plus a
//! synthetic [`test_pattern::TestPatternSource`] so the whole pipeline is
//! testable without a real screen.

pub mod test_pattern;

use std::time::Duration;

use thiserror::Error;
use tokio::sync::mpsc;

/// A single uncompressed frame handed from a capture source to the encoder.
///
/// Pixel data is tightly packed 8-bit BGRA (`width * height * 4` bytes).
#[derive(Clone)]
pub struct RawFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Presentation time relative to the source's capture epoch. Strictly
    /// increasing within one capture session.
    pub timestamp: Duration,
    /// Sequence number assigned by the source (0-based, counts *delivered*
    /// frames; gaps in wall-clock time do not create gaps here).
    pub frame_number: u64,
}

impl RawFrame {
    /// Byte length the `data` buffer must have for this frame's dimensions.
    pub fn expected_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

impl std::fmt::Debug for RawFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("timestamp", &self.timestamp)
            .field("frame_number", &self.frame_number)
            .field("data_len", &self.data.len())
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("screen recording permission denied — grant it in System Settings → Privacy & Security → Screen Recording, then restart the app")]
    PermissionDenied,
    #[error("no capturable display found")]
    NoDisplay,
    #[error("capture source already started")]
    AlreadyStarted,
    #[error("capture backend error: {0}")]
    Backend(String),
}

/// Statistics a source keeps while running; used to verify the <1% drop-rate
/// acceptance criterion and surfaced in logs/UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureStats {
    /// Frames delivered downstream.
    pub delivered: u64,
    /// Frames the backend produced but we dropped (full channel, stalls).
    pub dropped: u64,
}

impl CaptureStats {
    pub fn drop_rate(&self) -> f64 {
        let total = self.delivered + self.dropped;
        if total == 0 {
            0.0
        } else {
            self.dropped as f64 / total as f64
        }
    }
}

/// A source of uncompressed frames (screen capture backend or test pattern).
///
/// `start` returns the receiving end of an async frame stream; the source
/// pushes frames from its own capture thread/callbacks. Dropping the receiver
/// or calling `stop` ends the stream.
pub trait FrameSource: Send {
    /// Begin capturing. Returns the frame stream; frames arrive in
    /// presentation order with strictly increasing timestamps.
    fn start(&mut self) -> Result<mpsc::Receiver<RawFrame>, CaptureError>;

    /// Stop capturing and release backend resources. Idempotent.
    fn stop(&mut self);

    /// Delivery/drop counters for the current (or last) session.
    fn stats(&self) -> CaptureStats;
}
