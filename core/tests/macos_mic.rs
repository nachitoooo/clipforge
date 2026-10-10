//! The real thing on macOS: ScreenCaptureKit (screen + system audio) plus the
//! real microphone through `cpal`, with the hardware video encoder and the
//! AAC encoders, in both microphone modes. Ignored by default: needs screen
//! recording and microphone permission for the terminal running it, and it
//! records a few seconds of whatever the microphone hears (nothing is kept).
//!
//! Run with:
//! `cargo test -p clipforge-core --test macos_mic -- --ignored --nocapture`
//! Set `CLIPFORGE_KEEP_CLIP=1` to keep the clips for inspection.
#![cfg(target_os = "macos")]

use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;

use clipforge_core::audio::aac_audiotoolbox::AudioToolboxAacEncoder;
use clipforge_core::audio::mic::{
    list_input_devices, mic_audio_config, CpalMicSource, MicControls,
};
use clipforge_core::audio::{AudioConfig, AudioEncoder, MicMode};
use clipforge_core::buffer::BufferConfig;
use clipforge_core::capture::macos::{ScreenCaptureConfig, ScreenCaptureKitSource};
use clipforge_core::capture::AudioSource;
use clipforge_core::clip::ClipService;
use clipforge_core::encode::videotoolbox::VideoToolboxEncoder;
use clipforge_core::encode::EncoderConfig;
use clipforge_core::engine::CaptureEngine;

fn audio_track_count(path: &std::path::Path) -> usize {
    let size = std::fs::metadata(path).unwrap().len();
    let mp4 = mp4::Mp4Reader::read_header(BufReader::new(File::open(path).unwrap()), size).unwrap();
    mp4.tracks()
        .values()
        .filter(|t| t.track_type().unwrap() == mp4::TrackType::Audio)
        .count()
}

fn run(mode: MicMode) {
    let dir = std::env::temp_dir().join(format!("clipforge-macos-mic-{mode:?}"));
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
    engine
        .start_with_audio(
            Box::new(ScreenCaptureKitSource::new(ScreenCaptureConfig {
                width: Some(video.width),
                height: Some(video.height),
                fps: video.fps,
                show_cursor: false,
                capture_audio: true,
            })),
            Box::new(VideoToolboxEncoder::new(video).expect("hardware encoder")),
            Some(Box::new(
                AudioToolboxAacEncoder::new(AudioConfig::default()).unwrap(),
            )),
            Arc::clone(&clips),
        )
        .unwrap_or_else(|e| panic!("capture failed to start (permission?): {e}"));
    std::thread::sleep(Duration::from_millis(800));

    let controls = MicControls::new(1.0, false);
    let epoch = engine.epoch().expect("ScreenCaptureKit shares its epoch");
    let mut mic = CpalMicSource::new(None, Arc::clone(&controls), epoch);
    let rx = mic
        .start()
        .unwrap_or_else(|e| panic!("microphone failed to start: {e}"));
    let encoder: Option<Box<dyn AudioEncoder>> = (mode == MicMode::Separate)
        .then(|| Box::new(AudioToolboxAacEncoder::new(mic_audio_config()).unwrap()) as _);
    engine
        .attach_microphone(Box::new(mic), rx, mode, encoder)
        .unwrap();

    std::thread::sleep(Duration::from_secs(4));
    let saved = clips.save_last(Duration::from_secs(3)).unwrap();
    let stats = engine.stats();
    let peak = controls.level.take_peak();
    let heard = controls.input.heard_signal();
    engine.stop();

    println!(
        "{mode:?}: {} video / {} audio / {} mic packets in the clip; mic peak {peak:.4}, signal heard: {heard}; stats {stats:?}",
        saved.packets, saved.audio_packets, saved.mic_packets
    );

    assert!(
        saved.audio_packets > 50,
        "game audio packets: {}",
        saved.audio_packets
    );
    match mode {
        MicMode::Separate => {
            assert!(saved.mic_packets > 50, "mic packets: {}", saved.mic_packets);
            assert_eq!(audio_track_count(&saved.path), 2);
        }
        MicMode::Mixed => {
            assert_eq!(saved.mic_packets, 0);
            assert_eq!(audio_track_count(&saved.path), 1);
        }
    }
    assert_eq!(stats.mic_errors, 0);
    assert!(
        heard,
        "the microphone delivered only digital silence: permission denied?"
    );
    assert!(
        controls.error().is_none(),
        "stream error: {:?}",
        controls.error()
    );

    if std::env::var_os("CLIPFORGE_KEEP_CLIP").is_some() {
        println!("kept clip at {}", saved.path.display());
    } else {
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
#[ignore = "records the real screen, system audio and microphone; needs permissions"]
fn macos_mic_smoke_mixed() {
    run(MicMode::Mixed);
}

#[test]
#[ignore = "records the real screen, system audio and microphone; needs permissions"]
fn macos_mic_smoke_separate() {
    run(MicMode::Separate);
}

#[test]
#[ignore = "talks to the real audio service"]
fn macos_lists_at_least_one_microphone() {
    let devices = list_input_devices().expect("device enumeration works");
    println!("microphones: {devices:#?}");
    assert!(!devices.is_empty(), "no input devices found");
    assert!(
        devices.iter().filter(|d| d.is_default).count() <= 1,
        "at most one default"
    );
    // The default comes first, and ids are unique.
    if let Some(i) = devices.iter().position(|d| d.is_default) {
        assert_eq!(i, 0);
    }
    let mut ids: Vec<_> = devices.iter().map(|d| d.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), devices.len());
}
