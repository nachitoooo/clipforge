//! Clip output: extract the last N seconds from the replay buffer and save
//! them as an .mp4 ([`ClipService`]), muxing in-process ([`mp4mux`]) — a
//! container write, never a re-encode. [`trim`] edits saved clips the same
//! way (cut, drop audio tracks, draw waveforms) without an ffmpeg sidecar.

pub mod edit_list;
pub mod mp4mux;
pub mod trim;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;

use crate::buffer::ReplayBuffer;
use crate::encode::ParameterSets;
use mp4mux::{write_mp4_with_tracks, AudioParams, MuxError, VideoParams};

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
    /// AAC packets muxed alongside the video in the main audio track (0 for
    /// a silent clip).
    pub audio_packets: usize,
    /// AAC packets of the separate microphone track (0 when there is none).
    pub mic_packets: usize,
    pub bytes: u64,
}

/// Saves "the last N seconds" from a shared [`ReplayBuffer`] into a clips
/// folder. Holds the buffer lock only long enough to clone the packets, so
/// capture keeps running uninterrupted while the file is written.
pub struct ClipService {
    buffer: Arc<Mutex<ReplayBuffer>>,
    clips_dir: PathBuf,
    stream_info: Mutex<Option<StreamInfo>>,
    /// Set once an audio encoder is running; without it clips are video-only.
    audio_info: Mutex<Option<AudioParams>>,
    /// Set while the microphone is recorded as its own track.
    mic_info: Mutex<Option<AudioParams>>,
}

impl ClipService {
    pub fn new(buffer: Arc<Mutex<ReplayBuffer>>, clips_dir: impl Into<PathBuf>) -> Self {
        Self {
            buffer,
            clips_dir: clips_dir.into(),
            stream_info: Mutex::new(None),
            audio_info: Mutex::new(None),
            mic_info: Mutex::new(None),
        }
    }

    /// Publish (or refresh, e.g. after an encoder restart) the stream info
    /// clips will be muxed with.
    pub fn set_stream_info(&self, info: StreamInfo) {
        *self.stream_info.lock().expect("stream_info lock") = Some(info);
    }

    /// Announce the audio format of the packets the buffer receives; from
    /// now on clips carry an AAC track whenever the extracted span has audio.
    pub fn set_audio_info(&self, info: AudioParams) {
        *self.audio_info.lock().expect("audio_info lock") = Some(info);
    }

    /// Announce (or withdraw, with `None`) the separate microphone track.
    pub fn set_mic_info(&self, info: Option<AudioParams>) {
        *self.mic_info.lock().expect("mic_info lock") = info;
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

        let slice = {
            let buffer = self.buffer.lock().expect("replay buffer lock");
            buffer.extract_last_with_audio(duration)
        };
        if slice.video.is_empty() {
            return Err(ClipError::BufferEmpty);
        }
        let audio_info = *self.audio_info.lock().expect("audio_info lock");
        let mic_info = *self.mic_info.lock().expect("mic_info lock");
        let mut tracks: Vec<(&AudioParams, &[crate::audio::AudioPacket])> = Vec::new();
        if let Some(params) = audio_info.as_ref() {
            tracks.push((params, slice.audio.as_slice()));
        }
        if let Some(params) = mic_info.as_ref() {
            tracks.push((params, slice.mic.as_slice()));
        }

        std::fs::create_dir_all(&self.clips_dir)
            .map_err(|e| ClipError::ClipsDir(self.clips_dir.clone(), e))?;
        let path = self.unique_clip_path();
        let written = write_mp4_with_tracks(
            &path,
            &info.video,
            &info.parameter_sets,
            &slice.video,
            &tracks,
        )?;
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        Ok(SavedClip {
            path,
            duration: written,
            packets: slice.video.len(),
            audio_packets: if audio_info.is_some() {
                slice.audio.len()
            } else {
                0
            },
            mic_packets: if mic_info.is_some() {
                slice.mic.len()
            } else {
                0
            },
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
