//! Synthetic frame source for tests: generates BGRA frames at a configurable
//! fps with the frame number embedded in the pixel data, so buffering,
//! timing, and encoding can be verified end-to-end without a real screen.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::{CaptureError, CaptureStats, FrameSource, RawFrame};

/// How many frames the stream buffers before the generator blocks (unpaced)
/// or drops (paced) — small on purpose, to mimic a real capture backend.
const CHANNEL_CAPACITY: usize = 8;

/// Marker preceding the embedded frame number in pixel row 0.
const MAGIC: [u8; 4] = *b"CFTP";

#[derive(Debug, Clone)]
pub struct TestPatternConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Stop after this many frames; `None` streams until `stop()`.
    pub frame_limit: Option<u64>,
    /// When true, frames are emitted in real time (1/fps apart on the wall
    /// clock) and dropped if the consumer lags — like a real screen. When
    /// false, frames are generated as fast as the consumer accepts them,
    /// with exact synthetic timestamps — right for deterministic tests.
    pub realtime: bool,
}

impl Default for TestPatternConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 60,
            frame_limit: None,
            realtime: false,
        }
    }
}

/// Synthetic [`FrameSource`]. See [`TestPatternConfig`].
pub struct TestPatternSource {
    config: TestPatternConfig,
    running: Arc<AtomicBool>,
    delivered: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    generator: Option<std::thread::JoinHandle<()>>,
}

impl TestPatternSource {
    pub fn new(config: TestPatternConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            delivered: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
            generator: None,
        }
    }

    /// Read back the frame number a generated frame carries in its pixels.
    /// Returns `None` if the frame doesn't start with the test-pattern magic
    /// (e.g. it came from a real capture source).
    pub fn embedded_frame_number(frame: &RawFrame) -> Option<u64> {
        let bytes = frame.data.get(..12)?;
        if bytes[..4] != MAGIC {
            return None;
        }
        Some(u64::from_le_bytes(bytes[4..12].try_into().unwrap()))
    }

    fn generate_frame(config: &TestPatternConfig, n: u64) -> RawFrame {
        let (w, h) = (config.width as usize, config.height as usize);
        let mut data = vec![0u8; w * h * 4];

        // Visible pattern: horizontal color bands that scroll one row per
        // frame, so consecutive frames differ everywhere (keeps encoders
        // honest) while staying cheap to generate.
        for y in 0..h {
            let band = ((y + n as usize) / 16) as u8;
            let (b, g, r) = (
                band.wrapping_mul(37),
                band.wrapping_mul(73),
                band.wrapping_mul(151),
            );
            let row = &mut data[y * w * 4..(y + 1) * w * 4];
            for px in row.chunks_exact_mut(4) {
                px.copy_from_slice(&[b, g, r, 255]);
            }
        }

        // Machine-readable frame number at the start of row 0.
        data[..4].copy_from_slice(&MAGIC);
        data[4..12].copy_from_slice(&n.to_le_bytes());

        RawFrame {
            data,
            width: config.width,
            height: config.height,
            timestamp: frame_timestamp(config.fps, n),
            frame_number: n,
        }
    }
}

/// Exact presentation time of frame `n` at `fps` (n/fps in nanoseconds).
fn frame_timestamp(fps: u32, n: u64) -> Duration {
    Duration::from_nanos(n * 1_000_000_000 / fps as u64)
}

