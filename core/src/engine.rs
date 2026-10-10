//! The background capture engine: wires a [`FrameSource`] through an
//! [`Encoder`] into the shared [`ReplayBuffer`], on a dedicated thread so
//! blocking hardware encoders never touch an async runtime. When the source
//! also captures audio, it flows through a mixing stage (where a microphone
//! can be mixed in, live) and an [`AudioEncoder`] into the same buffer. A
//! microphone can instead get its own lane and track.
//!
//! The engine is what "start/stop buffer" controls (tray, UI, tests). Saving
//! clips is *not* its job — a [`crate::clip::ClipService`] shares the same
//! buffer and can extract at any moment without stopping capture.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use tokio::sync::mpsc;

use crate::audio::mixer::{spawn_mix_stage, MixControl, MixStage};
use crate::audio::{
    AudioChunk, AudioEncoder, AudioError, AudioLaneId, AudioPacket, ContinuityGuard, MicMode,
};
use crate::buffer::{BufferConfig, ReplayBuffer};
use crate::capture::clock::SharedEpoch;
use crate::capture::{AudioSource, CaptureError, FrameSource};
use crate::clip::mp4mux::{AudioParams, VideoParams};
use crate::clip::{ClipService, StreamInfo};
use crate::encode::{EncodeError, Encoder};

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Encode(#[from] EncodeError),
    #[error("engine already running")]
    AlreadyRunning,
    #[error("the replay buffer isn't running")]
    NotRunning,
    #[error("a microphone is already attached")]
    MicrophoneAttached,
    #[error("this capture has no audio pipeline to attach the microphone to")]
    AudioUnavailable,
}

/// Counters exposed to the UI/logs.
#[derive(Debug, Clone, Copy, Default)]
pub struct EngineStats {
    pub frames_encoded: u64,
    pub packets_buffered: u64,
    pub encode_errors: u64,
    /// AAC packets stored in the buffer (0 when audio isn't being captured).
    pub audio_packets_buffered: u64,
    pub audio_errors: u64,
    /// AAC packets of the separate microphone lane.
    pub mic_packets_buffered: u64,
    pub mic_errors: u64,
}

struct Worker {
    handle: std::thread::JoinHandle<()>,
    audio_handle: Option<std::thread::JoinHandle<()>>,
    /// Sits between the system-audio capture and the audio encoder; the
    /// microphone attaches here in [`MicMode::Mixed`].
    mix: Option<MixStage>,
    source: Box<dyn FrameSource>,
}

/// A microphone currently feeding the engine.
struct MicLane {
    mode: MicMode,
    source: Box<dyn AudioSource>,
    /// The lane's encoder thread ([`MicMode::Separate`] only).
    worker: Option<std::thread::JoinHandle<()>>,
}

/// Owns the replay buffer and the capture→encode→buffer worker thread.
pub struct CaptureEngine {
    buffer: Arc<Mutex<ReplayBuffer>>,
    frames_encoded: Arc<AtomicU64>,
    packets_buffered: Arc<AtomicU64>,
    encode_errors: Arc<AtomicU64>,
    audio_packets_buffered: Arc<AtomicU64>,
    audio_errors: Arc<AtomicU64>,
    mic_packets_buffered: Arc<AtomicU64>,
    mic_errors: Arc<AtomicU64>,
    worker: Option<Worker>,
    epoch: Option<SharedEpoch>,
    clips: Option<Arc<ClipService>>,
    mic: Option<MicLane>,
}

