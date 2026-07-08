//! CaptureEngine wiring test: TestPatternSource → MockEncoder → ReplayBuffer
//! → ClipService, all on the engine's background thread.

use std::sync::Arc;
use std::time::Duration;

use clipforge_core::buffer::BufferConfig;
use clipforge_core::capture::test_pattern::{TestPatternConfig, TestPatternSource};
use clipforge_core::clip::ClipService;
use clipforge_core::encode::mock::MockEncoder;
use clipforge_core::encode::EncoderConfig;
use clipforge_core::engine::CaptureEngine;

fn small_config() -> EncoderConfig {
    EncoderConfig {
        width: 64,
        height: 48,
        fps: 60,
        keyframe_interval: 30,
        ..Default::default()
    }
}

#[test]
fn engine_fills_buffer_and_clips_can_be_saved_while_running() {
    let dir = std::env::temp_dir().join("clipforge-engine-test");
    std::fs::remove_dir_all(&dir).ok();

    let config = small_config();
    let mut engine = CaptureEngine::new(BufferConfig::default());
    let clips = Arc::new(ClipService::new(engine.buffer(), &dir));

    let source = TestPatternSource::new(TestPatternConfig {
        width: config.width,
        height: config.height,
        fps: config.fps,
        frame_limit: Some(600), // 10 s at 60 fps
        realtime: false,
    });
    engine
        .start(
            Box::new(source),
            Box::new(MockEncoder::new(config)),
            Arc::clone(&clips),
        )
        .expect("engine starts");
    assert!(engine.is_running());

    // Clip save must work *while* the engine is running, as soon as enough
    // is buffered.
    let mut mid_save = None;
    for _ in 0..200 {
        match clips.save_last(Duration::from_secs(1)) {
            Ok(saved) => {
                mid_save = Some(saved);
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mid_save = mid_save.expect("clip save while engine runs");
    assert!(mid_save.path.exists());

    // Let the source run out, then verify totals.
    engine.join();
    let stats = engine.stats();
    assert_eq!(stats.frames_encoded, 600);
    assert_eq!(stats.packets_buffered, 600, "finish() drained the pipeline");
    assert_eq!(stats.encode_errors, 0);

    // Buffered window spans the whole 10 s stream and is still extractable
    // after the engine stopped.
    let final_save = clips
        .save_last(Duration::from_secs(8))
        .expect("save after stop");
    assert!(final_save.duration >= Duration::from_secs(8));
    assert_ne!(final_save.path, mid_save.path);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn double_start_is_rejected_and_stop_is_idempotent() {
    let config = small_config();
    let mut engine = CaptureEngine::new(BufferConfig::default());
    let clips = Arc::new(ClipService::new(
        engine.buffer(),
        std::env::temp_dir().join("clipforge-engine-test2"),
    ));

    let make_source = || {
        Box::new(TestPatternSource::new(TestPatternConfig {
            width: 64,
            height: 48,
            fps: 60,
            frame_limit: None,
            realtime: true,
        }))
    };
    engine
        .start(
            make_source(),
            Box::new(MockEncoder::new(config.clone())),
            Arc::clone(&clips),
        )
        .unwrap();
    assert!(matches!(
        engine.start(
            make_source(),
            Box::new(MockEncoder::new(config)),
            Arc::clone(&clips),
        ),
        Err(clipforge_core::engine::EngineError::AlreadyRunning)
    ));
    engine.stop();
    assert!(!engine.is_running());
    engine.stop(); // idempotent
}
