//! Real-time mixing of the game audio with the microphone.
//!
//! In [`MicMode::Mixed`](super::MicMode) the voice is added to the game audio
//! *before* AAC encoding, so a saved clip stays a plain remux (mixing at save
//! time would mean decoding and re-encoding) and plays in every player.
//!
//! [`AudioMixer`] is pure and deterministic. Both inputs arrive as timestamped
//! chunks on the shared video clock; each is kept as a dense timeline (holes
//! become silence) and mixed in fixed 10 ms blocks by *timestamp*, so the two
//! stay aligned however irregularly the chunks arrive. A block is released
//! once the newest data from either source is [`MIX_LATENCY`] past it: by then
//! a live source has delivered its share, and one that hasn't is treated as
//! silent for that block — a stalled microphone (or a Windows loopback that
//! goes quiet while nothing plays) never freezes the other.
//!
//! [`spawn_mix_stage`] wraps the mixer in a thread that sits between the
//! system-audio capture and the audio encoder, and lets the microphone be
//! attached and detached while it runs.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::sync::mpsc;

use super::{frames_to_duration, AudioChunk};

/// Mixing block: 10 ms at 48 kHz.
pub const BLOCK_FRAMES: usize = 480;
/// How far behind the newest data a block is released. Must exceed the
/// microphone's delivery lag (capture-to-callback ≈ 36 ms measured on macOS)
/// plus scheduling jitter.
pub const MIX_LATENCY: Duration = Duration::from_millis(80);
/// Timestamp wobble that is snapped away rather than treated as a gap.
const TOLERANCE: Duration = Duration::from_millis(25);
/// Holes longer than this restart the timeline instead of being padded.
const MAX_BRIDGED_GAP: Duration = Duration::from_secs(5);
/// A queue never holds more than this much unmixed audio.
const MAX_QUEUED: Duration = Duration::from_secs(10);
/// Where the soft limiter starts bending the signal.
const LIMITER_KNEE: f32 = 0.9;

fn frames_of(d: Duration, rate: u32) -> i64 {
    (d.as_nanos() * u128::from(rate) / 1_000_000_000) as i64
}

/// Sum without ever exceeding full scale: identical below the knee, then a
/// smooth curve that approaches ±1 and never goes past it.
pub fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= LIMITER_KNEE {
        x
    } else {
        let room = 1.0 - LIMITER_KNEE;
        x.signum() * (LIMITER_KNEE + room * ((a - LIMITER_KNEE) / room).tanh())
    }
}

/// One input as a gap-free timeline of frames.
struct DenseQueue {
    channels: usize,
    rate: u32,
    /// Frame index (on the shared clock) of `samples[0]`.
    start: i64,
    samples: VecDeque<f32>,
}

impl DenseQueue {
    fn new(channels: usize, rate: u32) -> Self {
        Self {
            channels,
            rate,
            start: 0,
            samples: VecDeque::new(),
        }
    }

    fn frames(&self) -> i64 {
        (self.samples.len() / self.channels) as i64
    }

    fn end(&self) -> i64 {
        self.start + self.frames()
    }

    fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    fn clear(&mut self) {
        self.samples.clear();
    }

