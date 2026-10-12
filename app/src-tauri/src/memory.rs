//! What the replay buffer costs in RAM, and the "low RAM" mode that trades some
//! of it away.
//!
//! The buffer holds the last stretch of *encoded* video, so its size is the
//! bitrate times the length: 12 Mb/s × 120 s ≈ 180 MB. Low RAM mode keeps half
//! the time at half the bitrate, a quarter of the memory.
//!
//! Both numbers are fixed when the capture starts, so changing the mode restarts
//! the video capture and empties the buffer (the UI says so first).

use std::time::Duration;

use clipforge_core::buffer::BufferConfig;
use serde::Serialize;

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

/// The usual: two minutes at 12 Mb/s (the engine's own defaults).
pub const NORMAL: MemoryPlan = MemoryPlan {
    buffer_seconds: 120,
    buffer_max_bytes: 256 * 1024 * 1024,
    bitrate: 12_000_000,
};

/// Low RAM: one minute at 6 Mb/s. A little softer picture in fast scenes.
pub const LOW_RAM: MemoryPlan = MemoryPlan {
    buffer_seconds: 60,
    buffer_max_bytes: 96 * 1024 * 1024,
    bitrate: 6_000_000,
};

pub fn plan(low_ram: bool) -> MemoryPlan {
    if low_ram {
        LOW_RAM
    } else {
        NORMAL
    }
}

/// What the settings screen shows about one mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanInfo {
    pub buffer_seconds: u64,
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

pub fn plans() -> Plans {
    let info = |plan: MemoryPlan| PlanInfo {
        buffer_seconds: plan.buffer_seconds,
        bitrate: plan.bitrate,
        estimated_bytes: plan.estimated_buffer_bytes(),
    };
    Plans {
        normal: info(NORMAL),
        low_ram: info(LOW_RAM),
    }
}

impl MemoryPlan {
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

    #[test]
    fn the_normal_plan_is_what_the_engine_always_did() {
        let buffer = BufferConfig::default();
        assert_eq!(NORMAL.buffer_config().max_duration, buffer.max_duration);
        assert_eq!(NORMAL.buffer_config().max_bytes, buffer.max_bytes);
        assert_eq!(NORMAL.bitrate, EncoderConfig::default().bitrate);
    }

    #[test]
    fn low_ram_is_smaller_on_every_count() {
        let (low, normal) = (plan(true), plan(false));
        assert!(low.buffer_seconds < normal.buffer_seconds);
        assert!(low.buffer_max_bytes < normal.buffer_max_bytes);
        assert!(low.bitrate < normal.bitrate);
    }

    #[test]
    fn low_ram_needs_about_a_quarter_of_the_memory() {
        let normal = NORMAL.estimated_buffer_bytes();
        let low = LOW_RAM.estimated_buffer_bytes();
        assert_eq!(normal, 180_000_000, "12 Mb/s for two minutes");
        assert_eq!(low, 45_000_000, "6 Mb/s for one minute");
        assert_eq!(normal / low, 4);
    }

    #[test]
    fn a_full_buffer_always_fits_under_its_own_cap() {
        for plan in [NORMAL, LOW_RAM] {
            // a quarter of headroom for bitrate swings, so the length (not the cap)
            // is what normally limits the buffer
            assert!(
                plan.estimated_buffer_bytes() * 5 / 4 < plan.buffer_max_bytes as u64,
                "{plan:?}: the cap should be slack, not the thing that evicts"
            );
        }
    }

    #[test]
    fn the_flag_picks_the_plan() {
        assert_eq!(plan(false), NORMAL);
        assert_eq!(plan(true), LOW_RAM);
    }

    #[test]
    fn low_ram_is_a_shorter_clip_limit_than_the_settings_allow() {
        // The longest clip the buffer can give. Settings allow asking for more
        // (up to the normal buffer); the clip is then as long as the buffer is.
        assert!(plan(true).buffer_seconds < crate::settings::MAX_CLIP_SECONDS);
    }

    #[test]
    fn the_settings_screen_gets_both_modes_in_camel_case() {
        let json = serde_json::to_value(plans()).unwrap();
        assert_eq!(json["normal"]["bufferSeconds"], 120);
        assert_eq!(json["normal"]["estimatedBytes"], 180_000_000);
        assert_eq!(json["lowRam"]["bufferSeconds"], 60);
        assert_eq!(json["lowRam"]["bitrate"], 6_000_000);
        assert_eq!(json["lowRam"]["estimatedBytes"], 45_000_000);
    }
}
