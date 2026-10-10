//! Real capture → clip with sound, end to end on macOS: ScreenCaptureKit
//! (video + system audio) → VideoToolbox + AudioToolbox AAC → replay buffer →
//! .mp4. Ignored by default: needs a display, screen-recording permission for
//! the terminal running it, and (for the audio assertions) something playing.
//!
//! Run with: `cargo test -p clipforge-core --test macos_audio -- --ignored --nocapture`
//! Set `CLIPFORGE_KEEP_CLIP=1` to keep the clip for inspection.
//! Set `CLIPFORGE_EXPECT_AUDIO=1` to *require* audio (play something first);
//! otherwise a silent system only produces a note, because ScreenCaptureKit
//! may deliver nothing while no app is making sound.

#![cfg(target_os = "macos")]

use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;

use clipforge_core::audio::aac_audiotoolbox::AudioToolboxAacEncoder;
use clipforge_core::audio::AudioConfig;
use clipforge_core::buffer::BufferConfig;
use clipforge_core::capture::macos::{ScreenCaptureConfig, ScreenCaptureKitSource};
use clipforge_core::clip::ClipService;
use clipforge_core::encode::videotoolbox::VideoToolboxEncoder;
use clipforge_core::encode::EncoderConfig;
use clipforge_core::engine::CaptureEngine;

#[test]
#[ignore = "captures the real screen and system audio; needs screen-recording permission"]
fn macos_audio_smoke() {
    let dir = std::env::temp_dir().join("clipforge-macos-audio-smoke");
    std::fs::remove_dir_all(&dir).ok();

    let video = EncoderConfig {
        width: 1280,
        height: 720,
        fps: 60,
        keyframe_interval: 60,
        ..Default::default()
    };
    let mut engine = CaptureEngine::new(BufferConfig::default());
    let clips = Arc::new(ClipService::new(engine.buffer(), &dir));
    let source = ScreenCaptureKitSource::new(ScreenCaptureConfig {
        width: Some(video.width),
        height: Some(video.height),
        fps: video.fps,
        show_cursor: false,
        capture_audio: true,
    });
    engine
        .start_with_audio(
            Box::new(source),
            Box::new(VideoToolboxEncoder::new(video).expect("hardware encoder")),
            Some(Box::new(
                AudioToolboxAacEncoder::new(AudioConfig::default()).expect("AAC encoder"),
            )),
            Arc::clone(&clips),
        )
        .unwrap_or_else(|e| panic!("capture failed to start (permission?): {e}"));

    std::thread::sleep(Duration::from_secs(5));
    let saved = clips.save_last(Duration::from_secs(4));
    engine.stop();
    let stats = engine.stats();
    println!("engine stats: {stats:?}");
    let saved = saved.expect("a clip can be saved after 5 s of capture");
    println!(
        "clip: {:?}, {} video packets, {} audio packets, {} bytes",
        saved.duration, saved.packets, saved.audio_packets, saved.bytes
    );

    let size = std::fs::metadata(&saved.path).unwrap().len();
    let mp4 = mp4::Mp4Reader::read_header(BufReader::new(File::open(&saved.path).unwrap()), size)
        .unwrap();
    let kinds: Vec<_> = mp4
        .tracks()
        .values()
        .map(|t| t.track_type().unwrap())
        .collect();
    assert!(
        kinds.contains(&mp4::TrackType::Video),
        "video track present"
    );

    if std::env::var_os("CLIPFORGE_EXPECT_AUDIO").is_some() {
        assert!(
            stats.audio_packets_buffered > 0,
            "no audio reached the buffer: {stats:?}"
        );
        assert!(
            kinds.contains(&mp4::TrackType::Audio),
            "clip has an audio track"
        );
        let video = mp4
            .tracks()
            .values()
            .find(|t| t.track_type().unwrap() == mp4::TrackType::Video)
            .unwrap();
        let audio = mp4
            .tracks()
            .values()
            .find(|t| t.track_type().unwrap() == mp4::TrackType::Audio)
            .unwrap();
        let (v, a) = (video.duration(), audio.duration());
        assert!(
            v.abs_diff(a) < Duration::from_millis(300),
            "video {v:?} vs audio {a:?}"
        );
    } else if stats.audio_packets_buffered == 0 {
        println!("note: no audio was delivered (nothing playing?); rerun with sound and CLIPFORGE_EXPECT_AUDIO=1");
    }
    if std::env::var_os("CLIPFORGE_KEEP_CLIP").is_some() {
        println!("kept clip at {}", saved.path.display());
    } else {
        std::fs::remove_dir_all(&dir).ok();
    }
}
