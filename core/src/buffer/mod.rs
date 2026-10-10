//! Encoded-frame replay buffer.
//!
//! [`ReplayBuffer`] keeps the last `max_duration` of *encoded* packets in
//! memory (RAM stays bounded: ~180 MB for 120 s of 1080p60 at 12 Mb/s) and
//! maintains one invariant at all times: **the buffer starts at a keyframe**.
//! Eviction removes whole GOPs from the front, so any extraction can be
//! muxed directly — saving a clip is a container write, never a re-encode.

use std::collections::VecDeque;
use std::time::Duration;

use crate::audio::{AudioLaneId, AudioPacket};
use crate::encode::EncodedPacket;

/// Audio is kept slightly longer than `max_duration` so it always covers the
/// video even when the oldest GOP overshoots the window.
const AUDIO_WINDOW_MARGIN: Duration = Duration::from_secs(15);

/// Audio is handed over from this much *before* the first video packet, so
/// the muxer has packets to spare for the decoder's delay and warm-up...
const AUDIO_PREROLL: Duration = Duration::from_millis(100);
/// ...and this much past the last one, because a decoder emits the audio
/// that long after it was encoded.
const AUDIO_TAIL: Duration = Duration::from_millis(100);

/// A clip's worth of packets cut from the buffer.
#[derive(Debug, Clone, Default)]
pub struct ClipSlice {
    /// Starts at a keyframe.
    pub video: Vec<EncodedPacket>,
    /// AAC packets of the main lane (game audio, or game + voice mixed)
    /// around the video's time span, with a little margin on both sides (see
    /// `AUDIO_PREROLL`); empty without audio.
    pub audio: Vec<AudioPacket>,
    /// The microphone's own lane, same margins; empty unless the microphone
    /// is recorded as a separate track.
    pub mic: Vec<AudioPacket>,
}

/// One lane of AAC packets: independent of GOP structure, since every AAC
/// frame decodes on its own.
#[derive(Debug, Default)]
struct AudioLane {
    packets: VecDeque<AudioPacket>,
    bytes: usize,
}

impl AudioLane {
    /// Store a packet (monotonic pts expected). A backwards pts is a
    /// discontinuity: the history is dropped and the packet starts a new one.
    /// Keeps at most `window` of audio.
    fn push(&mut self, packet: AudioPacket, window: Duration) {
        if self
            .packets
            .back()
            .is_some_and(|last| packet.pts <= last.pts)
        {
            self.clear();
        }
        self.bytes += packet.data.len();
        self.packets.push_back(packet);
        while let (Some(first), Some(last)) = (self.packets.front(), self.packets.back()) {
            if last.pts - first.pts <= window {
                break;
            }
            if let Some(dropped) = self.packets.pop_front() {
                self.bytes -= dropped.data.len();
            }
        }
    }

    fn clear(&mut self) {
        self.packets.clear();
        self.bytes = 0;
    }

    /// Packets around the video span `[first, last]`, with pre-roll and tail.
    fn around(&self, first: Duration, last: Duration) -> Vec<AudioPacket> {
        self.packets
            .iter()
            .filter(|p| p.pts + AUDIO_PREROLL >= first && p.pts <= last + AUDIO_TAIL)
            .cloned()
            .collect()
    }
}

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
    /// AAC lanes for the same window.
    audio: AudioLane,
    mic: AudioLane,
    config: BufferConfig,
}