    /// Append a chunk, padding holes with silence and trimming overlap.
    /// Anything before `floor` (already mixed) is discarded.
    fn push(&mut self, chunk: &AudioChunk, floor: Option<i64>) {
        let channels = self.channels;
        if usize::from(chunk.channels) != channels || chunk.sample_rate != self.rate {
            return; // a source that changed format mid-stream: skip, don't garble
        }
        let mut start = frames_of(chunk.timestamp, self.rate);
        let mut data: &[f32] = &chunk.samples;

        if let Some(floor) = floor {
            if start < floor {
                let skip = ((floor - start) as usize).min(data.len() / channels);
                data = &data[skip * channels..];
                start = floor;
            }
        }
        if data.is_empty() {
            return;
        }

        if self.is_empty() {
            self.start = start;
        } else {
            let end = self.end();
            let tolerance = frames_of(TOLERANCE, self.rate);
            let max_gap = frames_of(MAX_BRIDGED_GAP, self.rate);
            if start >= end {
                let gap = start - end;
                if gap > max_gap {
                    self.clear();
                    self.start = start;
                } else if gap > tolerance {
                    self.samples
                        .extend(std::iter::repeat_n(0.0, gap as usize * channels));
                }
                // Within tolerance: trust sample counting, append directly.
            } else {
                let overlap = end - start;
                if overlap > max_gap {
                    self.clear();
                    self.start = start;
                } else if overlap > tolerance {
                    let skip = (overlap as usize).min(data.len() / channels);
                    data = &data[skip * channels..];
                    if data.is_empty() {
                        return;
                    }
                }
            }
        }
        self.samples.extend(data.iter().copied());

        // Bound memory if the consumer stalls.
        let cap = frames_of(MAX_QUEUED, self.rate);
        if self.frames() > cap {
            let drop_frames = self.frames() - cap;
            self.samples.drain(..drop_frames as usize * channels);
            self.start += drop_frames;
        }
    }

    /// Stereo frame at absolute index `frame` (silence outside the queue).
    fn stereo_at(&self, frame: i64) -> (f32, f32) {
        let offset = frame - self.start;
        if offset < 0 || offset >= self.frames() {
            return (0.0, 0.0);
        }
        let i = offset as usize * self.channels;
        if self.channels == 1 {
            let s = self.samples[i];
            (s, s)
        } else {
            (self.samples[i], self.samples[i + 1])
        }
    }

    /// Forget everything before `frame`.
    fn drop_before(&mut self, frame: i64) {
        if frame <= self.start {
            return;
        }
        let n = ((frame - self.start).min(self.frames())) as usize;
        self.samples.drain(..n * self.channels);
        self.start = frame;
    }
}

/// Mixes a stereo system stream with a mono (or stereo) microphone stream
/// into contiguous stereo blocks. See the module docs.
pub struct AudioMixer {
    rate: u32,
    system: DenseQueue,
    mic: DenseQueue,
    /// First frame of the next block to emit.
    cursor: Option<i64>,
}

impl AudioMixer {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            rate: sample_rate,
            system: DenseQueue::new(2, sample_rate),
            mic: DenseQueue::new(1, sample_rate),
            cursor: None,
        }
    }

    pub fn push_system(&mut self, chunk: AudioChunk) {
        self.system.push(&chunk, self.cursor);
    }

    pub fn push_mic(&mut self, chunk: AudioChunk) {
        // The microphone queue's channel count follows the source.
        if usize::from(chunk.channels) != self.mic.channels
            && matches!(chunk.channels, 1 | 2)
            && self.mic.is_empty()
        {
            self.mic = DenseQueue::new(usize::from(chunk.channels), self.rate);
        }
        self.mic.push(&chunk, self.cursor);
    }

    /// The microphone went away: forget its pending audio.
    pub fn clear_mic(&mut self) {
        self.mic.clear();
    }

    fn block(&mut self, cursor: i64) -> AudioChunk {
        let mut samples = Vec::with_capacity(BLOCK_FRAMES * 2);
        for frame in cursor..cursor + BLOCK_FRAMES as i64 {
            let (sl, sr) = self.system.stereo_at(frame);
            let (ml, mr) = self.mic.stereo_at(frame);
            samples.push(soft_limit(sl + ml));
            samples.push(soft_limit(sr + mr));
        }
        AudioChunk {
            samples,
            channels: 2,
            sample_rate: self.rate,
            timestamp: frames_to_duration(cursor.max(0) as u64, self.rate),
        }
    }

    fn emit_until(&mut self, limit: i64) -> Vec<AudioChunk> {
        let mut out = Vec::new();
        let Some(mut cursor) = self.cursor else {
            return out;
        };
        while cursor + BLOCK_FRAMES as i64 <= limit {
            out.push(self.block(cursor));
            cursor += BLOCK_FRAMES as i64;
        }
        self.cursor = Some(cursor);
        self.system.drop_before(cursor);
        self.mic.drop_before(cursor);
        out
    }

    fn newest(&self) -> Option<i64> {
        [&self.system, &self.mic]
            .into_iter()
            .filter(|q| !q.is_empty())
            .map(DenseQueue::end)
            .max()
    }

    /// Every block that is ready now, in order.
    pub fn pop_ready(&mut self) -> Vec<AudioChunk> {
        if self.cursor.is_none() {
            self.cursor = [&self.system, &self.mic]
                .into_iter()
                .filter(|q| !q.is_empty())
                .map(|q| q.start)
                .min();
        }
        let (Some(cursor), Some(newest)) = (self.cursor, self.newest()) else {
            return Vec::new();
        };
        let horizon = newest - frames_of(MIX_LATENCY, self.rate);
        // A huge jump (clock reset, long stall) restarts instead of emitting
        // minutes of silence.
        let max_gap = frames_of(MAX_BRIDGED_GAP, self.rate);
        if horizon - cursor > max_gap {
            self.cursor = Some(horizon - frames_of(Duration::from_millis(100), self.rate));
        }
        self.emit_until(horizon)
    }

    /// End of stream: emit everything left, without waiting for latency.
    pub fn flush(&mut self) -> Vec<AudioChunk> {
        let Some(newest) = self.newest() else {
            return Vec::new();
        };
        self.emit_until(newest)
    }
}

