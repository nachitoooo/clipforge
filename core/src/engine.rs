//! The background capture engine: wires a [`FrameSource`] through an
//! [`Encoder`] into the shared [`ReplayBuffer`], on a dedicated thread so
//! blocking hardware encoders never touch an async runtime.
//!
//! The engine is what "start/stop buffer" controls (tray, UI, tests). Saving
//! clips is *not* its job — a [`crate::clip::ClipService`] shares the same
//! buffer and can extract at any moment without stopping capture.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use crate::buffer::{BufferConfig, ReplayBuffer};
use crate::capture::{CaptureError, FrameSource};
use crate::clip::mp4mux::VideoParams;
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
}

/// Counters exposed to the UI/logs.
#[derive(Debug, Clone, Copy, Default)]
pub struct EngineStats {
    pub frames_encoded: u64,
    pub packets_buffered: u64,
    pub encode_errors: u64,
}

struct Worker {
    handle: std::thread::JoinHandle<()>,
    source: Box<dyn FrameSource>,
}

/// Owns the replay buffer and the capture→encode→buffer worker thread.
pub struct CaptureEngine {
    buffer: Arc<Mutex<ReplayBuffer>>,
    frames_encoded: Arc<AtomicU64>,
    packets_buffered: Arc<AtomicU64>,
    encode_errors: Arc<AtomicU64>,
    worker: Option<Worker>,
}

impl CaptureEngine {
    pub fn new(buffer_config: BufferConfig) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(ReplayBuffer::new(buffer_config))),
            frames_encoded: Arc::new(AtomicU64::new(0)),
            packets_buffered: Arc::new(AtomicU64::new(0)),
            encode_errors: Arc::new(AtomicU64::new(0)),
            worker: None,
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
        }
    }

    /// Start capturing: frames from `source` are encoded and pushed into the
    /// buffer until `stop` is called or the source ends. Publishes
    /// [`StreamInfo`] to `clips` as soon as the encoder produces parameter
    /// sets, so clip saving becomes possible.
    pub fn start(
        &mut self,
        mut source: Box<dyn FrameSource>,
        mut encoder: Box<dyn Encoder>,
        clips: Arc<ClipService>,
    ) -> Result<(), EngineError> {
        if self.is_running() {
            return Err(EngineError::AlreadyRunning);
        }
        let mut rx = source.start()?;

        let buffer = Arc::clone(&self.buffer);
        let frames_encoded = Arc::clone(&self.frames_encoded);
        let packets_buffered = Arc::clone(&self.packets_buffered);
        let encode_errors = Arc::clone(&self.encode_errors);

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

        self.worker = Some(Worker { handle, source });
        Ok(())
    }

    /// Stop capturing. The buffered window is kept — clips of the last
    /// minutes can still be saved after stopping. Idempotent.
    pub fn stop(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.source.stop(); // closes the frame channel → thread ends
            let _ = worker.handle.join();
        }
    }

    /// Wait until the worker thread ends on its own (source exhausted).
    /// Test helper; production code uses `stop`.
    pub fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.handle.join();
        }
    }
}

impl Drop for CaptureEngine {
    fn drop(&mut self) {
        self.stop();
    }
}
