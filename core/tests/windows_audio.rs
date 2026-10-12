//! Windows-only audio integration tests: WASAPI loopback capture and the
//! Media Foundation AAC encoder. They need real OS audio services, so they
//! are `#[ignore]`d — run them on a Windows machine with
//! `cargo test -p clipforge-core --test windows_audio -- --ignored`.
//!
//! They were written without access to a Windows machine (the rest of the
//! audio code is verified on macOS); treat a first failure here as a bug to
//! investigate, not as flakiness.
#![cfg(target_os = "windows")]

use std::time::Duration;

use clipforge_core::audio::aac_mediafoundation::MediaFoundationAacEncoder;
use clipforge_core::audio::mic::{list_input_devices, CpalMicSource, MicControls};
use clipforge_core::audio::test_tone::{TestToneConfig, TestToneSource};
use clipforge_core::audio::wasapi::WasapiLoopbackSource;
use clipforge_core::audio::{AudioChunk, AudioConfig, AudioEncoder};
use clipforge_core::capture::clock::SharedEpoch;
use clipforge_core::capture::AudioSource;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

fn tone_chunks(seconds: u64) -> Vec<AudioChunk> {
    let mut source = TestToneSource::new(TestToneConfig {
        duration_limit: Some(Duration::from_secs(seconds)),
        ..Default::default()
    });
    let mut rx = source.start().unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.blocking_recv() {
        chunks.push(chunk);
    }
    chunks
}

fn qpc_now_ns() -> i64 {
    let (mut counter, mut frequency) = (0i64, 0i64);
    unsafe {
        QueryPerformanceCounter(&mut counter).unwrap();
        QueryPerformanceFrequency(&mut frequency).unwrap();
    }
    (counter as i128 * 1_000_000_000 / frequency as i128) as i64
}

#[test]
#[ignore = "needs the Windows Media Foundation AAC encoder"]
fn mf_aac_encodes_a_tone_into_gap_free_packets() {
    let mut encoder = MediaFoundationAacEncoder::new(AudioConfig::default())
        .expect("Media Foundation AAC encoder available");
    let mut packets = Vec::new();
    for chunk in tone_chunks(2) {
        packets.extend(encoder.encode(&chunk).unwrap());
    }
    packets.extend(encoder.finish().unwrap());

    // 2 s = 93.75 AAC frames; allow for the encoder's delay and flush.
    assert!(
        (90..=98).contains(&packets.len()),
        "{} packets",
        packets.len()
    );
    for p in &packets {
        assert!(
            (1..3000).contains(&p.data.len()),
            "AAC size {}",
            p.data.len()
        );
    }
    for pair in packets.windows(2) {
        let step = pair[1].pts - pair[0].pts;
        assert!(
            step.abs_diff(Duration::from_secs_f64(1024.0 / 48_000.0)) < Duration::from_micros(5),
            "packet spacing {step:?}"
        );
    }
    println!("priming assumed: {} frames", encoder.priming_frames());
}

#[test]
#[ignore = "needs the Windows Media Foundation AAC encoder"]
fn mf_aac_rejects_unsupported_bitrates() {
    assert!(MediaFoundationAacEncoder::new(AudioConfig {
        bitrate: 100_000,
        ..Default::default()
    })
    .is_err());
}

#[test]
#[ignore = "needs a Windows audio output device"]
fn wasapi_loopback_smoke() {
    let epoch = SharedEpoch::new();
    epoch.get_or_set(qpc_now_ns());
    let mut source = WasapiLoopbackSource::new(AudioConfig::default(), epoch);
    let mut rx = source.start().expect("loopback stream starts");

    // Loopback only produces packets while something is playing, so a silent
    // machine legitimately yields zero chunks: only check what does arrive.
    std::thread::sleep(Duration::from_millis(1500));
    source.stop();
    let mut chunks = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        chunks.push(chunk);
    }
    println!("loopback delivered {} chunks in 1.5 s", chunks.len());
    for pair in chunks.windows(2) {
        assert!(
            pair[1].timestamp >= pair[0].timestamp,
            "timestamps must not go back"
        );
    }
    for chunk in &chunks {
        assert_eq!(chunk.channels, 2);
        assert_eq!(chunk.sample_rate, 48_000);
    }
}

#[test]
#[ignore = "needs a Windows audio input device"]
fn cpal_lists_input_devices() {
    let devices = list_input_devices().expect("device enumeration works");
    println!("microphones: {devices:#?}");
    assert!(!devices.is_empty(), "no input devices found");
}