// ---- the stage thread ------------------------------------------------------

/// Commands for a running [`MixStage`].
pub enum MixControl {
    AttachMic(mpsc::Receiver<AudioChunk>),
    DetachMic,
}

/// A thread running an [`AudioMixer`] between the system-audio capture and the
/// encoder.
pub struct MixStage {
    pub control: mpsc::UnboundedSender<MixControl>,
    pub thread: std::thread::JoinHandle<()>,
}

async fn recv_mic(mic: &mut Option<mpsc::Receiver<AudioChunk>>) -> Option<AudioChunk> {
    match mic {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Start the stage. `system` is the capture's audio stream; the returned
/// receiver carries the mixed (or, without a microphone, plain system) audio
/// and ends when `system` does.
pub fn spawn_mix_stage(
    mut system: mpsc::Receiver<AudioChunk>,
    sample_rate: u32,
) -> (MixStage, mpsc::Receiver<AudioChunk>) {
    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    let (out_tx, out_rx) = mpsc::channel(128);

    let thread = std::thread::Builder::new()
        .name("clipforge-mix".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("mix stage runtime");
            runtime.block_on(async move {
                let mut mixer = AudioMixer::new(sample_rate);
                let mut mic: Option<mpsc::Receiver<AudioChunk>> = None;
                let mut system_open = true;
                let mut control_open = true;

                while system_open {
                    tokio::select! {
                        chunk = system.recv() => match chunk {
                            Some(chunk) => mixer.push_system(chunk),
                            None => system_open = false,
                        },
                        chunk = recv_mic(&mut mic) => match chunk {
                            Some(chunk) => mixer.push_mic(chunk),
                            None => mic = None, // the source ended; nothing more to expect
                        },
                        command = control_rx.recv(), if control_open => match command {
                            Some(MixControl::AttachMic(rx)) => {
                                mixer.clear_mic();
                                mic = Some(rx);
                            }
                            Some(MixControl::DetachMic) => {
                                mixer.clear_mic();
                                mic = None;
                            }
                            None => control_open = false,
                        },
                    }
                    for block in mixer.pop_ready() {
                        if out_tx.send(block).await.is_err() {
                            return;
                        }
                    }
                }
                for block in mixer.flush() {
                    if out_tx.send(block).await.is_err() {
                        return;
                    }
                }
            });
        })
        .expect("spawn mix thread");

    (
        MixStage {
            control: control_tx,
            thread,
        },
        out_rx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A chunk of constant value, `ms` long, starting at `start_ms`.
    fn tone(start_ms: u64, ms: u64, channels: u16, value: f32) -> AudioChunk {
        let frames = (ms * 48) as usize;
        AudioChunk {
            samples: vec![value; frames * usize::from(channels)],
            channels,
            sample_rate: RATE,
            timestamp: Duration::from_millis(start_ms),
        }
    }

    fn system(start_ms: u64, ms: u64, v: f32) -> AudioChunk {
        tone(start_ms, ms, 2, v)
    }

    fn mic(start_ms: u64, ms: u64, v: f32) -> AudioChunk {
        tone(start_ms, ms, 1, v)
    }

    /// Feed chunks in 10 ms pieces and gather everything ready.
    fn collect(mixer: &mut AudioMixer) -> Vec<AudioChunk> {
        let mut out = mixer.pop_ready();
        out.extend(mixer.flush());
        out
    }

    fn value_at(blocks: &[AudioChunk], ms: u64) -> f32 {
        let frame = (ms * 48) as usize;
        let block = &blocks[frame / BLOCK_FRAMES];
        block.samples[(frame % BLOCK_FRAMES) * 2]
    }

    #[test]
    fn blocks_are_contiguous_stereo_and_exactly_10_ms() {
        let mut m = AudioMixer::new(RATE);
        for i in 0..100 {
            m.push_system(system(i * 10, 10, 0.1));
        }
        let blocks = collect(&mut m);
        assert_eq!(blocks.len(), 100);
        for (i, b) in blocks.iter().enumerate() {
            assert_eq!(b.channels, 2);
            assert_eq!(b.frames(), BLOCK_FRAMES);
            assert_eq!(b.timestamp, Duration::from_millis(i as u64 * 10));
        }
    }

    #[test]
    fn without_a_microphone_the_system_audio_passes_through_untouched() {
        let mut m = AudioMixer::new(RATE);
        for i in 0..50 {
            m.push_system(system(i * 10, 10, 0.37));
        }
        let blocks = collect(&mut m);
        assert!(blocks.iter().all(|b| b.samples.iter().all(|s| *s == 0.37)));
    }

    #[test]
    fn mic_is_upmixed_to_both_channels_and_added() {
        let mut m = AudioMixer::new(RATE);
        for i in 0..50 {
            m.push_system(system(i * 10, 10, 0.2));
            m.push_mic(mic(i * 10, 10, 0.3));
        }
        let blocks = collect(&mut m);
        for b in &blocks {
            for frame in b.samples.chunks_exact(2) {
                assert!((frame[0] - 0.5).abs() < 1e-6 && (frame[1] - 0.5).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn sources_line_up_by_timestamp_not_arrival_order() {
        // A 10 ms "click" at t = 200 ms in each source. The microphone's data
        // arrives much later, in bigger pieces, yet both clicks must coincide.
        let mut m = AudioMixer::new(RATE);
        for i in 0..40 {
            let v = if i == 20 { 0.4 } else { 0.0 };
            m.push_system(system(i * 10, 10, v));
        }
        for piece in 0..4 {
            // 100 ms pieces: click is in piece 2 (200-300 ms), first 10 ms.
            let mut c = mic(piece * 100, 100, 0.0);
            if piece == 2 {
                c.samples[..480].fill(0.4);
            }
            m.push_mic(c);
        }
        let blocks = collect(&mut m);
        assert!(
            (value_at(&blocks, 205) - 0.8).abs() < 1e-6,
            "clicks add at 205 ms"
        );
        assert_eq!(value_at(&blocks, 195), 0.0);
        assert_eq!(value_at(&blocks, 215), 0.0);
    }

    #[test]
    fn a_microphone_that_lags_within_the_latency_is_still_mixed() {
        let mut m = AudioMixer::new(RATE);
        let mut out = Vec::new();
        // Each 10 ms step the system is current; the mic is 50 ms behind.
        for i in 0..60u64 {
            m.push_system(system(i * 10, 10, 0.1));
            if i >= 5 {
                m.push_mic(mic((i - 5) * 10, 10, 0.2));
            }
            out.extend(m.pop_ready());
        }
        out.extend(m.flush());
        // Well after the start, every block carries both voices.
        let settled = &out[10..40];
        assert!(settled
            .iter()
            .all(|b| b.samples.iter().all(|s| (s - 0.3).abs() < 1e-6)));
    }

    #[test]
    fn a_microphone_later_than_the_latency_is_dropped_not_waited_for() {
        let mut m = AudioMixer::new(RATE);
        let mut out = Vec::new();
        for i in 0..60u64 {
            m.push_system(system(i * 10, 10, 0.1));
            out.extend(m.pop_ready());
        }
        // 600 ms of microphone shows up only now: far too late for the
        // blocks already released.
        m.push_mic(mic(0, 500, 0.5));
        out.extend(m.pop_ready());
        out.extend(m.flush());
        assert!(out[..40]
            .iter()
            .all(|b| b.samples.iter().all(|s| *s == 0.1)));
    }

    #[test]
    fn a_microphone_that_stops_leaves_the_system_audio_flowing() {
        let mut m = AudioMixer::new(RATE);
        for i in 0..100u64 {
            m.push_system(system(i * 10, 10, 0.1));
            if i < 30 {
                m.push_mic(mic(i * 10, 10, 0.2));
            }
        }
        let blocks = collect(&mut m);
        assert_eq!(blocks.len(), 100, "no stall");
        assert!((value_at(&blocks, 100) - 0.3).abs() < 1e-6);
        assert!((value_at(&blocks, 700) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn a_silent_system_does_not_freeze_the_microphone() {
        // Windows loopback delivers nothing while nothing plays.
        let mut m = AudioMixer::new(RATE);
        for i in 0..100u64 {
            m.push_mic(mic(i * 10, 10, 0.25));
        }
        let blocks = collect(&mut m);
        assert_eq!(blocks.len(), 100);
        assert!(blocks.iter().all(|b| b.samples.iter().all(|s| *s == 0.25)));
    }

    #[test]
    fn a_hole_in_the_system_audio_becomes_silence_under_the_voice() {
        let mut m = AudioMixer::new(RATE);
        m.push_system(system(0, 200, 0.1));
        m.push_system(system(400, 200, 0.1)); // 200 ms hole
        m.push_mic(mic(0, 600, 0.2));
        let blocks = collect(&mut m);
        assert!((value_at(&blocks, 100) - 0.3).abs() < 1e-6);
        assert!(
            (value_at(&blocks, 300) - 0.2).abs() < 1e-6,
            "hole = voice only"
        );
        assert!((value_at(&blocks, 500) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn detaching_the_microphone_stops_mixing_it() {
        let mut m = AudioMixer::new(RATE);
        let mut blocks = Vec::new();
        for i in 0..20u64 {
            m.push_system(system(i * 10, 10, 0.1));
            m.push_mic(mic(i * 10, 10, 0.2));
            blocks.extend(m.pop_ready());
        }
        m.clear_mic(); // the microphone is switched off
        for i in 20..60u64 {
            m.push_system(system(i * 10, 10, 0.1));
            blocks.extend(m.pop_ready());
        }
        blocks.extend(m.flush());
        assert!(
            (value_at(&blocks, 20) - 0.3).abs() < 1e-6,
            "mixed while attached"
        );
        assert!(
            (value_at(&blocks, 500) - 0.1).abs() < 1e-6,
            "system only afterwards"
        );
    }

    #[test]
    fn the_limiter_keeps_loud_sums_below_full_scale() {
        let mut m = AudioMixer::new(RATE);
        for i in 0..20u64 {
            m.push_system(system(i * 10, 10, 0.95));
            m.push_mic(mic(i * 10, 10, 0.95));
        }
        for b in collect(&mut m) {
            assert!(b.samples.iter().all(|s| s.abs() <= 1.0 && *s > 0.9));
        }
    }

    #[test]
    fn soft_limit_is_transparent_below_the_knee_and_monotonic_above() {
        for x in [-0.9f32, -0.5, 0.0, 0.3, 0.9] {
            assert_eq!(soft_limit(x), x);
        }
        let mut last = soft_limit(0.9);
        for step in 1..=200 {
            let v = soft_limit(0.9 + step as f32 * 0.05);
            assert!(v >= last && v <= 1.0, "{v} after {last}");
            last = v;
        }
        assert_eq!(soft_limit(-5.0), -soft_limit(5.0));
    }

    #[test]
    fn jitter_within_tolerance_does_not_open_gaps() {
        let mut m = AudioMixer::new(RATE);
        // Chunks stamped 5 ms late each time (cumulative drift of the stamps).
        for i in 0..50u64 {
            m.push_system(system(i * 10 + (i % 2) * 5, 10, 0.1));
        }
        let blocks = collect(&mut m);
        assert!(blocks.iter().all(|b| b.samples.iter().all(|s| *s == 0.1)));
    }

    #[test]
    fn a_huge_forward_jump_restarts_without_emitting_minutes_of_silence() {
        let mut m = AudioMixer::new(RATE);
        for i in 0..20u64 {
            m.push_system(system(i * 10, 10, 0.1));
        }
        let before = collect(&mut m).len();
        assert_eq!(before, 20);
        m.push_system(system(600_000, 100, 0.1)); // ten minutes later
        let after = collect(&mut m);
        assert!(after.len() < 100, "{} blocks", after.len());
        assert!(after.last().unwrap().timestamp >= Duration::from_secs(599));
    }

    #[test]
    fn a_source_with_a_different_format_is_ignored() {
        let mut m = AudioMixer::new(RATE);
        let mut odd = system(0, 100, 0.9);
        odd.sample_rate = 44_100;
        m.push_system(odd);
        assert!(collect(&mut m).is_empty());
    }

    // ---- the stage thread ----

    /// Feed both sources "live": one pair of chunks per step, with a short
    /// pause so the stage thread keeps up, as real capture threads would.
    fn run_stage(
        system_chunks: Vec<AudioChunk>,
        mic_chunks: Option<Vec<AudioChunk>>,
    ) -> Vec<AudioChunk> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let (sys_tx, sys_rx) = mpsc::channel(512);
            let (stage, mut out) = spawn_mix_stage(sys_rx, RATE);
            let mic_tx = mic_chunks.as_ref().map(|_| {
                let (mic_tx, mic_rx) = mpsc::channel(512);
                stage.control.send(MixControl::AttachMic(mic_rx)).unwrap();
                mic_tx
            });
            let mut mic_iter = mic_chunks.unwrap_or_default().into_iter();
            for chunk in system_chunks {
                sys_tx.send(chunk).await.unwrap();
                if let (Some(tx), Some(mic_chunk)) = (&mic_tx, mic_iter.next()) {
                    tx.send(mic_chunk).await.unwrap();
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            drop(mic_tx);
            drop(sys_tx);
            let mut blocks = Vec::new();
            while let Some(b) = out.recv().await {
                blocks.push(b);
            }
            stage.thread.join().unwrap();
            blocks
        })
    }

    #[test]
    fn the_stage_passes_system_audio_through_and_ends_with_it() {
        let blocks = run_stage((0..30).map(|i| system(i * 10, 10, 0.2)).collect(), None);
        assert_eq!(blocks.len(), 30);
        assert!(blocks.iter().all(|b| b.samples.iter().all(|s| *s == 0.2)));
    }

    #[test]
    fn the_stage_mixes_an_attached_microphone() {
        let blocks = run_stage(
            (0..30).map(|i| system(i * 10, 10, 0.2)).collect(),
            Some((0..30).map(|i| mic(i * 10, 10, 0.1)).collect()),
        );
        assert!(!blocks.is_empty());
        let mixed = blocks
            .iter()
            .filter(|b| b.samples.iter().all(|s| (s - 0.3).abs() < 1e-6))
            .count();
        assert!(mixed >= 20, "{mixed} of {} blocks carry both", blocks.len());
    }
}