impl ReplayBuffer {
    pub fn new(config: BufferConfig) -> Self {
        Self {
            packets: VecDeque::new(),
            bytes: 0,
            audio: AudioLane::default(),
            mic: AudioLane::default(),
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

    /// Like [`Self::extract_last`], plus the audio packets of every lane that
    /// fall around the extracted video's time span.
    pub fn extract_last_with_audio(&self, duration: Duration) -> ClipSlice {
        let video = self.extract_last(duration);
        let (audio, mic) = match (video.first(), video.last()) {
            (Some(first), Some(last)) => (
                self.audio.around(first.pts, last.pts),
                self.mic.around(first.pts, last.pts),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        ClipSlice { video, audio, mic }
    }

    fn audio_window(&self) -> Duration {
        self.config.max_duration + AUDIO_WINDOW_MARGIN
    }

    fn lane_mut(&mut self, lane: AudioLaneId) -> &mut AudioLane {
        match lane {
            AudioLaneId::Main => &mut self.audio,
            AudioLaneId::Mic => &mut self.mic,
        }
    }

    fn lane(&self, lane: AudioLaneId) -> &AudioLane {
        match lane {
            AudioLaneId::Main => &self.audio,
            AudioLaneId::Mic => &self.mic,
        }
    }

    /// Add one AAC packet to the main lane.
    pub fn push_audio(&mut self, packet: AudioPacket) {
        self.push_audio_lane(AudioLaneId::Main, packet);
    }

    pub fn push_audio_lane(&mut self, lane: AudioLaneId, packet: AudioPacket) {
        let window = self.audio_window();
        self.lane_mut(lane).push(packet, window);
    }

    /// Forget the main lane's audio (the video is untouched).
    pub fn clear_audio(&mut self) {
        self.audio.clear();
    }

    pub fn clear_audio_lane(&mut self, lane: AudioLaneId) {
        self.lane_mut(lane).clear();
    }

    pub fn audio_packet_count(&self) -> usize {
        self.audio.packets.len()
    }

    pub fn audio_lane_packet_count(&self, lane: AudioLaneId) -> usize {
        self.lane(lane).packets.len()
    }

    /// Main-lane audio payload bytes currently held (small next to the video).
    pub fn audio_memory_bytes(&self) -> usize {
        self.audio.bytes
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

    /// Drop everything, video and audio.
    pub fn clear(&mut self) {
        self.packets.clear();
        self.bytes = 0;
        self.audio.clear();
        self.mic.clear();
    }
}

#[cfg(test)]
mod audio_tests {
    use super::*;

    const AAC_MS: u64 = 21; // ≈ 1024 samples at 48 kHz

    fn video(n: u64, gop: u64) -> EncodedPacket {
        let pts = Duration::from_millis(n * 1000 / 60);
        EncodedPacket {
            data: vec![0; 100],
            pts,
            dts: pts,
            is_keyframe: n.is_multiple_of(gop),
        }
    }

    fn audio(k: u64) -> AudioPacket {
        AudioPacket {
            data: vec![0; 10],
            pts: Duration::from_millis(k * AAC_MS),
            duration: Duration::from_millis(AAC_MS),
        }
    }

    fn filled(seconds: u64, gop: u64) -> ReplayBuffer {
        let mut buffer = ReplayBuffer::new(BufferConfig::default());
        for n in 0..seconds * 60 {
            buffer.push(video(n, gop));
        }
        for k in 0..seconds * 1000 / AAC_MS {
            buffer.push_audio(audio(k));
        }
        buffer
    }

    #[test]
    fn extraction_pairs_audio_with_the_video_span() {
        let buffer = filled(10, 60);
        let slice = buffer.extract_last_with_audio(Duration::from_secs(3));
        let (v_first, v_last) = (slice.video.first().unwrap(), slice.video.last().unwrap());
        assert!(v_first.is_keyframe);
        assert!(!slice.audio.is_empty());
        assert!(slice.audio.first().unwrap().pts + AUDIO_PREROLL >= v_first.pts);
        assert!(slice.audio.last().unwrap().pts <= v_last.pts + AUDIO_TAIL);
        // Roughly 3-4 s of audio (3 s requested, up to one 1 s GOP extra).
        let total: Duration = slice.audio.iter().map(|p| p.duration).sum();
        assert!(
            total >= Duration::from_millis(2900) && total <= Duration::from_millis(4300),
            "{total:?}"
        );
    }

    #[test]
    fn extraction_without_audio_matches_video_only() {
        let mut buffer = ReplayBuffer::new(BufferConfig::default());
        for n in 0..300 {
            buffer.push(video(n, 60));
        }
        let slice = buffer.extract_last_with_audio(Duration::from_secs(2));
        assert_eq!(
            slice.video.len(),
            buffer.extract_last(Duration::from_secs(2)).len()
        );
        assert!(slice.audio.is_empty());
    }

    #[test]
    fn empty_buffer_yields_an_empty_slice() {
        let mut buffer = ReplayBuffer::new(BufferConfig::default());
        buffer.push_audio(audio(0));
        let slice = buffer.extract_last_with_audio(Duration::from_secs(5));
        assert!(slice.video.is_empty() && slice.audio.is_empty());
    }

    #[test]
    fn audio_window_is_bounded() {
        let mut buffer = ReplayBuffer::new(BufferConfig {
            max_duration: Duration::from_secs(10),
            ..Default::default()
        });
        // 120 s of audio into a 10 s (+15 s margin) window.
        for k in 0..120_000 / AAC_MS {
            buffer.push_audio(audio(k));
        }
        let span =
            buffer.audio.packets.back().unwrap().pts - buffer.audio.packets.front().unwrap().pts;
        assert!(span <= Duration::from_secs(25), "span {span:?}");
        assert!(span >= Duration::from_secs(24), "span {span:?}");
        assert_eq!(
            buffer.audio_memory_bytes(),
            buffer.audio_packet_count() * 10
        );
    }

    #[test]
    fn backwards_audio_pts_clears_only_the_audio() {
        let mut buffer = filled(5, 60);
        let video_before = buffer.packet_count();
        buffer.push_audio(audio(0)); // pts goes back to 0
        assert_eq!(buffer.audio_packet_count(), 1);
        assert_eq!(buffer.packet_count(), video_before);
    }

    #[test]
    fn video_discontinuity_clears_audio_too() {
        let mut buffer = filled(5, 60);
        buffer.push(video(0, 60)); // video pts jumps back: new epoch
        assert_eq!(buffer.audio_packet_count(), 0);
        assert_eq!(buffer.audio_memory_bytes(), 0);
    }

    #[test]
    fn clear_empties_both_streams() {
        let mut buffer = filled(5, 60);
        buffer.clear();
        assert!(buffer.is_empty());
        assert_eq!(buffer.audio_packet_count(), 0);
    }

    #[test]
    fn the_mic_lane_is_independent_and_extracted_alongside() {
        let mut buffer = filled(10, 60);
        for k in 0..10_000 / AAC_MS {
            buffer.push_audio_lane(AudioLaneId::Mic, audio(k));
        }
        assert_eq!(
            buffer.audio_lane_packet_count(AudioLaneId::Mic),
            10_000 / AAC_MS as usize
        );
        let slice = buffer.extract_last_with_audio(Duration::from_secs(3));
        assert!(!slice.audio.is_empty() && !slice.mic.is_empty());
        let (v_first, v_last) = (slice.video.first().unwrap(), slice.video.last().unwrap());
        assert!(slice.mic.first().unwrap().pts + AUDIO_PREROLL >= v_first.pts);
        assert!(slice.mic.last().unwrap().pts <= v_last.pts + AUDIO_TAIL);

        // Clearing one lane leaves the other alone.
        buffer.clear_audio_lane(AudioLaneId::Mic);
        assert_eq!(buffer.audio_lane_packet_count(AudioLaneId::Mic), 0);
        assert!(buffer.audio_packet_count() > 0);
    }

    #[test]
    fn a_backwards_mic_pts_only_resets_the_mic_lane() {
        let mut buffer = filled(5, 60);
        for k in 0..50 {
            buffer.push_audio_lane(AudioLaneId::Mic, audio(k));
        }
        let main_before = buffer.audio_packet_count();
        buffer.push_audio_lane(AudioLaneId::Mic, audio(0));
        assert_eq!(buffer.audio_lane_packet_count(AudioLaneId::Mic), 1);
        assert_eq!(buffer.audio_packet_count(), main_before);
    }

    #[test]
    fn clear_empties_the_mic_lane_too() {
        let mut buffer = filled(5, 60);
        buffer.push_audio_lane(AudioLaneId::Mic, audio(0));
        buffer.clear();
        assert_eq!(buffer.audio_lane_packet_count(AudioLaneId::Mic), 0);
    }
}