#[test]
#[ignore = "needs a Windows microphone and its privacy permission"]
fn cpal_microphone_smoke() {
    // cpal stamps capture instants with QueryPerformanceCounter on Windows,
    // the same clock as the epoch below.
    let epoch = SharedEpoch::new();
    epoch.get_or_set(qpc_now_ns());
    let controls = MicControls::new(1.0, false);
    let mut mic = CpalMicSource::new(None, controls.clone(), epoch);
    let mut rx = mic.start().expect("microphone starts (permission?)");
    std::thread::sleep(Duration::from_millis(1500));
    mic.stop();

    let mut chunks = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        chunks.push(chunk);
    }
    println!(
        "microphone delivered {} chunks; signal heard: {}",
        chunks.len(),
        controls.input.heard_signal()
    );
    assert!(
        chunks.len() > 50,
        "expected ~150 chunks in 1.5 s, got {}",
        chunks.len()
    );
    for pair in chunks.windows(2) {
        assert!(
            pair[1].timestamp > pair[0].timestamp,
            "timestamps must increase"
        );
    }
    for chunk in &chunks {
        assert_eq!((chunk.channels, chunk.sample_rate), (1, 48_000));
    }
    assert!(controls.error().is_none());
}

// ---- hearing what the computer plays ---------------------------------------------
//
// These need an audio output device. A CI runner has none until a virtual
// cable is installed (the `windows-video` workflow does that).

/// Write a stereo 16-bit 48 kHz WAV of a sine tone.
fn write_tone_wav(path: &std::path::Path, seconds: u32, hz: f32, amplitude: f32) {
    let rate = 48_000u32;
    let frames = rate * seconds;
    let mut pcm = Vec::with_capacity(frames as usize * 4);
    for n in 0..frames {
        let v = (amplitude
            * (2.0 * std::f32::consts::PI * hz * n as f32 / rate as f32).sin()
            * 32767.0) as i16;
        pcm.extend(v.to_le_bytes());
        pcm.extend(v.to_le_bytes());
    }
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + pcm.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes()); // PCM
    wav.extend(2u16.to_le_bytes());
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((pcm.len() as u32).to_le_bytes());
    wav.extend(pcm);
    std::fs::write(path, wav).unwrap();
}

/// Play `path` through the default output device, from another process.
fn play_in_background(path: &std::path::Path) -> std::process::Child {
    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(New-Object Media.SoundPlayer '{}').PlaySync()",
                path.display()
            ),
        ])
        .spawn()
        .expect("powershell plays the tone")
}

/// How loud `hz` is in interleaved samples, as a sine amplitude (0..1).
fn amplitude_at(samples: &[f32], channels: usize, rate: f32, hz: f32) -> f32 {
    let (mut re, mut im, mut n) = (0f64, 0f64, 0usize);
    for (i, frame) in samples.chunks_exact(channels).enumerate() {
        let mono = frame.iter().sum::<f32>() / channels as f32;
        let phase = 2.0 * std::f64::consts::PI * f64::from(hz) * i as f64 / f64::from(rate);
        re += f64::from(mono) * phase.cos();
        im += f64::from(mono) * phase.sin();
        n += 1;
    }
    (2.0 * (re * re + im * im).sqrt() / n.max(1) as f64) as f32
}

#[test]
#[ignore = "needs a Windows audio output device"]
fn loopback_hears_a_tone_that_is_playing_with_sane_timestamps() {
    let dir = std::env::temp_dir().join("clipforge-loopback");
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("tone-440.wav");
    write_tone_wav(&wav, 4, 440.0, 0.5);

    let epoch = SharedEpoch::new();
    epoch.get_or_set(qpc_now_ns());
    let mut source = WasapiLoopbackSource::new(AudioConfig::default(), epoch);
    let mut rx = source.start().expect("loopback stream starts");
    let mut player = play_in_background(&wav);

    std::thread::sleep(Duration::from_millis(3500));
    source.stop();
    let _ = player.kill();
    let _ = player.wait();
    let mut chunks = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        chunks.push(chunk);
    }
    println!("loopback delivered {} chunks in 3.5 s", chunks.len());
    assert!(
        chunks.len() > 100,
        "{} chunks: nothing was heard",
        chunks.len()
    );

    for pair in chunks.windows(2) {
        assert!(
            pair[1].timestamp >= pair[0].timestamp,
            "timestamps must not go back"
        );
    }
    // The loudest second of what was heard is the tone.
    let loud: Vec<f32> = chunks
        .iter()
        .flat_map(|c| c.samples.iter().copied())
        .collect();
    let at_440 = amplitude_at(&loud, 2, 48_000.0, 440.0);
    let at_1000 = amplitude_at(&loud, 2, 48_000.0, 1000.0);
    println!("amplitude at 440 Hz {at_440:.3}, at 1000 Hz {at_1000:.3}");
    assert!(at_440 > 0.02, "the 440 Hz tone is too quiet: {at_440}");
    assert!(
        at_440 > at_1000 * 10.0,
        "440 Hz should dominate 1 kHz: {at_440} vs {at_1000}"
    );
}
