//! Real-screen integration test for the ScreenCaptureKit source. Ignored by
//! default: needs macOS, a display, and screen-recording permission for the
//! terminal/IDE running it.
//!
//! Run with: `cargo test -p clipforge-core -- --ignored macos_capture_smoke`

#![cfg(target_os = "macos")]

use std::time::{Duration, Instant};

use clipforge_core::capture::macos::{ScreenCaptureConfig, ScreenCaptureKitSource};
use clipforge_core::capture::{CaptureError, FrameSource};

#[tokio::test]
#[ignore = "captures the real screen; needs screen-recording permission"]
async fn macos_capture_smoke() {
    const CAPTURE_SECS: u64 = 3;
    const FPS: u32 = 60;

    let mut source = ScreenCaptureKitSource::new(ScreenCaptureConfig {
        fps: FPS,
        ..Default::default()
    });

    let mut rx = match source.start() {
        Ok(rx) => rx,
        Err(CaptureError::PermissionDenied) => {
            panic!(
                "screen recording permission denied — grant it to this terminal in \
                 System Settings → Privacy & Security → Screen Recording and re-run"
            );
        }
        Err(e) => panic!("capture failed to start: {e}"),
    };

    let deadline = Instant::now() + Duration::from_secs(CAPTURE_SECS);
    let mut frames: u64 = 0;
    let mut last_ts = None;
    let mut expected_len = None;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(frame)) => {
                frames += 1;
                // Contract checks on real frames.
                assert_eq!(
                    frame.data.len(),
                    frame.expected_len(),
                    "tightly packed BGRA"
                );
                if let Some(prev) = last_ts {
                    assert!(frame.timestamp > prev, "timestamps must strictly increase");
                }
                last_ts = Some(frame.timestamp);
                let len = frame.data.len();
                if let Some(expected) = expected_len {
                    assert_eq!(len, expected, "frame size must not change mid-capture");
                }
                expected_len = Some(len);
            }
            Ok(None) => panic!("frame stream ended early"),
            Err(_) => break, // deadline hit while waiting
        }
    }
    source.stop();

    let stats = source.stats();
    let measured_fps = frames as f64 / CAPTURE_SECS as f64;
    let drop_rate = stats.drop_rate();
    println!(
        "macos_capture_smoke: {frames} frames in {CAPTURE_SECS} s ({measured_fps:.1} fps), \
         delivered={}, dropped={}, drop_rate={:.3}%",
        stats.delivered,
        stats.dropped,
        drop_rate * 100.0
    );

    // A static screen legitimately produces few frames (SCK skips unchanged
    // content), so assert on drops — the acceptance criterion — not on fps.
    assert!(frames > 0, "no frames captured in {CAPTURE_SECS} s");
    assert!(
        drop_rate < 0.01,
        "frame drop rate {:.3}% exceeds 1%",
        drop_rate * 100.0
    );
}
