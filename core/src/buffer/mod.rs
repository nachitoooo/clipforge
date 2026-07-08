//! Encoded-frame replay buffer.
//!
//! [`ReplayBuffer`] keeps the last `max_duration` of *encoded* packets in
//! memory (RAM stays bounded: ~180 MB for 120 s of 1080p60 at 12 Mb/s) and
//! maintains one invariant at all times: **the buffer starts at a keyframe**.
//! Eviction removes whole GOPs from the front, so any extraction can be
//! muxed directly — saving a clip is a container write, never a re-encode.

use std::collections::VecDeque;
use std::time::Duration;

use crate::encode::EncodedPacket;

#[derive(Debug, Clone)]
pub struct BufferConfig {
    /// Rolling window to keep. Default 120 s.
    pub max_duration: Duration,
    /// Hard safety cap on packet payload bytes; protects RAM if the encoder
    /// misbehaves (e.g. emits no keyframes). Default 256 MB.
    pub max_bytes: usize,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            max_duration: Duration::from_secs(120),
            max_bytes: 256 * 1024 * 1024,
        }
    }
}

/// What [`ReplayBuffer::push`] did with the packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    /// Packet stored.
    Stored,
    /// Dropped: the buffer is empty and only accepts a keyframe as its first
    /// packet (start of a GOP).
    AwaitingKeyframe,
    /// A backwards pts (stream discontinuity, e.g. encoder restart) cleared
    /// the buffer; the packet itself was then stored/dropped per the
    /// keyframe rule.
    ResetOnDiscontinuity(bool),
}

#[derive(Debug, Default)]
pub struct ReplayBuffer {
    packets: VecDeque<EncodedPacket>,
    /// Sum of packet payload sizes (the dominant memory cost).
    bytes: usize,
    config: BufferConfig,
}

impl ReplayBuffer {
    pub fn new(config: BufferConfig) -> Self {
        Self {
            packets: VecDeque::new(),
            bytes: 0,
            config,
        }
    }

    /// Add one encoded packet (decode order, monotonic pts expected).
    pub fn push(&mut self, packet: EncodedPacket) -> PushOutcome {
        // Discontinuity: pts went backwards — drop history, start over.
        if let Some(last) = self.packets.back() {
            if packet.pts <= last.pts {
                self.clear();
                let stored = self.accept_first(packet);
                return PushOutcome::ResetOnDiscontinuity(stored);
            }
        }
        if self.packets.is_empty() {
            return if self.accept_first(packet) {
                PushOutcome::Stored
            } else {
                PushOutcome::AwaitingKeyframe
            };
        }
        self.bytes += packet.data.len();
        self.packets.push_back(packet);
        self.evict();
        PushOutcome::Stored
    }

    fn accept_first(&mut self, packet: EncodedPacket) -> bool {
        if packet.is_keyframe {
            self.bytes = packet.data.len();
            self.packets.push_back(packet);
            true
        } else {
            false
        }
    }

    /// Drop whole GOPs from the front while over the duration or byte limit.
    fn evict(&mut self) {
        loop {
            let over_duration = self.duration() > self.config.max_duration;
            let over_bytes = self.bytes > self.config.max_bytes;
            if !over_duration && !over_bytes {
                return;
            }
            match self.next_keyframe_after_front() {
                Some(next_key) => {
                    for _ in 0..next_key {
                        if let Some(dropped) = self.packets.pop_front() {
                            self.bytes -= dropped.data.len();
                        }
                    }
                }
                // Single GOP left. Over duration: keep it (can't shrink
                // without breaking the keyframe invariant). Over the hard
                // byte cap: drop everything rather than exhaust RAM.
                None => {
                    if over_bytes {
                        self.clear();
                    }
                    return;
                }
            }
        }
    }

    fn next_keyframe_after_front(&self) -> Option<usize> {
        self.packets
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, p)| p.is_keyframe)
            .map(|(i, _)| i)
    }

    /// Packets covering (at least) the last `duration`, starting at a
    /// keyframe. Returns the whole buffer when it is shorter than requested.
    /// The clip may exceed `duration` by up to one GOP (keyframe alignment).
    pub fn extract_last(&self, duration: Duration) -> Vec<EncodedPacket> {
        let Some(last) = self.packets.back() else {
            return Vec::new();
        };
        let target_start = last.pts.saturating_sub(duration);
        // Latest keyframe at or before target_start; front (index 0) is
        // always a keyframe, so `unwrap_or(0)` keeps the invariant.
        let start = self
            .packets
            .iter()
            .enumerate()
            .rev()
            .find(|(_, p)| p.is_keyframe && p.pts <= target_start)
            .map_or(0, |(i, _)| i);
        self.packets.iter().skip(start).cloned().collect()
    }

    /// pts span between the first and last packet.
    pub fn duration(&self) -> Duration {
        match (self.packets.front(), self.packets.back()) {
            (Some(first), Some(last)) => last.pts - first.pts,
            _ => Duration::ZERO,
        }
    }

    /// Payload bytes currently held.
    pub fn memory_bytes(&self) -> usize {
        self.bytes
    }

    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    pub fn clear(&mut self) {
        self.packets.clear();
        self.bytes = 0;
    }
}
