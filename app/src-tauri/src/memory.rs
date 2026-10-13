//! What the replay buffer costs in RAM, and what the app does to keep it small.
//!
//! The buffer holds the last stretch of *encoded* video, so its size is the
//! bitrate times the length: 12 Mb/s × 120 s ≈ 180 MB. Two things shrink it:
//!
//! * **The length follows the clips.** The buffer only has to hold what the
//!   longest save shortcut asks for (plus a margin for the keyframe the clip
//!   starts on), not a fixed two minutes: with the usual 30 s clip it is about
//!   34 s, ~50 MB. It is just an eviction window, so it changes live, with no
//!   restart (a longer one fills up as time goes by).
//! * **Low RAM mode** halves the bitrate and the longest buffer there can be.
//!   The bitrate is fixed when the capture starts, so switching the mode
//!   restarts the video capture and empties the buffer (the UI says so first).

use std::time::Duration;

use clipforge_core::buffer::BufferConfig;
use serde::Serialize;

/// Video kept beyond the longest clip. A clip starts on the keyframe before its
/// first second (one every 2 s) and the buffer drops whole groups of pictures
/// from the front, so two of them make sure a clip is never a hair short.
pub const MARGIN_SECONDS: u64 = 4;

/// The buffer is never shorter than this, whatever the clips (the shortest is 5 s).
pub const MIN_BUFFER_SECONDS: u64 = 10;

/// How long the buffer is, how many bytes it may hold at most, and how
/// finely the video is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPlan {
    pub buffer_seconds: u64,
    /// Hard cap on the buffered video, a safety net if the encoder misbehaves.
    pub buffer_max_bytes: usize,
    /// Average video bitrate, bits per second.
    pub bitrate: u32,
}

/// The usual, as a ceiling: up to two minutes at 12 Mb/s (the engine's own
/// defaults). [`MemoryPlan::sized_for`] shortens it to what the clips need.
pub const NORMAL: MemoryPlan = MemoryPlan {
    buffer_seconds: 120,
    buffer_max_bytes: 256 * 1024 * 1024,
    bitrate: 12_000_000,
};

/// Low RAM, as a ceiling: up to one minute at 6 Mb/s. A little softer picture
/// in fast scenes.
pub const LOW_RAM: MemoryPlan = MemoryPlan {
    buffer_seconds: 60,
    buffer_max_bytes: 96 * 1024 * 1024,
    bitrate: 6_000_000,
};

/// The plan for a mode, with the buffer as long as the longest clip needs.
pub fn plan(low_ram: bool, longest_clip_seconds: u64) -> MemoryPlan {
    let ceiling = if low_ram { LOW_RAM } else { NORMAL };
    ceiling.sized_for(longest_clip_seconds)
}

/// What the settings screen shows about one mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanInfo {
    /// What the buffer holds now, for the longest clip in Settings.
    pub buffer_seconds: u64,
    /// The most this mode could ever hold: no clip is longer.
    pub max_buffer_seconds: u64,
    pub bitrate: u32,
    /// About how much a full buffer weighs, in bytes.
    pub estimated_bytes: u64,
}

/// Both modes side by side, for the settings screen to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plans {
    pub normal: PlanInfo,
    pub low_ram: PlanInfo,
}

pub fn plans(longest_clip_seconds: u64) -> Plans {
    let info = |ceiling: MemoryPlan| {
        let plan = ceiling.sized_for(longest_clip_seconds);
        PlanInfo {
            buffer_seconds: plan.buffer_seconds,
            max_buffer_seconds: ceiling.buffer_seconds,
            bitrate: plan.bitrate,
            estimated_bytes: plan.estimated_buffer_bytes(),
        }
    };
    Plans {
        normal: info(NORMAL),
        low_ram: info(LOW_RAM),
    }
}

