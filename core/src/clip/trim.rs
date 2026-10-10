//! Editing a saved clip: look inside it ([`probe`]), draw its audio
//! ([`audio_peaks`]) and cut a piece out of it ([`trim_clip`]) — all in Rust,
//! with no ffmpeg and no re-encoding.
//!
//! A cut is a stream copy, so the new video has to begin on a keyframe. The
//! requested start usually isn't one (keyframes are seconds apart), so the
//! copy starts at the keyframe *before* it and the extra frames are hidden by
//! an edit list on the video track (see [`super::edit_list`]): the start is
//! frame accurate and the cost is a little extra data. Audio is copied packet
//! by packet and re-aligned to the new start by the same muxer that wrote the
//! original clip, so a clip can be trimmed again and again.
//!
//! Only clips written by [`super::ClipService`] (H.264 without B-frames plus
//! AAC-LC) are understood; anything else is refused with
//! [`TrimError::Unsupported`] rather than cut wrongly.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use mp4::{Mp4Reader, Mp4Track, TrackType};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecParameters, DecoderOptions, CODEC_TYPE_AAC};
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::Packet;
use thiserror::Error;

use super::mp4mux::{write_mp4_cut, AudioParams, MuxError, VideoParams};
use crate::audio::{frames_to_duration, AudioPacket};
use crate::encode::{Codec, EncodedPacket, ParameterSets};

/// Resolution of the waveform handed to the UI.
pub const PEAKS_PER_SECOND: u32 = 50;

/// How far from the cut the audio packets are taken: enough for the AAC
/// decoder to warm up, and for the muxer to pick its starting packet.
const AUDIO_MARGIN: Duration = Duration::from_millis(250);
/// Offset added to every timestamp handed to the muxer so audio that starts
/// before the video start never needs a negative `Duration`.
const ORIGIN: Duration = Duration::from_millis(500);
/// Frames within this distance of a requested time count as being at it, so
/// `12.3 s` picks the frame the UI shows at `12.3 s` despite float noise.
const TOLERANCE_NS: i128 = 1_000_000;

