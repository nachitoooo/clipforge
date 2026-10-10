//! Trimmed clips as AVFoundation (QuickTime, Safari, the Tauri webview on
//! macOS) plays them: the video edit list must hide the lead-in frames and the
//! audio must still line up. The source is `tests/fixtures/tone.mp4`, a real
//! H.264 + AAC clip: 4 s, a keyframe every second, silence until 2.0 s and a
//! 440 Hz tone after that.
#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use clipforge_core::clip::trim::{trim_clip, TrimSpec};

const RATE: usize = 48_000;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.mp4")
}

fn cut(name: &str, start: f64, end: f64) -> PathBuf {
    let dir = std::env::temp_dir().join("clipforge-macos-trim");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join(format!("{name}-{}.mp4", std::process::id()));
    trim_clip(
        &fixture(),
        &out,
        &TrimSpec {
            start: Duration::from_secs_f64(start),
            end: Duration::from_secs_f64(end),
            keep_audio: vec![0],
        },
    )
    .unwrap();
    out
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
