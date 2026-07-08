//! Property-style tests for the GOP-aligned replay buffer (F-2.1).
//!
//! Invariants under test, across boundaries (wraparound, exact durations,
//! empty buffer, clock jumps):
//!   1. the buffer always starts at a keyframe;
//!   2. buffered duration never exceeds max_duration by more than one GOP;
//!   3. extractions start at a keyframe and cover the requested window;
//!   4. memory stays within budget for 2 min of 1080p60.

use std::time::Duration;

use clipforge_core::buffer::{BufferConfig, PushOutcome, ReplayBuffer};
use clipforge_core::encode::EncodedPacket;

const FPS: u64 = 60;
const GOP: u64 = 120; // 2 s at 60 fps, matches EncoderConfig::default()

/// Deterministic pseudo-random for property-style loops (no proptest dep).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn pts_of(n: u64) -> Duration {
    Duration::from_nanos(n * 1_000_000_000 / FPS)
}

/// Packet `n` with realistic 1080p60 @ 12 Mb/s sizes: keyframes ~4x larger
/// than delta frames, averaging 25 KB/frame.
fn packet(n: u64, size_jitter: u64) -> EncodedPacket {
    let is_keyframe = n.is_multiple_of(GOP);
    let base = if is_keyframe { 80_000 } else { 22_000 };
    EncodedPacket {
        data: vec![0u8; base + (size_jitter % 8_000) as usize],
        pts: pts_of(n),
        dts: pts_of(n),
        is_keyframe,
    }
}

fn fill(buffer: &mut ReplayBuffer, range: std::ops::Range<u64>, rng: &mut Lcg) {
    for n in range {
        buffer.push(packet(n, rng.next()));
    }
}

fn assert_starts_at_keyframe(buffer: &ReplayBuffer) {
    if !buffer.is_empty() {
        let first = &buffer.extract_last(Duration::from_secs(u64::MAX / 2))[0];
        assert!(first.is_keyframe, "buffer must start at a keyframe");
    }
}

#[test]
fn empty_buffer_behaves() {
    let buffer = ReplayBuffer::new(BufferConfig::default());
    assert!(buffer.is_empty());
    assert_eq!(buffer.duration(), Duration::ZERO);
    assert_eq!(buffer.memory_bytes(), 0);
    assert!(buffer.extract_last(Duration::from_secs(30)).is_empty());
}

#[test]
fn first_packet_must_be_a_keyframe() {
    let mut buffer = ReplayBuffer::new(BufferConfig::default());
    let delta = packet(1, 0);
    assert!(!delta.is_keyframe);
    assert_eq!(buffer.push(delta), PushOutcome::AwaitingKeyframe);
    assert!(buffer.is_empty());
    assert_eq!(buffer.push(packet(0, 0)), PushOutcome::Stored);
    assert_eq!(buffer.packet_count(), 1);
}

#[test]
fn wraparound_stays_bounded_and_keyframe_aligned() {
    let mut rng = Lcg(7);
    let config = BufferConfig {
        max_duration: Duration::from_secs(120),
        ..Default::default()
    };
    let mut buffer = ReplayBuffer::new(config.clone());
    // 10 minutes of stream through a 2-minute window.
    for n in 0..(600 * FPS) {
        buffer.push(packet(n, rng.next()));
        // Spot-check invariants at GOP boundaries (checking every frame is
        // O(n^2) with extract; boundaries are where eviction happens).
        if n.is_multiple_of(GOP) {
            assert!(
                buffer.duration() <= config.max_duration + Duration::from_secs(2),
                "duration {:?} exceeds window + one GOP at frame {n}",
                buffer.duration()
            );
        }
    }
    assert_starts_at_keyframe(&buffer);
    // Window must actually be full (near max_duration, not shrunk).
    assert!(
        buffer.duration() >= config.max_duration - Duration::from_secs(2),
        "window underfilled: {:?}",
        buffer.duration()
    );
}

#[test]
fn extraction_covers_requested_window_and_starts_at_keyframe() {
    let mut rng = Lcg(42);
    let mut buffer = ReplayBuffer::new(BufferConfig::default());
    fill(&mut buffer, 0..(120 * FPS), &mut rng);

    for &seconds in &[1u64, 5, 15, 30, 60, 90] {
        let clip = buffer.extract_last(Duration::from_secs(seconds));
        assert!(!clip.is_empty());
        assert!(
            clip[0].is_keyframe,
            "{seconds}s clip must start at keyframe"
        );
        let span = clip.last().unwrap().pts - clip[0].pts;
        assert!(
            span >= Duration::from_secs(seconds) - Duration::from_millis(20),
            "{seconds}s clip only spans {span:?}"
        );
        assert!(
            span <= Duration::from_secs(seconds) + Duration::from_secs(2),
            "{seconds}s clip overshoots by more than one GOP: {span:?}"
        );
        // pts strictly increasing within the clip.
        for pair in clip.windows(2) {
            assert!(pair[1].pts > pair[0].pts);
        }
    }
}

