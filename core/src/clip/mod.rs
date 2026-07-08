//! Clip output: extract the last N seconds from the replay buffer and save
//! them as an .mp4 ([`ClipService`]), muxing in-process ([`mp4mux`]) — a
//! container write, never a re-encode. The ffmpeg sidecar remains reserved
//! for stream-copy trimming of existing files (Milestone 3).

pub mod mp4mux;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;

use crate::buffer::ReplayBuffer;
use crate::encode::ParameterSets;
use mp4mux::{write_mp4, MuxError, VideoParams};

#[derive(Debug, Error)]
pub enum ClipError {
    #[error("replay buffer is empty — nothing to save yet")]
    BufferEmpty,
    #[error("stream info not available yet (no frames encoded)")]
    NotReady,
    #[error("could not create clips folder {0}: {1}")]
    ClipsDir(PathBuf, std::io::Error),
    #[error(transparent)]
    Mux(#[from] MuxError),
}

/// Everything the muxer needs about the running stream; published by the
/// capture pipeline once the encoder has produced its first packet.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub video: VideoParams,
    pub parameter_sets: ParameterSets,
}

/// A clip that was written to disk.
#[derive(Debug, Clone)]
pub struct SavedClip {
    pub path: PathBuf,
    /// Track duration actually written (≥ requested, keyframe-aligned).
    pub duration: Duration,
    pub packets: usize,
    pub bytes: u64,
}

/// Saves "the last N seconds" from a shared [`ReplayBuffer`] into a clips
/// folder. Holds the buffer lock only long enough to clone the packets, so
/// capture keeps running uninterrupted while the file is written.
pub struct ClipService {
    buffer: Arc<Mutex<ReplayBuffer>>,
    clips_dir: PathBuf,
    stream_info: Mutex<Option<StreamInfo>>,
}

impl ClipService {
    pub fn new(buffer: Arc<Mutex<ReplayBuffer>>, clips_dir: impl Into<PathBuf>) -> Self {
        Self {
            buffer,
            clips_dir: clips_dir.into(),
            stream_info: Mutex::new(None),
        }
    }

    /// Publish (or refresh, e.g. after an encoder restart) the stream info
    /// clips will be muxed with.
    pub fn set_stream_info(&self, info: StreamInfo) {
        *self.stream_info.lock().expect("stream_info lock") = Some(info);
    }

    pub fn clips_dir(&self) -> &Path {
        &self.clips_dir
    }

    /// Save the last `duration` of buffered video as a timestamped .mp4.
    ///
    /// Lock discipline: the replay buffer is locked only for the packet
    /// clone; muxing and disk I/O happen outside the lock.
    pub fn save_last(&self, duration: Duration) -> Result<SavedClip, ClipError> {
        let info = self
            .stream_info
            .lock()
            .expect("stream_info lock")
            .clone()
            .ok_or(ClipError::NotReady)?;

        let packets = {
            let buffer = self.buffer.lock().expect("replay buffer lock");
            buffer.extract_last(duration)
        };
        if packets.is_empty() {
            return Err(ClipError::BufferEmpty);
        }

        std::fs::create_dir_all(&self.clips_dir)
            .map_err(|e| ClipError::ClipsDir(self.clips_dir.clone(), e))?;
        let path = self.unique_clip_path();
        let written = write_mp4(&path, &info.video, &info.parameter_sets, &packets)?;
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        Ok(SavedClip {
            path,
            duration: written,
            packets: packets.len(),
            bytes,
        })
    }

    /// `clip-YYYYMMDD-HHMMSS.mp4` (local time), de-duplicated with `-2`,
    /// `-3`… if several clips land in the same second.
    fn unique_clip_path(&self) -> PathBuf {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let base = format!("clip-{stamp}");
        let mut path = self.clips_dir.join(format!("{base}.mp4"));
        let mut n = 2u32;
        while path.exists() {
            path = self.clips_dir.join(format!("{base}-{n}.mp4"));
            n += 1;
        }
        path
    }
}