#[derive(Debug, Error)]
pub enum TrimError {
    #[error("no se pudo leer el clip: {0}")]
    Io(#[from] std::io::Error),
    #[error("el archivo no es un mp4 válido: {0}")]
    Mp4(#[from] mp4::Error),
    #[error("el clip no tiene pista de video")]
    NoVideo,
    #[error("este clip no se puede editar: {0}")]
    Unsupported(String),
    #[error("la selección está vacía")]
    EmptySelection,
    #[error("el clip no tiene la pista de audio {0}")]
    NoSuchAudioTrack(usize),
    #[error(transparent)]
    Mux(#[from] MuxError),
    #[error("no se pudo decodificar el audio: {0}")]
    Decode(String),
}

/// What the editor needs to know about a clip.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipProbe {
    /// Length of what plays (hidden lead-in excluded).
    pub duration: Duration,
    pub width: u32,
    pub height: u32,
    /// Frames per second, for stepping frame by frame.
    pub fps: f64,
    pub frames: usize,
    pub audio: Vec<AudioTrackInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrackInfo {
    /// Position among the audio tracks, in file order: the track players pick
    /// by default is `0`.
    pub index: usize,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: Duration,
}

/// A cut to make: the piece of the clip between `start` and `end`
/// (presentation time, `end` excluded) and which audio tracks to keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimSpec {
    pub start: Duration,
    pub end: Duration,
    /// Audio tracks to carry over (indices of [`AudioTrackInfo::index`]); the
    /// ones left out are dropped from the copy.
    pub keep_audio: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrimmedClip {
    pub duration: Duration,
    pub video_frames: usize,
    pub audio_tracks: usize,
    pub bytes: u64,
}

/// Loudness of an audio track over time, for drawing a waveform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peaks {
    pub per_second: u32,
    /// Peak amplitude (0 = silence, 255 = full scale) of each
    /// `1 / per_second` of the *played* timeline, starting at 0.
    pub values: Vec<u8>,
}

// ---- reading the container ------------------------------------------------

/// How a track's media time maps to the clip's playback time.
#[derive(Debug, Clone, Copy)]
struct Timing {
    timescale: u32,
    /// Media ticks hidden at the start (lead-in, encoder delay).
    skip: i64,
    /// Silence before the track starts, in nanoseconds.
    delay_ns: i128,
}

impl Timing {
    /// Playback time, in nanoseconds, of media time `ticks`.
    fn presentation_ns(&self, ticks: i64) -> i128 {
        i128::from(ticks - self.skip) * 1_000_000_000 / i128::from(self.timescale) + self.delay_ns
    }
}

/// Start, length and keyframe flag of every sample of a track.
struct SampleTable {
    starts: Vec<i64>,
    durations: Vec<u32>,
    sync: Vec<bool>,
}

impl SampleTable {
    fn len(&self) -> usize {
        self.starts.len()
    }

    /// Media time where the last sample ends.
    fn end(&self) -> i64 {
        match (self.starts.last(), self.durations.last()) {
            (Some(&start), Some(&duration)) => start + i64::from(duration),
            _ => 0,
        }
    }
}

struct VideoTrack {
    id: u32,
    width: u32,
    height: u32,
    timing: Timing,
    table: SampleTable,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

struct AudioTrack {
    id: u32,
    sample_rate: u32,
    channels: u16,
    bitrate: u32,
    timing: Timing,
    table: SampleTable,
}

struct Source {
    reader: Mp4Reader<BufReader<File>>,
    video: VideoTrack,
    audio: Vec<AudioTrack>,
}

fn unsupported<T>(why: &str) -> Result<T, TrimError> {
    Err(TrimError::Unsupported(why.to_string()))
}

fn timing_of(track: &Mp4Track, movie_timescale: u32) -> Result<Timing, TrimError> {
    let entries = track
        .trak
        .edts
        .as_ref()
        .and_then(|e| e.elst.as_ref())
        .map(|e| e.entries.as_slice())
        .unwrap_or(&[]);
    // Version 0 stores -1 as u32::MAX, version 1 as u64::MAX.
    let empty = |media_time: u64| media_time == u64::from(u32::MAX) || media_time == u64::MAX;
    let (delay, skip) = match entries {
        [] => (0, 0),
        [e] if !empty(e.media_time) => (0, e.media_time as i64),
        [gap, e] if empty(gap.media_time) && !empty(e.media_time) => {
            (gap.segment_duration, e.media_time as i64)
        }
        _ => return unsupported("tiene una lista de ediciones compleja"),
    };
    Ok(Timing {
        timescale: track.timescale().max(1),
        skip,
        delay_ns: i128::from(delay) * 1_000_000_000 / i128::from(movie_timescale.max(1)),
    })
}

fn sample_table_of(track: &Mp4Track) -> Result<SampleTable, TrimError> {
    let stbl = &track.trak.mdia.minf.stbl;
    if stbl
        .ctts
        .as_ref()
        .is_some_and(|c| c.entries.iter().any(|e| e.sample_offset != 0))
    {
        return unsupported("el video usa fotogramas B");
    }
    let expected = track.sample_count() as usize;
    let mut starts = Vec::with_capacity(expected);
    let mut durations = Vec::with_capacity(expected);
    let mut time = 0i64;
    for entry in &stbl.stts.entries {
        if starts.len() + entry.sample_count as usize > expected {
            return unsupported("la tabla de muestras es inconsistente");
        }
        for _ in 0..entry.sample_count {
            starts.push(time);
            durations.push(entry.sample_delta);
            time += i64::from(entry.sample_delta);
        }
    }
    if starts.len() != expected {
        return unsupported("la tabla de muestras es inconsistente");
    }
    let sync = match &stbl.stss {
        Some(stss) => {
            let mut sync = vec![false; expected];
            for &id in &stss.entries {
                if let Some(slot) = (id as usize).checked_sub(1).and_then(|i| sync.get_mut(i)) {
                    *slot = true;
                }
            }
            sync
        }
        None => vec![true; expected],
    };
    Ok(SampleTable {
        starts,
        durations,
        sync,
    })
}

fn sample_rate_of(track: &Mp4Track) -> Result<u32, TrimError> {
    Ok(track.sample_freq_index()?.freq())
}

fn channels_of(track: &Mp4Track) -> Result<u16, TrimError> {
    match track.channel_config()? {
        mp4::ChannelConfig::Mono => Ok(1),
        mp4::ChannelConfig::Stereo => Ok(2),
        _ => unsupported("el audio no es mono ni estéreo"),
    }
}

impl Source {
    fn open(path: &Path) -> Result<Self, TrimError> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        let reader = Mp4Reader::read_header(BufReader::new(file), size)?;
        if reader.is_fragmented() {
            return unsupported("es un mp4 fragmentado");
        }
        let movie_timescale = reader.timescale();

        let mut ids: Vec<u32> = reader.tracks().keys().copied().collect();
        ids.sort_unstable();
        let mut video = None;
        let mut audio = Vec::new();
        for id in ids {
            let track = &reader.tracks()[&id];
            match track.track_type() {
                Ok(TrackType::Video) if video.is_none() => {
                    if !matches!(track.media_type(), Ok(mp4::MediaType::H264)) {
                        return unsupported("el video no es H.264");
                    }
                    let timing = timing_of(track, movie_timescale)?;
                    if timing.delay_ns != 0 {
                        return unsupported("el video empieza con un silencio");
                    }
                    video = Some(VideoTrack {
                        id,
                        width: u32::from(track.width()),
                        height: u32::from(track.height()),
                        timing,
                        table: sample_table_of(track)?,
                        sps: track.sequence_parameter_set()?.to_vec(),
                        pps: track.picture_parameter_set()?.to_vec(),
                    });
                }
                Ok(TrackType::Audio) => {
                    audio.push(AudioTrack {
                        id,
                        sample_rate: sample_rate_of(track)?,
                        channels: channels_of(track)?,
                        bitrate: track.bitrate(),
                        timing: timing_of(track, movie_timescale)?,
                        table: sample_table_of(track)?,
                    });
                }
                _ => {}
            }
        }
        let video = video.ok_or(TrimError::NoVideo)?;
        if video.table.len() == 0 {
            return Err(TrimError::NoVideo);
        }
        Ok(Self {
            reader,
            video,
            audio,
        })
    }

    fn read(&mut self, track_id: u32, index: usize) -> Result<Vec<u8>, TrimError> {
        match self.reader.read_sample(track_id, index as u32 + 1)? {
            Some(sample) => Ok(sample.bytes.to_vec()),
            None => unsupported("falta una muestra del archivo"),
        }
    }
}

fn nanos_to_duration(ns: i128) -> Duration {
    Duration::from_nanos(ns.clamp(0, i128::from(u64::MAX)) as u64)
}

/// `ticks` of a `timescale` clock as a `Duration`, rounded *up* so that
/// converting back (which truncates) gives exactly `ticks` again.
fn ticks_to_duration(ticks: i64, timescale: u32) -> Duration {
    let ts = i128::from(timescale.max(1));
    nanos_to_duration((i128::from(ticks) * 1_000_000_000 + ts - 1) / ts)
}

// ---- probe ------------------------------------------------------------------

/// Read the structure of the clip at `path`.
pub fn probe(path: &Path) -> Result<ClipProbe, TrimError> {
    let source = Source::open(path)?;
    let video = &source.video;
    let first_visible = first_visible(video);
    let frames = video.table.len() - first_visible;
    let duration_ns = video.timing.presentation_ns(video.table.end());
    let last_frame_ticks = i64::from(*video.table.durations.last().expect("non-empty"));
    let fps = if last_frame_ticks > 0 {
        f64::from(video.timing.timescale) / last_frame_ticks as f64
    } else {
        0.0
    };
    Ok(ClipProbe {
        duration: nanos_to_duration(duration_ns),
        width: video.width,
        height: video.height,
        fps,
        frames,
        audio: source
            .audio
            .iter()
            .enumerate()
            .map(|(index, a)| AudioTrackInfo {
                index,
                sample_rate: a.sample_rate,
                channels: a.channels,
                duration: nanos_to_duration(a.timing.presentation_ns(a.table.end())),
            })
            .collect(),
    })
}

/// Index of the first sample that plays (earlier ones are hidden lead-in).
fn first_visible(video: &VideoTrack) -> usize {
    video
        .table
        .starts
        .iter()
        .position(|&t| t >= video.timing.skip)
        .unwrap_or(video.table.len() - 1)
}

// ---- trim -------------------------------------------------------------------

/// Write the piece of `src` described by `spec` to `dst` (a new file). The
/// original is never touched; on failure no partial file is left behind.
pub fn trim_clip(src: &Path, dst: &Path, spec: &TrimSpec) -> Result<TrimmedClip, TrimError> {
    let result = trim_inner(src, dst, spec);
    if result.is_err() {
        std::fs::remove_file(dst).ok();
    }
    result
}

fn trim_inner(src: &Path, dst: &Path, spec: &TrimSpec) -> Result<TrimmedClip, TrimError> {
    let mut source = Source::open(src)?;
    let mut keep = spec.keep_audio.clone();
    keep.sort_unstable();
    keep.dedup();
    if let Some(&missing) = keep.iter().find(|&&i| i >= source.audio.len()) {
        return Err(TrimError::NoSuchAudioTrack(missing));
    }

    // -- which video samples
    let video = &source.video;
    let timing = video.timing;
    let starts = video.table.starts.clone();
    let last = video.table.len() - 1;
    let play_ns = |index: usize| timing.presentation_ns(starts[index]);
    let first = first_visible(video);
    let start_ns = spec.start.as_nanos() as i128;
    let end_ns = spec.end.as_nanos() as i128;
    if end_ns <= start_ns || start_ns >= timing.presentation_ns(video.table.end()) {
        return Err(TrimError::EmptySelection);
    }
    // The frame on screen at `start`, and the last frame that begins before
    // `end`. A selection shorter than a frame still keeps that frame.
    let visible_start = (first..=last)
        .rev()
        .find(|&i| play_ns(i) <= start_ns + TOLERANCE_NS)
        .unwrap_or(first);
    let visible_end = (visible_start..=last)
        .rev()
        .find(|&i| play_ns(i) < end_ns - TOLERANCE_NS)
        .unwrap_or(visible_start);
    // Decoding has to begin on a keyframe at or before the first shown frame.
    let decode_start = (0..=visible_start)
        .rev()
        .find(|&i| video.table.sync[i])
        .ok_or_else(|| {
            TrimError::Unsupported("no hay un fotograma clave antes del inicio".into())
        })?;

    let video_id = video.id;
    let sync = video.table.sync[decode_start..=visible_end].to_vec();
    let params = VideoParams {
        codec: Codec::H264,
        width: video.width,
        height: video.height,
        fps: {
            let ticks = i64::from(video.table.durations[last]).max(1);
            ((f64::from(timing.timescale) / ticks as f64).round() as u32).max(1)
        },
    };
    let sets = ParameterSets {
        sps: vec![video.sps.clone()],
        pps: vec![video.pps.clone()],
    };
    let last_shown_end = starts[visible_end] + i64::from(video.table.durations[visible_end]);

    // Playback time of the first decoded sample, to express everything else
    // against it.
    let decode_start_ns = play_ns(decode_start);
    let visible_start_rel = play_ns(visible_start) - decode_start_ns;
    let end_rel = timing.presentation_ns(last_shown_end) - decode_start_ns;

    // -- video packets, on a clock that starts at `ORIGIN`
    let mut packets = Vec::with_capacity(sync.len());
    for (n, &is_keyframe) in sync.iter().enumerate() {
        let index = decode_start + n;
        let data = source.read(video_id, index)?;
        let pts =
            ORIGIN + ticks_to_duration(starts[index] - starts[decode_start], timing.timescale);
        packets.push(EncodedPacket {
            data,
            pts,
            dts: pts,
            is_keyframe,
        });
    }

    // -- audio packets of the kept tracks, on the same clock
    let mut audio: Vec<(AudioParams, Vec<AudioPacket>)> = Vec::new();
    for &index in &keep {
        let track = &source.audio[index];
        let (id, rate, timing) = (track.id, track.sample_rate, track.timing);
        let selected: Vec<(usize, i128, u32)> = (0..track.table.len())
            .filter_map(|k| {
                let at = timing.presentation_ns(track.table.starts[k]) - decode_start_ns;
                let length =
                    i128::from(track.table.durations[k]) * 1_000_000_000 / i128::from(rate.max(1));
                let overlaps = at + length >= visible_start_rel - AUDIO_MARGIN.as_nanos() as i128
                    && at < end_rel;
                overlaps.then_some((k, at, track.table.durations[k]))
            })
            .collect();
        let params = AudioParams {
            sample_rate: rate,
            channels: track.channels,
            bitrate: if track.bitrate > 0 {
                track.bitrate
            } else {
                128_000
            },
            // The source's edit list already absorbed the encoder delay: the
            // timestamps below are playback times, not encoder input times.
            priming_frames: 0,
        };
        let mut packets = Vec::with_capacity(selected.len());
        for (k, at, ticks) in selected {
            packets.push(AudioPacket {
                data: source.read(id, k)?,
                pts: ORIGIN + nanos_to_duration(at),
                duration: frames_to_duration(u64::from(ticks), rate),
            });
        }
        audio.push((params, packets));
    }

    let tracks: Vec<(&AudioParams, &[AudioPacket])> =
        audio.iter().map(|(p, a)| (p, a.as_slice())).collect();
    let duration = write_mp4_cut(
        dst,
        &params,
        &sets,
        &packets,
        visible_start - decode_start,
        &tracks,
    )?;
    Ok(TrimmedClip {
        duration,
        video_frames: visible_end - visible_start + 1,
        audio_tracks: audio.iter().filter(|(_, a)| !a.is_empty()).count(),
        bytes: std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0),
    })
}

// ---- waveform ---------------------------------------------------------------

/// MPEG-4 frequency index for the AudioSpecificConfig.
fn frequency_index(rate: u32) -> Option<u16> {
    Some(match rate {
        96_000 => 0,
        88_200 => 1,
        64_000 => 2,
        48_000 => 3,
        44_100 => 4,
        32_000 => 5,
        24_000 => 6,
        22_050 => 7,
        16_000 => 8,
        12_000 => 9,
        11_025 => 10,
        8_000 => 11,
        7_350 => 12,
        _ => return None,
    })
}

/// Peak loudness over time of the audio track `track` of the clip at `path`.
///
/// Decodes the whole track (AAC-LC, in pure Rust) and lays it out on the
/// played timeline, so bucket `i` is what is heard during
/// `i / PEAKS_PER_SECOND` seconds of the clip.
pub fn audio_peaks(path: &Path, track: usize) -> Result<Peaks, TrimError> {
    let mut source = Source::open(path)?;
    let audio = source
        .audio
        .get(track)
        .ok_or(TrimError::NoSuchAudioTrack(track))?;
    let (id, rate, channels, timing) = (audio.id, audio.sample_rate, audio.channels, audio.timing);
    let table_len = audio.table.len();
    let starts = audio.table.starts.clone();
    let durations = audio.table.durations.clone();
    let length_ns = timing.presentation_ns(audio.table.end()).max(0);
    let buckets = (length_ns * i128::from(PEAKS_PER_SECOND) / 1_000_000_000) as usize + 1;
    let mut values = vec![0u8; buckets];

    let index = frequency_index(rate)
        .ok_or_else(|| TrimError::Unsupported(format!("audio de {rate} Hz")))?;
    let config: u16 = (2 << 11) | (index << 7) | (channels << 3); // AAC-LC
    let mut params = CodecParameters::new();
    params
        .for_codec(CODEC_TYPE_AAC)
        .with_sample_rate(rate)
        .with_extra_data(config.to_be_bytes().to_vec().into_boxed_slice());
    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| TrimError::Decode(e.to_string()))?;