impl MemoryPlan {
    /// This plan with a buffer just long enough for a clip of `longest_clip_seconds`
    /// (and its margin), never longer than the plan's own length. The byte cap
    /// shrinks with it, but stays at twice what the buffer should weigh.
    pub fn sized_for(self, longest_clip_seconds: u64) -> MemoryPlan {
        let wanted = longest_clip_seconds
            .saturating_add(MARGIN_SECONDS)
            .max(MIN_BUFFER_SECONDS);
        let seconds = wanted.min(self.buffer_seconds);
        let sized = MemoryPlan {
            buffer_seconds: seconds,
            ..self
        };
        let proportional = self.buffer_max_bytes as u64 * seconds / self.buffer_seconds;
        let cap = proportional
            .max(sized.estimated_buffer_bytes() * 2)
            .min(self.buffer_max_bytes as u64);
        MemoryPlan {
            buffer_max_bytes: cap as usize,
            ..sized
        }
    }

    pub fn buffer_config(&self) -> BufferConfig {
        BufferConfig {
            max_duration: Duration::from_secs(self.buffer_seconds),
            max_bytes: self.buffer_max_bytes,
        }
    }

    /// About how much a full buffer weighs, in bytes (bitrate × length).
    pub fn estimated_buffer_bytes(&self) -> u64 {
        u64::from(self.bitrate) / 8 * self.buffer_seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipforge_core::encode::EncoderConfig;

    /// A clip as long as the buffer's ceiling lets: the plan keeps its full length.
    const LONGEST: u64 = 600;

    #[test]
    fn the_normal_ceiling_is_what_the_engine_always_did() {
        let buffer = BufferConfig::default();
        assert_eq!(NORMAL.buffer_config().max_duration, buffer.max_duration);
        assert_eq!(NORMAL.buffer_config().max_bytes, buffer.max_bytes);
        assert_eq!(NORMAL.bitrate, EncoderConfig::default().bitrate);
    }

    #[test]
    fn low_ram_is_smaller_on_every_count() {
        let (low, normal) = (plan(true, LONGEST), plan(false, LONGEST));
        assert!(low.buffer_seconds < normal.buffer_seconds);
        assert!(low.buffer_max_bytes < normal.buffer_max_bytes);
        assert!(low.bitrate < normal.bitrate);
    }

    #[test]
    fn low_ram_needs_about_a_quarter_of_the_memory_when_clips_are_long() {
        let normal = NORMAL.estimated_buffer_bytes();
        let low = LOW_RAM.estimated_buffer_bytes();
        assert_eq!(normal, 180_000_000, "12 Mb/s for two minutes");
        assert_eq!(low, 45_000_000, "6 Mb/s for one minute");
        assert_eq!(normal / low, 4);
    }

    #[test]
    fn the_buffer_holds_the_longest_clip_and_a_margin_not_a_fixed_two_minutes() {
        // The usual 30 s clip: 34 s instead of 120 s, 70 % less video kept.
        let usual = plan(false, 30);
        assert_eq!(usual.buffer_seconds, 30 + MARGIN_SECONDS);
        assert_eq!(usual.estimated_buffer_bytes(), 51_000_000);
        assert!(usual.estimated_buffer_bytes() * 3 < NORMAL.estimated_buffer_bytes());
        // Low RAM on top of that: half the bitrate for the same 34 s.
        assert_eq!(plan(true, 30).estimated_buffer_bytes(), 25_500_000);
    }

    #[test]
    fn a_longer_clip_needs_a_longer_buffer_up_to_the_ceiling() {
        let seconds = |clip| plan(false, clip).buffer_seconds;
        assert_eq!(seconds(60), 64);
        assert_eq!(seconds(115), 119);
        assert_eq!(seconds(116), 120, "the ceiling is reached exactly");
        assert_eq!(seconds(120), 120, "and a clip can't ask for more than that");
        assert_eq!(
            seconds(u64::MAX),
            120,
            "whatever the number, nothing overflows"
        );
        assert_eq!(
            plan(true, 120).buffer_seconds,
            60,
            "low RAM stops at a minute"
        );
    }

    #[test]
    fn the_buffer_is_never_shorter_than_the_minimum() {
        assert_eq!(plan(false, 0).buffer_seconds, MIN_BUFFER_SECONDS);
        assert_eq!(
            plan(false, 5).buffer_seconds,
            MIN_BUFFER_SECONDS.max(5 + MARGIN_SECONDS)
        );
        assert!(plan(true, 1).buffer_seconds >= MIN_BUFFER_SECONDS);
    }

    #[test]
    fn a_clip_always_fits_in_the_buffer_made_for_it() {
        // The buffer drops whole 2 s groups of pictures until it is under its
        // length, so it ends up at least a group shorter: still longer than the clip.
        let encoder = EncoderConfig::default();
        let gop = u64::from(encoder.keyframe_interval / encoder.fps);
        assert!(
            MARGIN_SECONDS >= 2 * gop,
            "the margin is two groups of pictures"
        );
        for clip in [5, 15, 30, 60, 100, 116] {
            let kept = plan(false, clip).buffer_seconds;
            assert!(
                kept.saturating_sub(gop) >= clip,
                "{clip} s clip, {kept} s buffer"
            );
        }
    }

    #[test]
    fn a_full_buffer_always_fits_under_its_own_cap() {
        for ceiling in [NORMAL, LOW_RAM] {
            for clip in [0, 5, 30, 60, 100, 120, 10_000] {
                let plan = ceiling.sized_for(clip);
                // a quarter of headroom for bitrate swings, so the length (not the cap)
                // is what normally limits the buffer
                assert!(
                    plan.estimated_buffer_bytes() * 5 / 4 < plan.buffer_max_bytes as u64,
                    "{plan:?}: the cap should be slack, not the thing that evicts"
                );
                assert!(plan.buffer_max_bytes <= ceiling.buffer_max_bytes);
            }
        }
    }

    #[test]
    fn the_cap_is_still_a_safety_net_for_a_short_buffer() {
        // A 34 s buffer must not be allowed 256 MB if the encoder misbehaves.
        assert!(plan(false, 30).buffer_max_bytes < NORMAL.buffer_max_bytes / 2);
        assert!(plan(true, 30).buffer_max_bytes < LOW_RAM.buffer_max_bytes);
    }

    #[test]
    fn the_flag_picks_the_ceiling() {
        assert_eq!(plan(false, LONGEST), NORMAL);
        assert_eq!(plan(true, LONGEST), LOW_RAM);
    }

    #[test]
    fn low_ram_is_a_shorter_clip_limit_than_the_settings_allow() {
        // The longest clip the buffer can give. Settings allow asking for more
        // (up to the normal buffer); the clip is then as long as the buffer is.
        assert!(plan(true, LONGEST).buffer_seconds < crate::settings::MAX_CLIP_SECONDS);
    }

    #[test]
    fn the_settings_screen_gets_both_modes_in_camel_case() {
        let json = serde_json::to_value(plans(30)).unwrap();
        assert_eq!(json["normal"]["bufferSeconds"], 34);
        assert_eq!(json["normal"]["maxBufferSeconds"], 120);
        assert_eq!(json["normal"]["estimatedBytes"], 51_000_000);
        assert_eq!(json["lowRam"]["bufferSeconds"], 34);
        assert_eq!(json["lowRam"]["maxBufferSeconds"], 60);
        assert_eq!(json["lowRam"]["bitrate"], 6_000_000);
        assert_eq!(json["lowRam"]["estimatedBytes"], 25_500_000);

        let long = serde_json::to_value(plans(120)).unwrap();
        assert_eq!(long["normal"]["bufferSeconds"], 120);
        assert_eq!(long["normal"]["estimatedBytes"], 180_000_000);
        assert_eq!(long["lowRam"]["bufferSeconds"], 60);
        assert_eq!(long["lowRam"]["estimatedBytes"], 45_000_000);
    }
}
