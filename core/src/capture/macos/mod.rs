//! macOS screen capture via ScreenCaptureKit.
//!
//! [`ScreenCaptureKitSource`] captures the main display and implements
//! [`FrameSource`]: SCK delivers BGRA `CMSampleBuffer`s on its own dispatch
//! queue; the handler repacks them into tightly-packed [`RawFrame`]s and
//! pushes them into the async stream. The callback never blocks — if the
//! consumer lags, frames are dropped and counted.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use screencapturekit::cm::CMTime;
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::error::SCError;
use screencapturekit::prelude::*;
use tokio::sync::mpsc;

use super::{CaptureError, CaptureStats, FrameSource, RawFrame};

/// Frames buffered between the SCK callback queue and the consumer. Small on
/// purpose: at 60 fps, 8 frames ≈ 130 ms of slack before we start dropping.
const CHANNEL_CAPACITY: usize = 8;

#[derive(Debug, Clone)]
pub struct ScreenCaptureConfig {
    /// Output size in pixels; `None` captures at the display's native size.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: u32,
    pub show_cursor: bool,
}

impl Default for ScreenCaptureConfig {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            fps: 60,
            show_cursor: true,
        }
    }
}

/// [`FrameSource`] for the main display, backed by ScreenCaptureKit.
pub struct ScreenCaptureKitSource {
    config: ScreenCaptureConfig,
    stream: Option<SCStream>,
    delivered: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
}

impl ScreenCaptureKitSource {
    pub fn new(config: ScreenCaptureConfig) -> Self {
        Self {
            config,
            stream: None,
            delivered: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }
}

fn map_sc_error(err: SCError) -> CaptureError {
    match err {
        // SCK reports a TCC (permission) denial as "no shareable content".
        SCError::NoShareableContent(_) => CaptureError::PermissionDenied,
        SCError::DisplayNotFound(_) => CaptureError::NoDisplay,
        other => CaptureError::Backend(other.to_string()),
    }
}

fn cmtime_nanos(t: CMTime) -> i64 {
    if t.timescale == 0 {
        return 0;
    }
    (t.value as i128 * 1_000_000_000 / t.timescale as i128) as i64
}

/// SCK output handler: repacks each complete frame and forwards it.
struct FrameForwarder {
    tx: mpsc::Sender<RawFrame>,
    delivered: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    /// pts of the first delivered frame, in ns; -1 until seen. Timestamps
    /// handed downstream are relative to this epoch.
    epoch_ns: AtomicI64,
    /// Last relative timestamp delivered, in ns; ensures strict monotonicity.
    last_ns: AtomicI64,
    frame_number: AtomicU64,
}

impl FrameForwarder {
    fn forward(&self, sample: &CMSampleBuffer) -> Option<()> {
        // Idle/suspended frames (screen unchanged, display asleep) carry no
        // new image; skip them. A missing status attachment (seen on newer
        // macOS) is not a reason to discard a frame that has pixels.
        if sample.frame_status().is_some_and(|s| !s.is_complete()) {
            return None;
        }
        let pixel_buffer = sample.image_buffer()?;
        let pts_ns = cmtime_nanos(sample.output_presentation_timestamp());

        let epoch =
            match self
                .epoch_ns
                .compare_exchange(-1, pts_ns, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => pts_ns,
                Err(existing) => existing,
            };
        let rel_ns = pts_ns - epoch;
        if rel_ns < 0 || rel_ns <= self.last_ns.load(Ordering::SeqCst) {
            // Out-of-order or duplicate pts: drop rather than break the
            // monotonic-timestamp contract (first frame is exempt via -1).
            if self.frame_number.load(Ordering::SeqCst) > 0 {
                self.dropped.fetch_add(1, Ordering::SeqCst);
                return None;
            }
        }

        let guard = pixel_buffer.lock(CVPixelBufferLockFlags::READ_ONLY).ok()?;
        let (width, height) = (guard.width(), guard.height());
        let stride = guard.bytes_per_row();
        let row_len = width * 4;
        let src = guard.as_slice();
        if src.len() < stride * height || row_len > stride {
            self.dropped.fetch_add(1, Ordering::SeqCst);
            return None;
        }
        // Repack to tightly-packed BGRA (SCK pads rows to stride).
        let mut data = Vec::with_capacity(row_len * height);
        for row in 0..height {
            let start = row * stride;
            data.extend_from_slice(&src[start..start + row_len]);
        }
        drop(guard);

        let frame = RawFrame {
            data,
            width: width as u32,
            height: height as u32,
            timestamp: Duration::from_nanos(rel_ns.max(0) as u64),
            frame_number: self.frame_number.load(Ordering::SeqCst),
        };

        match self.tx.try_send(frame) {
            Ok(()) => {
                self.last_ns.store(rel_ns, Ordering::SeqCst);
                self.frame_number.fetch_add(1, Ordering::SeqCst);
                self.delivered.fetch_add(1, Ordering::SeqCst);
                Some(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::SeqCst);
                None
            }
            Err(mpsc::error::TrySendError::Closed(_)) => None,
        }
    }
}

impl SCStreamOutputTrait for FrameForwarder {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if std::env::var_os("CLIPFORGE_CAPTURE_DEBUG").is_some() {
            eprintln!(
                "[capture-debug] type={of_type:?} status={:?} has_image={}",
                sample.frame_status(),
                sample.image_buffer().is_some()
            );
        }
        if of_type == SCStreamOutputType::Screen {
            self.forward(&sample);
        }
    }
}

impl FrameSource for ScreenCaptureKitSource {
    fn start(&mut self) -> Result<mpsc::Receiver<RawFrame>, CaptureError> {
        if self.stream.is_some() {
            return Err(CaptureError::AlreadyStarted);
        }
        if self.config.fps == 0 {
            return Err(CaptureError::Backend("fps must be non-zero".into()));
        }

        let content = SCShareableContent::get().map_err(map_sc_error)?;
        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or(CaptureError::NoDisplay)?;

        let width = self.config.width.unwrap_or_else(|| display.width());
        let height = self.config.height.unwrap_or_else(|| display.height());

        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();
        let sc_config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(self.config.show_cursor)
            .with_minimum_frame_interval(&CMTime::new(1, self.config.fps as i32));

        self.delivered.store(0, Ordering::SeqCst);
        self.dropped.store(0, Ordering::SeqCst);

        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let handler = FrameForwarder {
            tx,
            delivered: Arc::clone(&self.delivered),
            dropped: Arc::clone(&self.dropped),
            epoch_ns: AtomicI64::new(-1),
            last_ns: AtomicI64::new(-1),
            frame_number: AtomicU64::new(0),
        };

        let mut stream = SCStream::new(&filter, &sc_config);
        stream.add_output_handler(handler, SCStreamOutputType::Screen);
        stream.start_capture().map_err(|e| match map_sc_error(e) {
            CaptureError::Backend(msg) if msg.to_lowercase().contains("declined") => {
                CaptureError::PermissionDenied
            }
            other => other,
        })?;

        self.stream = Some(stream);
        Ok(rx)
    }

    fn stop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.stop_capture();
        }
    }

    fn stats(&self) -> CaptureStats {
        CaptureStats {
            delivered: self.delivered.load(Ordering::SeqCst),
            dropped: self.dropped.load(Ordering::SeqCst),
        }
    }
}

impl Drop for ScreenCaptureKitSource {
    fn drop(&mut self) {
        self.stop();
    }
}