    let delay_samples = timing.delay_ns * i128::from(rate) / 1_000_000_000;
    let mut samples: Option<SampleBuffer<f32>> = None;
    for k in 0..table_len {
        let bytes = source.read(id, k)?;
        let packet = Packet::new_from_slice(0, starts[k] as u64, u64::from(durations[k]), &bytes);
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // A damaged packet is silence in the picture, not a failure.
            Err(DecodeError::DecodeError(_)) => continue,
            Err(e) => return Err(TrimError::Decode(e.to_string())),
        };
        let spec = *decoded.spec();
        let buffer =
            samples.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        buffer.copy_interleaved_ref(decoded);
        let width = spec.channels.count().max(1);
        for (offset, frame) in buffer.samples().chunks(width).enumerate() {
            let position = starts[k] - timing.skip + offset as i64;
            let at = i128::from(position) + delay_samples;
            if at < 0 {
                continue;
            }
            let bucket = (at * i128::from(PEAKS_PER_SECOND) / i128::from(rate)) as usize;
            let Some(slot) = values.get_mut(bucket) else {
                continue;
            };
            let peak = frame.iter().fold(0f32, |m, s| m.max(s.abs()));
            *slot = (*slot).max((peak.min(1.0) * 255.0).round() as u8);
        }
    }
    Ok(Peaks {
        per_second: PEAKS_PER_SECOND,
        values,
    })
}