impl FrameSource for TestPatternSource {
    fn start(&mut self) -> Result<mpsc::Receiver<RawFrame>, CaptureError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CaptureError::AlreadyStarted);
        }
        if self.config.fps == 0 || self.config.width == 0 || self.config.height == 0 {
            self.running.store(false, Ordering::SeqCst);
            return Err(CaptureError::Backend(
                "fps, width and height must be non-zero".into(),
            ));
        }
        self.delivered.store(0, Ordering::SeqCst);
        self.dropped.store(0, Ordering::SeqCst);

        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let config = self.config.clone();
        let running = Arc::clone(&self.running);
        let delivered = Arc::clone(&self.delivered);
        let dropped = Arc::clone(&self.dropped);

        self.generator = Some(std::thread::spawn(move || {
            let interval = Duration::from_nanos(1_000_000_000 / config.fps as u64);
            let epoch = std::time::Instant::now();
            let mut n: u64 = 0;

            while running.load(Ordering::SeqCst) {
                if config.frame_limit.is_some_and(|limit| n >= limit) {
                    break;
                }
                let frame = TestPatternSource::generate_frame(&config, n);

                if config.realtime {
                    // Pace to the wall clock; a slow consumer loses frames.
                    let due = interval * n as u32;
                    if let Some(wait) = due.checked_sub(epoch.elapsed()) {
                        std::thread::sleep(wait);
                    }
                    match tx.try_send(frame) {
                        Ok(()) => {
                            delivered.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            dropped.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                    }
                } else {
                    // Deterministic mode: block until the consumer takes it.
                    if tx.blocking_send(frame).is_err() {
                        break;
                    }
                    delivered.fetch_add(1, Ordering::SeqCst);
                }
                n += 1;
            }
            running.store(false, Ordering::SeqCst);
        }));

        Ok(rx)
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.generator.take() {
            let _ = handle.join();
        }
    }

    fn stats(&self) -> CaptureStats {
        CaptureStats {
            delivered: self.delivered.load(Ordering::SeqCst),
            dropped: self.dropped.load(Ordering::SeqCst),
        }
    }
}

impl Drop for TestPatternSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small(frame_limit: u64) -> TestPatternSource {
        TestPatternSource::new(TestPatternConfig {
            width: 64,
            height: 48,
            fps: 60,
            frame_limit: Some(frame_limit),
            realtime: false,
        })
    }

    #[tokio::test]
    async fn frames_carry_sequential_embedded_numbers() {
        let mut source = small(10);
        let mut rx = source.start().unwrap();
        for expected in 0..10 {
            let frame = rx.recv().await.expect("frame");
            assert_eq!(frame.frame_number, expected);
            assert_eq!(
                TestPatternSource::embedded_frame_number(&frame),
                Some(expected)
            );
            assert_eq!(frame.data.len(), frame.expected_len());
        }
        assert!(rx.recv().await.is_none(), "stream ends at frame_limit");
        assert_eq!(source.stats().delivered, 10);
        assert_eq!(source.stats().dropped, 0);
    }

    #[tokio::test]
    async fn timestamps_are_strictly_monotonic_and_exact() {
        let mut source = small(120);
        let mut rx = source.start().unwrap();
        let mut last = None;
        while let Some(frame) = rx.recv().await {
            assert_eq!(
                frame.timestamp,
                frame_timestamp(60, frame.frame_number),
                "timestamp must be exactly n/fps"
            );
            if let Some(prev) = last {
                assert!(frame.timestamp > prev, "timestamps must strictly increase");
            }
            last = Some(frame.timestamp);
        }
    }

    #[tokio::test]
    async fn consecutive_frames_differ_visibly() {
        let mut source = small(2);
        let mut rx = source.start().unwrap();
        let a = rx.recv().await.unwrap();
        let b = rx.recv().await.unwrap();
        // Compare away from the embedded header.
        assert_ne!(a.data[1024..], b.data[1024..]);
    }

    #[tokio::test]
    async fn double_start_is_rejected() {
        let mut source = TestPatternSource::new(TestPatternConfig {
            frame_limit: Some(1_000_000),
            width: 8,
            height: 8,
            ..Default::default()
        });
        let _rx = source.start().unwrap();
        assert!(matches!(source.start(), Err(CaptureError::AlreadyStarted)));
    }

    #[tokio::test]
    async fn stop_ends_the_stream() {
        let mut source = TestPatternSource::new(TestPatternConfig {
            width: 8,
            height: 8,
            frame_limit: None,
            ..Default::default()
        });
        let mut rx = source.start().unwrap();
        let _ = rx.recv().await.unwrap();
        source.stop();
        // Drain whatever was in flight; the stream must then end.
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn realtime_mode_paces_and_reports_drops() {
        let mut source = TestPatternSource::new(TestPatternConfig {
            width: 8,
            height: 8,
            fps: 240,
            frame_limit: Some(48),
            realtime: true,
        });
        let mut rx = source.start().unwrap();
        let started = std::time::Instant::now();
        let mut received = 0u64;
        while let Some(_frame) = rx.recv().await {
            received += 1;
        }
        // 48 frames at 240 fps ≈ 200 ms of pacing.
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "realtime mode must pace emission"
        );
        let stats = source.stats();
        assert_eq!(stats.delivered, received);
        assert_eq!(stats.delivered + stats.dropped, 48);
    }
}
