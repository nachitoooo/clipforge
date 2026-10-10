//! Audio: PCM chunks from the OS, AAC packets for the replay buffer.
//!
//! Mirrors the video side. A capture backend hands out [`AudioChunk`]s (raw
//! interleaved `f32` PCM) on the same clock as the video frames; an
//! [`AudioEncoder`] turns them into AAC [`AudioPacket`]s; the replay buffer
//! keeps the last N seconds of packets next to the video, and saving a clip
//! muxes both into one .mp4 — still a remux, never a re-encode.
//!
//! Between the capture backend and the encoder sits a [`ContinuityGuard`]:
//! the mp4 muxer places audio samples back to back (it cannot express gaps),
//! so any hole in the capture stream must be filled with silence *before*
//! encoding, or everything after it would drift against the picture.

pub mod level;
pub mod mixer;
pub mod mock;
pub mod resample;
pub mod test_tone;

#[cfg(target_os = "macos")]
pub mod aac_audiotoolbox;
#[cfg(target_os = "windows")]
pub mod aac_mediafoundation;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod mic;
#[cfg(target_os = "windows")]
pub mod wasapi;

use std::time::Duration;

use thiserror::Error;

/// Samples (per channel) in one AAC-LC access unit.
pub const AAC_FRAME_SAMPLES: u32 = 1024;

/// Which track of the clip an audio packet belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioLaneId {
    /// Game/system audio — or game and voice mixed, see [`MicMode::Mixed`].
    Main,
    /// The microphone on its own track ([`MicMode::Separate`]).
    Mic,
}

/// How the microphone ends up in a saved clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MicMode {
    /// Voice and game are mixed into the main track before encoding: plays
    /// everywhere, can't be muted afterwards.
    #[default]
    Mixed,
    /// Voice is encoded on its own track: can be muted afterwards, but many
    /// players only play the first track.
    Separate,
}

/// A run of raw audio from a capture backend.
#[derive(Clone)]
pub struct AudioChunk {
    /// Interleaved 32-bit float samples in `[-1.0, 1.0]`
    /// (`frames * channels` values).
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    /// Presentation time of the first sample, relative to the same capture
    /// epoch as the video frames.
    pub timestamp: Duration,
}

impl AudioChunk {
    /// Samples per channel.
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    pub fn duration(&self) -> Duration {
        frames_to_duration(self.frames() as u64, self.sample_rate)
    }
}

impl std::fmt::Debug for AudioChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioChunk")
            .field("frames", &self.frames())
            .field("channels", &self.channels)
            .field("sample_rate", &self.sample_rate)
            .field("timestamp", &self.timestamp)
            .finish()
    }
}

/// One encoded audio access unit (an AAC frame).
#[derive(Clone)]
pub struct AudioPacket {
    pub data: Vec<u8>,
    /// Capture time of the first input sample this packet encodes. A decoder
    /// outputs that audio [`AudioEncoder::priming_frames`] samples later;
    /// the muxer compensates with an edit list.
    pub pts: Duration,
    pub duration: Duration,
}

impl std::fmt::Debug for AudioPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioPacket")
            .field("pts", &self.pts)
            .field("duration", &self.duration)
            .field("data_len", &self.data.len())
            .finish()
    }
}

/// What every stage of the audio pipeline agrees on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// AAC bitrate in bits/s. 128 kb/s stereo is transparent for game audio
    /// and adds ~2 MB to a 120 s buffer.
    pub bitrate: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            bitrate: 128_000,
        }
    }
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("audio format not supported: {0}")]
    Unsupported(String),
    #[error("chunk does not match encoder config: {0}")]
    BadChunk(String),
    #[error("audio backend error: {0}")]
    Backend(String),
}

/// Raw audio in, AAC packets out.
///
/// Chunks must be contiguous (see [`ContinuityGuard`]): packet `k`'s `pts` is
/// derived from the first chunk's timestamp plus `k * 1024` samples, so the
/// output is gap-free by construction.
pub trait AudioEncoder: Send {
    fn encode(&mut self, chunk: &AudioChunk) -> Result<Vec<AudioPacket>, AudioError>;

    /// Flush everything still buffered inside the encoder. The encoder is
    /// finished afterwards.
    fn finish(&mut self) -> Result<Vec<AudioPacket>, AudioError>;

