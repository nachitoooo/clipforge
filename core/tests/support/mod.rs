//! Helpers shared by the macOS integration tests that decode a clip's audio
//! the way QuickTime/Safari would (see `avf_decode.swift`).
#![allow(dead_code)]
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join("clipforge-test-support");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The AVFoundation decoder, compiled once per test process. `None` where
/// there is no Swift toolchain (the test then skips its decode checks).
pub fn avf_decoder() -> Option<&'static Path> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = scratch();
        let source = dir.join(format!("avf_decode-{}.swift", std::process::id()));
        let bin = dir.join(format!("avf_decode-{}", std::process::id()));
        std::fs::write(&source, include_str!("avf_decode.swift")).ok()?;
        let ok = Command::new("swiftc")
            .arg("-O")
            .arg(&source)
            .arg("-o")
            .arg(&bin)
            .output()
            .ok()?
            .status
            .success();
        ok.then_some(bin)
    })
    .as_deref()
}

/// What AVFoundation makes of the video track of `mp4`: the container
/// duration and the frames it actually decodes (presentation order).
#[derive(Debug, Clone, Copy)]
pub struct VideoView {
    pub duration: f64,
    pub frames: usize,
    pub first: f64,
    pub last: f64,
}

/// Decode the video of `mp4` with AVFoundation; `None` without a Swift
/// toolchain.
pub fn avf_video(mp4: &Path) -> Option<VideoView> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    let bin = BIN
        .get_or_init(|| {
            let dir = scratch();
            let source = dir.join(format!("avf_video-{}.swift", std::process::id()));
            let bin = dir.join(format!("avf_video-{}", std::process::id()));
            std::fs::write(&source, include_str!("avf_video.swift")).ok()?;
            let ok = Command::new("swiftc")
                .arg("-O")
                .arg(&source)
                .arg("-o")
                .arg(&bin)
                .output()
                .ok()?
                .status
                .success();
            ok.then_some(bin)
        })
        .as_deref()?;
    let output = Command::new(bin).arg(mp4).output().ok()?;
    assert!(
        output.status.success(),
        "avf_video failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |name: &str| -> f64 {
        text.split_whitespace()
            .find_map(|part| part.strip_prefix(name)?.strip_prefix('='))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("no {name} in {text:?}"))
    };
    Some(VideoView {
        duration: field("duration"),
        frames: field("frames") as usize,
        first: field("first"),
        last: field("last"),
    })
}

/// Left channel of audio track `track` (0 = first), on the clip's timeline.
pub fn decode_track(mp4: &Path, track: usize) -> Option<Vec<f32>> {
    let decoder = avf_decoder()?;
    // Next to the clip, so tests running in parallel never share an output file.
    let raw = mp4.with_extension(format!("track{track}.f32"));
    let output = Command::new(decoder)
        .arg(mp4)
        .arg(&raw)
        .arg(track.to_string())
        .output()
        .ok()?;
    assert!(
        output.status.success(),
        "avf_decode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(&raw).unwrap();
    std::fs::remove_file(&raw).ok();
    Some(
        bytes
            .chunks_exact(8) // interleaved L,R f32
            .map(|f| f32::from_le_bytes(f[..4].try_into().unwrap()))
            .collect(),
    )
}

/// Amplitude of the `freq` Hz component of `samples` (Goertzel). Exact for a
/// sine of that frequency over a whole number of cycles.
pub fn amplitude_at(samples: &[f32], rate: f32, freq: f32) -> f32 {
    let w = std::f32::consts::TAU * freq / rate;
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    let coeff = 2.0 * w.cos();
    for &x in samples {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    2.0 * power.max(0.0).sqrt() / samples.len() as f32
}