impl CaptureEngine {
    pub fn new(buffer_config: BufferConfig) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(ReplayBuffer::new(buffer_config))),
            frames_encoded: Arc::new(AtomicU64::new(0)),
            packets_buffered: Arc::new(AtomicU64::new(0)),
            encode_errors: Arc::new(AtomicU64::new(0)),
            audio_packets_buffered: Arc::new(AtomicU64::new(0)),
            audio_errors: Arc::new(AtomicU64::new(0)),
            mic_packets_buffered: Arc::new(AtomicU64::new(0)),
            mic_errors: Arc::new(AtomicU64::new(0)),
            worker: None,
            epoch: None,
            clips: None,
            mic: None,
        }
    }

    /// The buffer this engine fills; share it with a
    /// [`ClipService`](crate::clip::ClipService).
    pub fn buffer(&self) -> Arc<Mutex<ReplayBuffer>> {
        Arc::clone(&self.buffer)
    }

    pub fn is_running(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|w| !w.handle.is_finished())
    }

    pub fn stats(&self) -> EngineStats {
        EngineStats {
            frames_encoded: self.frames_encoded.load(Ordering::Relaxed),
            packets_buffered: self.packets_buffered.load(Ordering::Relaxed),
            encode_errors: self.encode_errors.load(Ordering::Relaxed),
            audio_packets_buffered: self.audio_packets_buffered.load(Ordering::Relaxed),
            audio_errors: self.audio_errors.load(Ordering::Relaxed),
            mic_packets_buffered: self.mic_packets_buffered.load(Ordering::Relaxed),
            mic_errors: self.mic_errors.load(Ordering::Relaxed),
        }
    }

    /// The clock epoch of the running capture. A microphone source must be
    /// built with it to line up with the video. `None` before `start`, and for
    /// sources that keep their own clock.
    pub fn epoch(&self) -> Option<SharedEpoch> {
        self.epoch.clone()
    }

    /// How the attached microphone is being recorded, if one is.
    pub fn microphone_mode(&self) -> Option<MicMode> {
        self.mic.as_ref().map(|m| m.mode)
    }

    /// Start capturing: frames from `source` are encoded and pushed into the
    /// buffer until `stop` is called or the source ends. Publishes
    /// [`StreamInfo`] to `clips` as soon as the encoder produces parameter
    /// sets, so clip saving becomes possible.
    pub fn start(
        &mut self,
        source: Box<dyn FrameSource>,
        encoder: Box<dyn Encoder>,
        clips: Arc<ClipService>,
    ) -> Result<(), EngineError> {
        self.start_with_audio(source, encoder, None, clips)
    }

    /// [`Self::start`] plus audio: if `source` captures audio
    /// ([`FrameSource::take_audio`]) it is encoded with `audio_encoder` and
    /// stored next to the video, so saved clips carry sound. A source without
    /// audio simply yields silent clips.
    pub fn start_with_audio(
        &mut self,
        mut source: Box<dyn FrameSource>,
        mut encoder: Box<dyn Encoder>,
        audio_encoder: Option<Box<dyn AudioEncoder>>,
        clips: Arc<ClipService>,
    ) -> Result<(), EngineError> {
        if self.is_running() {
            return Err(EngineError::AlreadyRunning);
        }
        let mut rx = source.start()?;
        self.epoch = source.epoch();
        self.clips = Some(Arc::clone(&clips));
        // Taking the stream even without an encoder closes it, so a source
        // that blocks on a full audio channel is released.
        let audio = audio_encoder.zip(source.take_audio());

        let buffer = Arc::clone(&self.buffer);
        let frames_encoded = Arc::clone(&self.frames_encoded);
        let packets_buffered = Arc::clone(&self.packets_buffered);
        let encode_errors = Arc::clone(&self.encode_errors);
        let mut mix = None;
        let audio_handle = audio.map(|(audio_encoder, audio_rx)| {
            let config = *audio_encoder.config();
            // The mixing stage works in 48 kHz stereo (what the system capture
            // delivers); any other format goes straight to the encoder and
            // can't have a microphone mixed in.
            let audio_rx = if config.sample_rate == 48_000 && config.channels == 2 {
                let (stage, mixed) = spawn_mix_stage(audio_rx, config.sample_rate);
                mix = Some(stage);
                mixed
            } else {
                audio_rx
            };
            spawn_audio_worker(
                AudioLaneId::Main,
                audio_encoder,
                audio_rx,
                Arc::clone(&self.buffer),
                Arc::clone(&clips),
                Arc::clone(&self.audio_packets_buffered),
                Arc::clone(&self.audio_errors),
            )
        });

        let handle = std::thread::Builder::new()
            .name("clipforge-engine".into())
            .spawn(move || {
                let mut info_published = false;
                while let Some(frame) = rx.blocking_recv() {
                    let packets = match encoder.encode(&frame) {
                        Ok(packets) => packets,
                        Err(e) => {
                            // Keep capturing: one bad frame must not kill the
                            // session. Persistent failure shows up in stats.
                            eprintln!("clipforge-engine: encode error: {e}");
                            encode_errors.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    };
                    frames_encoded.fetch_add(1, Ordering::Relaxed);

                    if !info_published {
                        if let Some(parameter_sets) = encoder.parameter_sets() {
                            let config = encoder.config();
                            clips.set_stream_info(StreamInfo {
                                video: VideoParams {
                                    codec: config.codec,
                                    width: config.width,
                                    height: config.height,
                                    fps: config.fps,
                                },
                                parameter_sets,
                            });
                            info_published = true;
                        }
                    }

                    if !packets.is_empty() {
                        let mut locked = buffer.lock().expect("replay buffer lock");
                        for packet in packets {
                            locked.push(packet);
                            packets_buffered.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                // Drain the encoder pipeline on clean end-of-stream.
                if let Ok(packets) = encoder.finish() {
                    let mut locked = buffer.lock().expect("replay buffer lock");
                    for packet in packets {
                        locked.push(packet);
                        packets_buffered.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
            .expect("spawn engine thread");

        self.worker = Some(Worker {
            handle,
            audio_handle,
            mix,
            source,
        });
        Ok(())
    }

    /// Attach an already-started microphone (`rx` is the stream `source`
    /// returned from `start`). Starting the source is the caller's job on
    /// purpose: opening a device can block on a permission prompt, and that
    /// must not happen while the engine is locked.
    ///
    /// * [`MicMode::Mixed`]: the voice joins the game audio before encoding.
    /// * [`MicMode::Separate`]: the voice gets its own lane, encoded with
    ///   `encoder`, and ends up as a second audio track in clips.
    ///
    /// Live: nothing about the video or the game audio restarts.
    pub fn attach_microphone(
        &mut self,
        source: Box<dyn AudioSource>,
        rx: mpsc::Receiver<AudioChunk>,
        mode: MicMode,
        encoder: Option<Box<dyn AudioEncoder>>,
    ) -> Result<(), EngineError> {
        if !self.is_running() {
            return Err(EngineError::NotRunning);
        }
        if self.mic.is_some() {
            return Err(EngineError::MicrophoneAttached);
        }
        let worker = self.worker.as_ref().expect("running implies a worker");
        match mode {
            MicMode::Mixed => {
                let stage = worker.mix.as_ref().ok_or(EngineError::AudioUnavailable)?;
                stage
                    .control
                    .send(MixControl::AttachMic(rx))
                    .map_err(|_| EngineError::AudioUnavailable)?;
                self.mic = Some(MicLane {
                    mode,
                    source,
                    worker: None,
                });
            }
            MicMode::Separate => {
                let encoder = encoder.ok_or(EngineError::AudioUnavailable)?;
                let clips = self.clips.clone().ok_or(EngineError::NotRunning)?;
                // A fresh lane: leftovers from an earlier attach would leave a
                // hole in the middle of the track.
                self.buffer
                    .lock()
                    .expect("replay buffer lock")
                    .clear_audio_lane(AudioLaneId::Mic);
                let handle = spawn_audio_worker(
                    AudioLaneId::Mic,
                    encoder,
                    rx,
                    Arc::clone(&self.buffer),
                    clips,
                    Arc::clone(&self.mic_packets_buffered),
                    Arc::clone(&self.mic_errors),
                );
                self.mic = Some(MicLane {
                    mode,
                    source,
                    worker: Some(handle),
                });
            }
        }
        Ok(())
    }

    /// Detach the microphone, live. Idempotent.
    pub fn detach_microphone(&mut self) {
        let Some(mut lane) = self.mic.take() else {
            return;
        };
        // Stopping the source closes its chunk stream, which is what ends a
        // separate lane's encoder thread (and tells the mixer it's gone).
        lane.source.stop();
        match lane.mode {
            MicMode::Mixed => {
                if let Some(stage) = self.worker.as_ref().and_then(|w| w.mix.as_ref()) {
                    let _ = stage.control.send(MixControl::DetachMic);
                }
            }
            MicMode::Separate => {
                if let Some(handle) = lane.worker {
                    let _ = handle.join();
                }
                if let Some(clips) = &self.clips {
                    clips.set_mic_info(None);
                }
                self.buffer
                    .lock()
                    .expect("replay buffer lock")
                    .clear_audio_lane(AudioLaneId::Mic);
            }
        }
    }

    fn join_worker(worker: Worker) {
        let _ = worker.handle.join();
        if let Some(stage) = worker.mix {
            let _ = stage.thread.join();
        }
        if let Some(audio) = worker.audio_handle {
            let _ = audio.join();
        }
    }

    /// Stop capturing. The buffered window is kept — clips of the last
    /// minutes can still be saved after stopping. Idempotent.
    pub fn stop(&mut self) {
        self.detach_microphone();
        if let Some(mut worker) = self.worker.take() {
            worker.source.stop(); // closes the frame channels → threads end
            Self::join_worker(worker);
        }
    }

    /// Wait until the worker thread ends on its own (source exhausted).
    /// Test helper; production code uses `stop`.
    pub fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            Self::join_worker(worker);
        }
        self.detach_microphone();
    }
}

/// Audio counterpart of the video worker: chunks → continuity guard → AAC →
/// the shared buffer.
fn spawn_audio_worker(
    lane: AudioLaneId,
    mut encoder: Box<dyn AudioEncoder>,
    mut rx: mpsc::Receiver<AudioChunk>,
    buffer: Arc<Mutex<ReplayBuffer>>,
    clips: Arc<ClipService>,
    packets_buffered: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name(match lane {
            AudioLaneId::Main => "clipforge-audio".into(),
            AudioLaneId::Mic => "clipforge-mic".into(),
        })
        .spawn(move || {
            let config = *encoder.config();
            let params = AudioParams {
                priming_frames: encoder.priming_frames(),
                ..AudioParams::from(config)
            };
            match lane {
                AudioLaneId::Main => clips.set_audio_info(params),
                AudioLaneId::Mic => clips.set_mic_info(Some(params)),
            }
            let mut guard = ContinuityGuard::new(config.sample_rate, config.channels);

            let store = |packets: Vec<AudioPacket>| {
                if packets.is_empty() {
                    return;
                }
                let mut locked = buffer.lock().expect("replay buffer lock");
                for packet in packets {
                    locked.push_audio_lane(lane, packet);
                    packets_buffered.fetch_add(1, Ordering::Relaxed);
                }
            };
            let report = |e: AudioError| {
                // Keep capturing: a bad chunk must not silence the session.
                eprintln!("clipforge-audio ({lane:?}): {e}");
                errors.fetch_add(1, Ordering::Relaxed);
            };

            while let Some(chunk) = rx.blocking_recv() {
                let Some(guarded) = guard.process(chunk) else {
                    continue;
                };
                if guarded.restart {
                    buffer
                        .lock()
                        .expect("replay buffer lock")
                        .clear_audio_lane(lane);
                    encoder.resync();
                }
                match encoder.encode(&guarded.chunk) {
                    Ok(packets) => store(packets),
                    Err(e) => report(e),
                }
            }
            match encoder.finish() {
                Ok(packets) => store(packets),
                Err(e) => report(e),
            }
        })
        .expect("spawn audio thread")
}

impl Drop for CaptureEngine {
    fn drop(&mut self) {
        self.stop();
    }
}