#[test]
fn extraction_longer_than_buffer_returns_everything() {
    let mut rng = Lcg(3);
    let mut buffer = ReplayBuffer::new(BufferConfig::default());
    fill(&mut buffer, 0..(10 * FPS), &mut rng); // 10 s buffered
    let clip = buffer.extract_last(Duration::from_secs(30));
    assert_eq!(clip.len(), buffer.packet_count(), "whole buffer returned");
    assert!(clip[0].is_keyframe);
}

#[test]
fn exact_duration_boundary() {
    // Buffer exactly max_duration long: nothing should be evicted, and the
    // full-window extraction returns the whole buffer.
    let config = BufferConfig {
        max_duration: Duration::from_secs(120),
        ..Default::default()
    };
    let mut rng = Lcg(11);
    let mut buffer = ReplayBuffer::new(config);
    let frames = 120 * FPS + 1; // last pts - first pts == exactly 120 s
    fill(&mut buffer, 0..frames, &mut rng);
    assert_eq!(buffer.duration(), Duration::from_secs(120));
    assert_eq!(buffer.packet_count() as u64, frames);
}

#[test]
fn backwards_clock_jump_resets_cleanly() {
    let mut rng = Lcg(5);
    let mut buffer = ReplayBuffer::new(BufferConfig::default());
    fill(&mut buffer, 0..(30 * FPS), &mut rng);
    assert!(buffer.duration() >= Duration::from_secs(29));

    // Encoder restart: pts starts over from 0 with a keyframe.
    let outcome = buffer.push(packet(0, rng.next()));
    assert_eq!(outcome, PushOutcome::ResetOnDiscontinuity(true));
    assert_eq!(buffer.packet_count(), 1);
    assert_starts_at_keyframe(&buffer);

    // Same discontinuity but landing on a delta frame: buffer empties and
    // waits for the next keyframe.
    fill(&mut buffer, 1..(5 * FPS), &mut rng);
    let outcome = buffer.push(packet(1, rng.next()));
    assert_eq!(outcome, PushOutcome::ResetOnDiscontinuity(false));
    assert!(buffer.is_empty());
}

#[test]
fn forward_clock_jump_evicts_stale_gops() {
    let mut rng = Lcg(13);
    let config = BufferConfig {
        max_duration: Duration::from_secs(120),
        ..Default::default()
    };
    let mut buffer = ReplayBuffer::new(config);
    fill(&mut buffer, 0..(60 * FPS), &mut rng); // 1 min of history

    // Machine slept 10 minutes: next frames arrive with a huge pts gap.
    let jump = 600 * FPS;
    fill(&mut buffer, jump..(jump + 2 * GOP + 1), &mut rng);

    // The pre-jump GOPs span more than max_duration relative to the new
    // frames, so they must all be gone.
    let clip = buffer.extract_last(Duration::from_secs(u64::MAX / 2));
    assert!(
        clip[0].pts >= pts_of(jump),
        "stale pre-jump packets survived: first pts {:?}",
        clip[0].pts
    );
    assert_starts_at_keyframe(&buffer);
}

#[test]
fn memory_stays_under_budget_for_two_minutes_of_1080p60() {
    // Acceptance: 2 min of 1080p60 within budget. With realistic packet
    // sizes for 12 Mb/s (avg ~26 KB/frame), 120 s ≈ 190 MB; budget 256 MB.
    let mut rng = Lcg(99);
    let config = BufferConfig::default();
    let budget = config.max_bytes;
    let mut buffer = ReplayBuffer::new(config);

    // Stream 6 minutes; the window keeps the last 2.
    let mut peak = 0usize;
    for n in 0..(360 * FPS) {
        buffer.push(packet(n, rng.next()));
        peak = peak.max(buffer.memory_bytes());
    }
    println!(
        "replay buffer: peak {} MiB for a 120 s window (budget {} MiB)",
        peak / (1024 * 1024),
        budget / (1024 * 1024)
    );
    assert!(
        peak <= budget,
        "peak {} bytes exceeds {} byte budget",
        peak,
        budget
    );
    // And the window is actually full — we didn't pass the test by
    // buffering less than 2 minutes.
    assert!(buffer.duration() >= Duration::from_secs(118));
}

#[test]
fn byte_cap_protects_against_keyframeless_streams() {
    // Encoder misbehaves: one keyframe, then endless giant deltas. The hard
    // byte cap must clear rather than grow unboundedly.
    let mut buffer = ReplayBuffer::new(BufferConfig {
        max_duration: Duration::from_secs(3600),
        max_bytes: 10 * 1024 * 1024,
    });
    buffer.push(packet(0, 0)); // keyframe
    for n in 1..2000u64 {
        buffer.push(EncodedPacket {
            data: vec![0u8; 100_000],
            pts: pts_of(n),
            dts: pts_of(n),
            is_keyframe: false,
        });
        assert!(
            buffer.memory_bytes() <= 10 * 1024 * 1024 + 100_000,
            "byte cap breached at frame {n}"
        );
    }
}
