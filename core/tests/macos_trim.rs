//! Trimmed clips as AVFoundation (QuickTime, Safari, the Tauri webview on
//! macOS) plays them: the video edit list must hide the lead-in frames and the
//! audio must still line up. The source is `tests/fixtures/tone.mp4`, a real
//! H.264 + AAC clip: 4 s, a keyframe every second, silence until 2.0 s and a
//! 440 Hz tone after that.
#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use clipforge_core::audio::MicMode;
use clipforge_core::clip::trim::{probe, trim_clip, TrimSpec};

const RATE: usize = 48_000;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.mp4")
}

fn cut(name: &str, start: f64, end: f64) -> PathBuf {
    cut_from(&fixture(), name, start, end, &[0])
}

fn cut_from(source: &Path, name: &str, start: f64, end: f64, keep_audio: &[usize]) -> PathBuf {
    let dir = std::env::temp_dir().join("clipforge-macos-trim");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join(format!("{name}-{}.mp4", std::process::id()));
    trim_clip(
        source,
        &out,
        &TrimSpec {
            start: Duration::from_secs_f64(start),
            end: Duration::from_secs_f64(end),
            keep_audio: keep_audio.to_vec(),
        },
    )
    .unwrap();
    out
}

/// Seconds until the track first carries a sound (10 ms windows).
fn onset(pcm: &[f32]) -> f64 {
    let window = RATE / 100;
    pcm.chunks(window)
        .position(|w| rms(w) > 0.05)
        .map_or(f64::INFINITY, |i| (i * window) as f64 / RATE as f64)
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[test]
fn avfoundation_hides_the_lead_in_frames_of_a_mid_gop_cut() {
    let out = cut("video", 1.5, 3.5);
    let Some(view) = support::avf_video(&out) else {
        eprintln!("no Swift toolchain: skipping the AVFoundation checks");
        return;
    };
    // 1.5 s sits in the middle of the 1 s..2 s GOP: 15 frames of lead-in are
    // stored, none of them may be shown.
    assert!(
        (view.duration - 2.0).abs() < 0.05,
        "container duration {view:?}"
    );
    assert_eq!(view.frames, 60, "{view:?}");
    assert!(view.first.abs() < 0.001, "first frame at {}", view.first);
    assert!(
        (view.last - (2.0 - 1.0 / 30.0)).abs() < 0.01,
        "last frame at {}",
        view.last
    );
    std::fs::remove_file(&out).ok();
}

#[test]
fn avfoundation_hears_the_tone_where_the_cut_says() {
    // The tone starts 0.5 s into the cut.
    let out = cut("audio", 1.5, 3.5);
    let Some(audio) = support::decode_track(&out, 0) else {
        eprintln!("no Swift toolchain: skipping the AVFoundation checks");
        return;
    };
    let before = rms(&audio[RATE / 10..RATE * 45 / 100]);
    let after = rms(&audio[RATE * 6 / 10..RATE * 19 / 10]);
    assert!(before < 0.01, "silence before the tone, got {before}");
    assert!(after > 0.2, "the tone after it, got {after}");
    let amp = support::amplitude_at(&audio[RATE * 6 / 10..RATE * 19 / 10], RATE as f32, 440.0);
    assert!(amp > 0.4, "440 Hz at about half scale, got {amp}");
    std::fs::remove_file(&out).ok();
}

#[test]
fn avfoundation_agrees_when_the_cut_starts_on_a_keyframe() {
    let out = cut("keyframe", 1.0, 3.0);
    let Some(view) = support::avf_video(&out) else {
        return;
    };
    assert_eq!(view.frames, 60, "{view:?}");
    assert!(view.first.abs() < 0.001);
    std::fs::remove_file(&out).ok();
}

#[test]
fn a_clip_from_the_real_pipeline_keeps_each_track_and_its_late_start() {
    // Real AudioToolbox AAC, two tracks: the game tone (440 Hz) from the
    // start, the voice (880 Hz) joining a moment later — so the voice track
    // starts with silence, written as an empty edit. (The video is a mock
    // encoder's, so only the audio is decoded here.)
    let source = support::record_clip(MicMode::Separate, "trim-source", Duration::from_secs(4));
    let (Some(game), Some(voice)) = (
        support::decode_track(&source.path, 0),
        support::decode_track(&source.path, 1),
    ) else {
        eprintln!("no Swift toolchain: skipping the AVFoundation checks");
        return;
    };
    let voice_enters = onset(&voice);
    eprintln!("the voice enters {voice_enters:.3} s into the source clip");
    assert!(
        voice_enters > 0.15,
        "the test needs a late voice: {voice_enters}"
    );
    let game_enters = onset(&game);
    eprintln!("the game enters {game_enters:.3} s into the source clip");

    let start = 0.1;
    let out = cut_from(&source.path, "real", start, 2.1, &[0, 1]);
    let info = probe(&out).unwrap();
    assert_eq!(info.audio.len(), 2);
    assert!((info.duration.as_secs_f64() - 2.0).abs() < 0.05, "{info:?}");

    let (Some(game_cut), Some(voice_cut)) = (
        support::decode_track(&out, 0),
        support::decode_track(&out, 1),
    ) else {
        return;
    };
    // The same sounds, `start` earlier.
    let entering = onset(&voice_cut);
    assert!(
        (entering - (voice_enters - start)).abs() < 0.03,
        "the voice enters at {entering:.3} s, expected {:.3} s",
        voice_enters - start
    );
    let game_entering = onset(&game_cut);
    assert!(
        (game_entering - (game_enters - start).max(0.0)).abs() < 0.03,
        "the game enters at {game_entering:.3} s, expected {:.3} s",
        (game_enters - start).max(0.0)
    );
    let second = |pcm: &[f32]| pcm[RATE..RATE * 2].to_vec();
    assert!(support::amplitude_at(&second(&game_cut), RATE as f32, 440.0) > 0.3);
    assert!(support::amplitude_at(&second(&game_cut), RATE as f32, 880.0) < 0.05);
    assert!(support::amplitude_at(&second(&voice_cut), RATE as f32, 880.0) > 0.3);
    assert!(support::amplitude_at(&second(&voice_cut), RATE as f32, 440.0) < 0.05);
    std::fs::remove_file(&out).ok();

    // Dropping the game track leaves the voice as the only (default) track.
    let voice_only = cut_from(&source.path, "voice-only", start, 2.1, &[1]);
    assert_eq!(probe(&voice_only).unwrap().audio.len(), 1);
    if let Some(pcm) = support::decode_track(&voice_only, 0) {
        assert!(support::amplitude_at(&pcm[RATE..RATE * 2], RATE as f32, 880.0) > 0.3);
        assert!((onset(&pcm) - (voice_enters - start)).abs() < 0.03);
    }
    std::fs::remove_file(&voice_only).ok();
}
