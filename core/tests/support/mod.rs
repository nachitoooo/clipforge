//! Helpers shared by the macOS integration tests that decode a clip's audio
//! the way QuickTime/Safari would (see `avf_decode.swift`).
#![allow(dead_code)]
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

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

/// Run a short session (a "game" tone at 440 Hz from the start, a "voice"
/// tone at 880 Hz joining after 0.5 s) with real AAC encoders, with the
/// microphone in `mode`; save the last `save` of it and return the clip.
pub fn record_clip(mode: MicMode, name: &str, save: Duration) -> clipforge_core::clip::SavedClip {
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
    let saved = clips.save_last(save).unwrap();
    engine.stop();
    saved
}