#[cfg(test)]
mod tests {
    // Tests that need whole files live in `core/tests/clip_trim.rs`.
    use super::*;

    #[test]
    fn ticks_round_trip_through_a_duration() {
        for ticks in [0i64, 1, 1500, 2999, 3000, 90_000, 123_457, 10_000_000] {
            let d = ticks_to_duration(ticks, 90_000);
            let back = d.as_nanos() * 90_000 / 1_000_000_000;
            assert_eq!(back as i64, ticks, "{ticks} ticks via {d:?}");
        }
    }

    #[test]
    fn timing_maps_media_time_to_playback_time() {
        let t = Timing {
            timescale: 48_000,
            skip: 1_088,
            delay_ns: 0,
        };
        assert_eq!(t.presentation_ns(1_088), 0);
        assert_eq!(t.presentation_ns(1_088 + 48_000), 1_000_000_000);
        assert!(t.presentation_ns(0) < 0, "the skipped start is before 0");

        let late = Timing {
            timescale: 48_000,
            skip: 0,
            delay_ns: 250_000_000,
        };
        assert_eq!(late.presentation_ns(0), 250_000_000);
    }

    #[test]
    fn frequency_indices_match_the_standard_table() {
        assert_eq!(frequency_index(48_000), Some(3));
        assert_eq!(frequency_index(44_100), Some(4));
        assert_eq!(frequency_index(12_345), None);
    }

    #[test]
    fn a_missing_file_is_an_io_error() {
        let err = probe(Path::new("/definitely/not/here.mp4")).unwrap_err();
        assert!(matches!(err, TrimError::Io(_)), "{err}");
    }
}
