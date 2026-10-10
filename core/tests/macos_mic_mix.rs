//! End to end on macOS with the real AAC encoders: a "game" tone (440 Hz) and
//! a "voice" tone (880 Hz) go through the engine, the mixer and the muxer;
//! the clip is then decoded with AVFoundation and its tracks are measured.
//! Synthetic sources only — no screen or microphone permission needed.
#![cfg(target_os = "macos")]

mod support;

use clipforge_core::audio::MicMode;

use std::time::Duration;

const RATE: f32 = 48_000.0;

/// The middle second of a decoded track.
fn middle(pcm: &[f32]) -> &[f32] {
    let start = RATE as usize / 2;
    &pcm[start..start + RATE as usize]
}

#[test]
fn mixed_mode_puts_the_voice_and_the_game_in_one_track() {
    let saved = support::record_clip(MicMode::Mixed, "mixed", Duration::from_secs(2));
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
    let saved = support::record_clip(MicMode::Separate, "separate", Duration::from_secs(2));
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
