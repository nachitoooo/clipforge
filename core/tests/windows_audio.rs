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