    /// The next chunk is *not* contiguous with the previous one (a long gap,
    /// a device change). Drop internal state and restart the timeline at the
    /// next chunk's timestamp.
    fn resync(&mut self);

    /// Encoder delay: how many samples a decoder outputs before the first
    /// real input sample (2112 for Apple's AAC-LC). The muxer turns this into
    /// an edit list so players start at the right instant.
    fn priming_frames(&self) -> u32 {
        0
    }

    fn config(&self) -> &AudioConfig;
}

pub(crate) fn frames_to_duration(frames: u64, sample_rate: u32) -> Duration {
    Duration::from_nanos(frames * 1_000_000_000 / u64::from(sample_rate.max(1)))
}

fn duration_to_frames(d: Duration, sample_rate: u32) -> u64 {
    (d.as_nanos() * u128::from(sample_rate) / 1_000_000_000) as u64
}

/// Timestamps may wobble by this much without anything being repaired.
const GAP_TOLERANCE: Duration = Duration::from_millis(25);
/// Gaps up to this long are bridged with silence; longer ones restart the
/// audio timeline (and forget what was buffered).
const MAX_BRIDGED_GAP: Duration = Duration::from_secs(5);

/// What [`ContinuityGuard::process`] decided about a chunk.
#[derive(Debug)]
pub struct Guarded {
    pub chunk: AudioChunk,
    /// The stream jumped: clear buffered audio and `resync` the encoder
    /// before encoding `chunk`.
    pub restart: bool,
}

/// Keeps the audio stream gap-free and non-overlapping.
///
/// It tracks where the next sample *should* start. A chunk that starts later
/// gets silence inserted in front; one that starts earlier loses its
/// overlapping head; one that is far away restarts the timeline.
pub struct ContinuityGuard {
    expected: Option<Duration>,
    sample_rate: u32,
    channels: u16,
}

