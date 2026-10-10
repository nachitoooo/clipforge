//! End to end on macOS with the real AAC encoders: a "game" tone (440 Hz) and
//! a "voice" tone (880 Hz) go through the engine, the mixer and the muxer;
//! the clip is then decoded with AVFoundation and its tracks are measured.
//! Synthetic sources only — no screen or microphone permission needed.
#![cfg(target_os = "macos")]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use clipforge_core::audio::aac_audiotoolbox::AudioToolboxAacEncoder;
use clipforge_core::audio::test_tone::{TestToneConfig, TestToneSource};
use clipforge_core::audio::{AudioConfig, AudioEncoder, MicMode};
use clipforge_core::buffer::BufferConfig;
use clipforge_core::capture::combined::WithAudio;
use clipforge_core::capture::test_pattern::{TestPatternConfig, TestPatternSource};
use clipforge_core::capture::AudioSource;
use clipforge_core::clip::ClipService;
use clipforge_core::encode::mock::MockEncoder;
use clipforge_core::encode::EncoderConfig;
use clipforge_core::engine::CaptureEngine;

const RATE: f32 = 48_000.0;

/// Run a short session with a microphone in `mode`; return the saved clip.
fn record_clip(mode: MicMode, name: &str) -> clipforge_core::clip::SavedClip {
    let dir = std::env::temp_dir().join("clipforge-mic-mix").join(name);
    std::fs::remove_dir_all(&dir).ok();
    let base = Instant::now();
    let mut engine = CaptureEngine::new(BufferConfig::default());
    let clips = Arc::new(ClipService::new(engine.buffer(), &dir));

    let video = TestPatternSource::new(TestPatternConfig {
        width: 64,
        height: 48,
        fps: 60,
        realtime: true,
        ..Default::default()
    });
    let game = TestToneSource::new(TestToneConfig {
        frequency: 440.0,
        realtime: true,
        clock_base: Some(base),
        ..Default::default()
    });
    engine
        .start_with_audio(
            Box::new(WithAudio::new(video, game)),
            Box::new(MockEncoder::new(EncoderConfig {
                width: 64,
                height: 48,
                keyframe_interval: 30,
                ..Default::default()
            })),
            Some(Box::new(
                AudioToolboxAacEncoder::new(AudioConfig::default()).unwrap(),
            )),
            Arc::clone(&clips),
        )
        .unwrap();

    std::thread::sleep(Duration::from_millis(500));
    let mut voice = TestToneSource::new(TestToneConfig {
        channels: 1,
        frequency: 880.0,
        realtime: true,
        clock_base: Some(base),
        ..Default::default()
    });
    let rx = voice.start().unwrap();
    let mic_encoder: Option<Box<dyn AudioEncoder>> = (mode == MicMode::Separate).then(|| {
        Box::new(
            AudioToolboxAacEncoder::new(AudioConfig {
                channels: 1,
                bitrate: 96_000,
                ..Default::default()
            })
            .unwrap(),
        ) as Box<dyn AudioEncoder>
    });
    engine
        .attach_microphone(Box::new(voice), rx, mode, mic_encoder)
        .unwrap();

    std::thread::sleep(Duration::from_millis(3500));
    let saved = clips.save_last(Duration::from_secs(2)).unwrap();
    engine.stop();
    saved
}

/// The middle second of a decoded track.
fn middle(pcm: &[f32]) -> &[f32] {
    let start = RATE as usize / 2;
    &pcm[start..start + RATE as usize]
}

#[test]
fn mixed_mode_puts_the_voice_and_the_game_in_one_track() {
    let saved = record_clip(MicMode::Mixed, "mixed");
    assert_eq!(saved.mic_packets, 0);
    let Some(pcm) = support::decode_track(&saved.path, 0) else {
        eprintln!("no Swift toolchain; skipping decode checks");
        return;
    };
    let game = support::amplitude_at(middle(&pcm), RATE, 440.0);
    let voice = support::amplitude_at(middle(&pcm), RATE, 880.0);
    assert!(game > 0.3, "game tone missing: {game}");
    assert!(voice > 0.3, "voice tone missing from the mix: {voice}");
}

#[test]
fn separate_mode_keeps_each_source_on_its_own_track() {
    let saved = record_clip(MicMode::Separate, "separate");
    assert!(saved.mic_packets > 20);
    let (Some(main), Some(voice)) = (
        support::decode_track(&saved.path, 0),
        support::decode_track(&saved.path, 1),
    ) else {
        eprintln!("no Swift toolchain; skipping decode checks");
        return;
    };
    for (name, pcm) in [("track 0", &main), ("track 1", &voice)] {
        eprintln!(
            "{name}: {} samples, 440 Hz = {:.4}, 880 Hz = {:.4}, 1320 Hz = {:.4}",
            pcm.len(),
            support::amplitude_at(middle(pcm), RATE, 440.0),
            support::amplitude_at(middle(pcm), RATE, 880.0),
            support::amplitude_at(middle(pcm), RATE, 1320.0),
        );
    }
    // Track 0: the game only.
    assert!(support::amplitude_at(middle(&main), RATE, 440.0) > 0.3);
    assert!(support::amplitude_at(middle(&main), RATE, 880.0) < 0.05);
    // Track 1: the voice only.
    assert!(support::amplitude_at(middle(&voice), RATE, 880.0) > 0.3);
    assert!(support::amplitude_at(middle(&voice), RATE, 440.0) < 0.05);
}