impl ContinuityGuard {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            expected: None,
            sample_rate,
            channels,
        }
    }

    /// `None` means the chunk is entirely stale and must be dropped.
    pub fn process(&mut self, mut chunk: AudioChunk) -> Option<Guarded> {
        let Some(expected) = self.expected else {
            self.expected = Some(chunk.timestamp + chunk.duration());
            return Some(Guarded {
                chunk,
                restart: false,
            });
        };

        let mut restart = false;
        if chunk.timestamp >= expected {
            let gap = chunk.timestamp - expected;
            if gap > MAX_BRIDGED_GAP {
                restart = true;
            } else if gap > GAP_TOLERANCE {
                let silent_frames = duration_to_frames(gap, self.sample_rate) as usize;
                let mut samples = vec![0.0f32; silent_frames * usize::from(self.channels)];
                samples.append(&mut chunk.samples);
                chunk.samples = samples;
                chunk.timestamp = expected;
            } else {
                // Within tolerance: trust sample counting over the clock.
                chunk.timestamp = expected;
            }
        } else {
            let overlap = expected - chunk.timestamp;
            if overlap > MAX_BRIDGED_GAP {
                restart = true;
            } else if overlap > GAP_TOLERANCE {
                let skip = duration_to_frames(overlap, self.sample_rate) as usize;
                if skip >= chunk.frames() {
                    return None; // entirely stale
                }
                chunk.samples.drain(..skip * usize::from(self.channels));
                chunk.timestamp = expected;
            } else {
                chunk.timestamp = expected;
            }
        }

        self.expected = Some(chunk.timestamp + chunk.duration());
        Some(Guarded { chunk, restart })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn chunk(start_ms: u64, frames: usize) -> AudioChunk {
        AudioChunk {
            samples: vec![0.5; frames * 2],
            channels: 2,
            sample_rate: RATE,
            timestamp: Duration::from_millis(start_ms),
        }
    }

    fn guard() -> ContinuityGuard {
        ContinuityGuard::new(RATE, 2)
    }

    #[test]
    fn chunk_frames_and_duration() {
        let c = chunk(0, 480);
        assert_eq!(c.frames(), 480);
        assert_eq!(c.duration(), Duration::from_millis(10));
    }

    #[test]
    fn first_chunk_passes_through() {
        let out = guard().process(chunk(7, 480)).unwrap();
        assert!(!out.restart);
        assert_eq!(out.chunk.timestamp, Duration::from_millis(7));
        assert_eq!(out.chunk.frames(), 480);
    }

    #[test]
    fn contiguous_chunks_are_untouched() {
        let mut g = guard();
        g.process(chunk(0, 480)).unwrap();
        let out = g.process(chunk(10, 480)).unwrap();
        assert!(!out.restart);
        assert_eq!(out.chunk.frames(), 480);
        assert_eq!(out.chunk.timestamp, Duration::from_millis(10));
    }

    #[test]
    fn jitter_within_tolerance_is_snapped_to_the_sample_grid() {
        let mut g = guard();
        g.process(chunk(0, 480)).unwrap();
        // Arrives 5 ms "late" and then 5 ms "early": both snap to expected.
        let late = g.process(chunk(15, 480)).unwrap();
        assert_eq!(late.chunk.timestamp, Duration::from_millis(10));
        assert_eq!(late.chunk.frames(), 480);
        let early = g.process(chunk(15, 480)).unwrap();
        assert_eq!(early.chunk.timestamp, Duration::from_millis(20));
        assert_eq!(early.chunk.frames(), 480);
    }

    #[test]
    fn gap_is_filled_with_silence() {
        let mut g = guard();
        g.process(chunk(0, 480)).unwrap(); // ends at 10 ms
        let out = g.process(chunk(110, 480)).unwrap(); // 100 ms hole
        assert!(!out.restart);
        assert_eq!(out.chunk.timestamp, Duration::from_millis(10));
        assert_eq!(out.chunk.frames(), 4800 + 480);
        assert!(out.chunk.samples[..4800 * 2].iter().all(|&s| s == 0.0));
        assert!(out.chunk.samples[4800 * 2..].iter().all(|&s| s == 0.5));
        // The stream is contiguous again afterwards.
        let next = g.process(chunk(120, 480)).unwrap();
        assert_eq!(next.chunk.timestamp, Duration::from_millis(120));
    }

    #[test]
    fn overlap_loses_its_head() {
        let mut g = guard();
        g.process(chunk(0, 4800)).unwrap(); // 0..100 ms
        let mut c = chunk(0, 9600); // 0..200 ms: its first 100 ms were already seen
        c.samples[..4800 * 2].fill(0.1);
        let out = g.process(c).unwrap();
        assert!(!out.restart);
        assert_eq!(out.chunk.timestamp, Duration::from_millis(100));
        assert_eq!(out.chunk.frames(), 4800);
        assert!(out.chunk.samples.iter().all(|&s| s == 0.5));
    }

    #[test]
    fn fully_stale_chunk_is_dropped() {
        let mut g = guard();
        g.process(chunk(0, 4800)).unwrap(); // ends at 100 ms
        assert!(g.process(chunk(10, 480)).is_none()); // 10..20 ms
    }

    #[test]
    fn huge_forward_jump_restarts_the_timeline() {
        let mut g = guard();
        g.process(chunk(0, 480)).unwrap();
        let out = g.process(chunk(60_000, 480)).unwrap();
        assert!(out.restart);
        assert_eq!(out.chunk.timestamp, Duration::from_secs(60));
        assert_eq!(out.chunk.frames(), 480, "no silence for a restart");
        // And continues normally from the new position.
        let next = g.process(chunk(60_010, 480)).unwrap();
        assert!(!next.restart);
    }

    #[test]
    fn huge_backward_jump_restarts_the_timeline() {
        let mut g = guard();
        g.process(chunk(60_000, 480)).unwrap();
        let out = g.process(chunk(0, 480)).unwrap();
        assert!(out.restart);
        assert_eq!(out.chunk.timestamp, Duration::ZERO);
    }

    #[test]
    fn long_run_with_clock_drift_stays_gap_free() {
        // The device clock runs 0.1% fast against the capture clock: every
        // 10 ms chunk is stamped 10.01 ms after the previous one. Whatever the
        // guard does, the emitted stream must remain exactly contiguous.
        let mut g = guard();
        let mut expected_next = None;
        for i in 0..2000u64 {
            let ts_us = i * 10_010;
            let c = AudioChunk {
                samples: vec![0.3; 480 * 2],
                channels: 2,
                sample_rate: RATE,
                timestamp: Duration::from_micros(ts_us),
            };
            let Some(out) = g.process(c) else { continue };
            assert!(!out.restart, "drift must never restart the timeline");
            if let Some(next) = expected_next {
                assert_eq!(out.chunk.timestamp, next, "chunk {i} not contiguous");
            }
            expected_next = Some(out.chunk.timestamp + out.chunk.duration());
        }
    }
}
