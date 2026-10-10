//! Mux encoded H.264 packets (and, optionally, AAC audio) into an .mp4 file —
//! a container write only, never a re-encode. Video packets must already be
//! AVCC (4-byte length-prefixed), which is what [`crate::encode`] encoders
//! emit; audio packets are raw AAC-LC access units.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::time::Duration;

use thiserror::Error;

use super::edit_list::{add_audio_edit_lists, EditEntry};
use crate::audio::{AudioConfig, AudioPacket};
use crate::encode::{Codec, EncodedPacket, ParameterSets};

/// Track timescale: 90 kHz, the conventional choice for video (exactly
/// divides 24/25/30/60 fps).
const TRACK_TIMESCALE: u32 = 90_000;

#[derive(Debug, Error)]
pub enum MuxError {
    #[error("no packets to mux")]
    NoPackets,
    #[error("first packet must be a keyframe (got a non-sync frame)")]
    FirstPacketNotKeyframe,
    #[error("missing SPS/PPS parameter sets")]
    MissingParameterSets,
    #[error("codec {0:?} not supported by the muxer yet")]
    UnsupportedCodec(Codec),
    #[error("audio format not supported by the muxer: {0}")]
    UnsupportedAudio(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("mp4 write error: {0}")]
    Mp4(#[from] mp4::Error),
}

/// Video properties the muxer needs (a subset of `EncoderConfig`).
#[derive(Debug, Clone)]
pub struct VideoParams {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    /// Nominal fps; used only for the last sample's duration.
    pub fps: u32,
}

/// Audio properties the muxer needs (what the AAC encoder was configured
/// with).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioParams {
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: u32,
    /// Encoder delay in samples (see `AudioEncoder::priming_frames`); drives
    /// the edit list that aligns the audio with the video.
    pub priming_frames: u32,
}

impl From<AudioConfig> for AudioParams {
    /// No encoder delay; set `priming_frames` for a real AAC encoder.
    fn from(c: AudioConfig) -> Self {
        Self {
            sample_rate: c.sample_rate,
            channels: c.channels,
            bitrate: c.bitrate,
            priming_frames: 0,
        }
    }
}

/// An AAC decoder needs one full block of input before its output is valid
/// (MDCT overlap), so playback must start at least this far into the decode.
const DECODER_WARMUP_FRAMES: i64 = 1024;

/// Which audio packets to write and what the edit list must say.
#[derive(Debug, PartialEq, Eq)]
struct AudioPlan {
    /// Index of the first packet to write.
    first: usize,
    /// Edit list entries; empty when the track plays from its first sample.
    edits: Vec<EditEntry>,
}

fn frames_at(t: Duration, sample_rate: u32) -> i64 {
    ((t.as_nanos() * u128::from(sample_rate) + 500_000_000) / 1_000_000_000) as i64
}

/// Sample duration in the audio track's timescale (= the sample rate).
fn audio_ticks(packet: &AudioPacket, sample_rate: u32) -> u32 {
    let ticks =
        (packet.duration.as_nanos() * u128::from(sample_rate) + 500_000_000) / 1_000_000_000;
    ticks.max(1) as u32
}

/// Decide where the audio track starts so that, after the decoder's delay
/// and warm-up, the first sound played lines up with `video_start`.
///
/// Position `p` of the decoded stream (counting from the first written
/// packet `j`) is the input sample `pts_j + p - priming`, so reaching the
/// video start `x0` means skipping `p0 = x0 - pts_j + priming` decoded
/// samples. We take the latest packet that still leaves `p0` at or above the
/// decoder warm-up. If the audio only begins after the video does, the
/// negative `p0` becomes leading silence (an empty edit) instead.
///
/// Returns `None` when nothing audible would remain.
fn plan_audio(
    packets: &[AudioPacket],
    params: &AudioParams,
    video_start: Duration,
) -> Option<AudioPlan> {
    let rate = params.sample_rate;
    let x0 = frames_at(video_start, rate);
    let priming = i64::from(params.priming_frames);
    let first = packets
        .iter()
        .rposition(|p| frames_at(p.pts, rate) <= x0 + priming - DECODER_WARMUP_FRAMES)
        .unwrap_or(0);

    let p0 = x0 - frames_at(packets.get(first)?.pts, rate) + priming;
    let (skip, delay) = if p0 >= 0 { (p0, 0) } else { (0, -p0) };
    let media: i64 = packets[first..]
        .iter()
        .map(|p| i64::from(audio_ticks(p, rate)))
        .sum();
    let play = media - skip;
    if play <= 0 {
        return None;
    }

    let to_movie = |frames: i64| (frames * i64::from(TRACK_TIMESCALE) / i64::from(rate)) as u32;
    let mut edits = Vec::new();
    if skip > 0 || delay > 0 {
        if delay > 0 {
            edits.push(EditEntry {
                segment_duration: to_movie(delay),
                media_time: -1,
            });
        }
        edits.push(EditEntry {
            segment_duration: to_movie(play),
            media_time: skip as i32,
        });
    }
    Some(AudioPlan { first, edits })
}

const VIDEO_TRACK_ID: u32 = 1;

fn aac_config(audio: &AudioParams) -> Result<mp4::AacConfig, MuxError> {
    use mp4::{ChannelConfig, SampleFreqIndex};
    let freq_index = match audio.sample_rate {
        96_000 => SampleFreqIndex::Freq96000,
        88_200 => SampleFreqIndex::Freq88200,
        64_000 => SampleFreqIndex::Freq64000,
        48_000 => SampleFreqIndex::Freq48000,
        44_100 => SampleFreqIndex::Freq44100,
        32_000 => SampleFreqIndex::Freq32000,
        24_000 => SampleFreqIndex::Freq24000,
        22_050 => SampleFreqIndex::Freq22050,
        16_000 => SampleFreqIndex::Freq16000,
        other => {
            return Err(MuxError::UnsupportedAudio(format!(
                "{other} Hz sample rate"
            )))
        }
    };
    let chan_conf = match audio.channels {
        1 => ChannelConfig::Mono,
        2 => ChannelConfig::Stereo,
        other => return Err(MuxError::UnsupportedAudio(format!("{other} channels"))),
    };
    Ok(mp4::AacConfig {
        bitrate: audio.bitrate,
        profile: mp4::AudioObjectType::AacLowComplexity,
        freq_index,
        chan_conf,
    })
}

/// Write `packets` to `path` as a single-video-track .mp4.
///
/// Packets must be in decode order with monotonic pts, starting with a
/// keyframe (the GOP-aligned ring buffer guarantees this). Returns the
/// duration of the written track.
pub fn write_mp4(
    path: &Path,
    params: &VideoParams,
    parameter_sets: &ParameterSets,
    packets: &[EncodedPacket],
) -> Result<Duration, MuxError> {
    write_mp4_with_audio(path, params, parameter_sets, packets, None)
}

/// [`write_mp4`] plus an optional AAC track.
///
/// Audio packets must be contiguous (each starts where the previous ended —
/// the muxer stores durations, not gaps) and are aligned to the video by
/// their `pts`; the first one should start at or just after the first video
/// packet. An empty audio slice writes a video-only file. Samples of both
/// tracks are interleaved in time order so players can stream the file.
pub fn write_mp4_with_audio(
    path: &Path,
    params: &VideoParams,
    parameter_sets: &ParameterSets,
    packets: &[EncodedPacket],
    audio: Option<(&AudioParams, &[AudioPacket])>,
) -> Result<Duration, MuxError> {
    write_mp4_with_tracks(path, params, parameter_sets, packets, audio.as_slice())
}

/// [`write_mp4`] plus any number of AAC tracks (game audio, voice, ...), in
/// the order given: the first one is the track players pick by default.
/// Each track is aligned to the video on its own, with its own edit list; a
/// track with nothing audible left is left out.
pub fn write_mp4_with_tracks(
    path: &Path,
    params: &VideoParams,
    parameter_sets: &ParameterSets,
    packets: &[EncodedPacket],
    audio: &[(&AudioParams, &[AudioPacket])],
) -> Result<Duration, MuxError> {
    let first = packets.first().ok_or(MuxError::NoPackets)?;
    // Validate every requested audio format up front, then decide which
    // tracks have something audible and how each is aligned.
    let mut tracks: Vec<AudioTrack<'_>> = Vec::new();
    for (audio_params, audio_packets) in audio {
        if audio_packets.is_empty() {
            continue;
        }
        let conf = aac_config(audio_params)?;
        if let Some(plan) = plan_audio(audio_packets, audio_params, first.pts) {
            tracks.push(AudioTrack {
                params: audio_params,
                packets: &audio_packets[plan.first..],
                edits: plan.edits,
                conf,
            });
        }
    }
    if !first.is_keyframe {
        return Err(MuxError::FirstPacketNotKeyframe);
    }
    if params.codec != Codec::H264 {
        // HEVC needs an hvcC box the `mp4` crate can't write yet.
        return Err(MuxError::UnsupportedCodec(params.codec));
    }
    let (sps, pps) = match (parameter_sets.sps.first(), parameter_sets.pps.first()) {
        (Some(sps), Some(pps)) if !sps.is_empty() && !pps.is_empty() => (sps, pps),
        _ => return Err(MuxError::MissingParameterSets),
    };

    let file = BufWriter::new(File::create(path)?);
    let mut writer = mp4::Mp4Writer::write_start(
        file,
        &mp4::Mp4Config {
            major_brand: (*b"isom").into(),
            minor_version: 512,
            compatible_brands: vec![
                (*b"isom").into(),
                (*b"iso2").into(),
                (*b"avc1").into(),
                (*b"mp41").into(),
            ],
            // Same timescale as the track: the writer converts per-sample
            // durations movie-side by integer division, so a mismatched
            // timescale (e.g. 1000) accumulates truncation error.
            timescale: TRACK_TIMESCALE,
        },
    )?;

    writer.add_track(&mp4::TrackConfig {
        track_type: mp4::TrackType::Video,
        timescale: TRACK_TIMESCALE,
        language: "und".to_string(),
        media_conf: mp4::MediaConfig::AvcConfig(mp4::AvcConfig {
            width: params.width as u16,
            height: params.height as u16,
            seq_param_set: sps.clone(),
            pic_param_set: pps.clone(),
        }),
    })?;
    for track in &tracks {
        writer.add_track(&mp4::TrackConfig {
            track_type: mp4::TrackType::Audio,
            timescale: track.params.sample_rate,
            language: "und".to_string(),
            media_conf: mp4::MediaConfig::AacConfig(track.conf.clone()),
        })?;
    }

    let base_pts = first.pts;
    let frame_ticks = (TRACK_TIMESCALE / params.fps.max(1)).max(1);
    let video_sample = |i: usize| {
        let packet = &packets[i];
        let start_time = to_ticks(packet.pts - base_pts);
        // Sample duration = gap to the next packet; nominal 1/fps for the
        // last one.
        let duration = match packets.get(i + 1) {
            Some(next) => (to_ticks(next.pts - base_pts) - start_time).max(1) as u32,
            None => frame_ticks,
        };
        mp4::Mp4Sample {
            start_time,
            duration,
            rendering_offset: 0,
            is_sync: packet.is_keyframe,
            bytes: bytes::Bytes::copy_from_slice(&packet.data),
        }
    };

    // Merge all streams by presentation time (video first on ties) so the
    // file interleaves tracks and players can stream it.
    let mut video_next = 0usize;
    let mut next: Vec<usize> = vec![0; tracks.len()];
    let mut ticks_written: Vec<u64> = vec![0; tracks.len()];
    loop {
        let mut due: Option<(Duration, Option<usize>)> = None; // (time, None = video)
        if video_next < packets.len() {
            due = Some((packets[video_next].pts - base_pts, None));
        }
        for (k, track) in tracks.iter().enumerate() {
            if let Some(packet) = track.packets.get(next[k]) {
                let time = packet.pts.saturating_sub(base_pts);
                if due.is_none_or(|(best, _)| time < best) {
                    due = Some((time, Some(k)));
                }
            }
        }
        match due {
            None => break,
            Some((_, None)) => {
                writer.write_sample(VIDEO_TRACK_ID, &video_sample(video_next))?;
                video_next += 1;
            }
            Some((_, Some(k))) => {
                let track = &tracks[k];
                let packet = &track.packets[next[k]];
                let sample = mp4::Mp4Sample {
                    start_time: ticks_written[k],
                    duration: audio_ticks(packet, track.params.sample_rate),
                    rendering_offset: 0,
                    is_sync: true, // every AAC frame decodes on its own
                    bytes: bytes::Bytes::copy_from_slice(&packet.data),
                };
                ticks_written[k] += u64::from(sample.duration);
                writer.write_sample(VIDEO_TRACK_ID + 1 + k as u32, &sample)?;
                next[k] += 1;
            }
        }
    }
    writer.write_end()?;
    let mut file = writer.into_writer();
    std::io::Write::flush(&mut file)?;
    drop(file);

    let edits: Vec<Vec<EditEntry>> = tracks.iter().map(|t| t.edits.clone()).collect();
    if edits.iter().any(|e| !e.is_empty()) {
        add_audio_edit_lists(path, &edits)?;
    }

    let last = packets.last().expect("checked non-empty");
    Ok(last.pts - base_pts + Duration::from_secs(1) / params.fps.max(1))
}

/// One audio track about to be written.
struct AudioTrack<'a> {
    params: &'a AudioParams,
    /// Packets from the planned starting point on.
    packets: &'a [AudioPacket],
    edits: Vec<EditEntry>,
    conf: mp4::AacConfig,
}

fn to_ticks(t: Duration) -> u64 {
    (t.as_nanos() * u128::from(TRACK_TIMESCALE) / 1_000_000_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn fake_packets(count: u64, fps: u32, gop: u64) -> Vec<EncodedPacket> {
        (0..count)
            .map(|n| EncodedPacket {
                // AVCC-shaped payload: one fake NAL of 16 bytes.
                data: {
                    let mut d = vec![0, 0, 0, 16];
                    d.extend_from_slice(&[if n % gop == 0 { 0x65 } else { 0x41 }; 16]);
                    d
                },
                pts: Duration::from_nanos(n * 1_000_000_000 / u64::from(fps)),
                dts: Duration::from_nanos(n * 1_000_000_000 / u64::from(fps)),
                is_keyframe: n % gop == 0,
            })
            .collect()
    }

    fn fake_params() -> (VideoParams, ParameterSets) {
        (
            VideoParams {
                codec: Codec::H264,
                width: 64,
                height: 48,
                fps: 60,
            },
            ParameterSets {
                // Minimal plausible SPS/PPS payloads (content is opaque to
                // the muxer; only NAL types matter).
                sps: vec![vec![0x67, 0x42, 0xc0, 0x0d, 0x96, 0x54, 0x0a, 0x0f, 0xc8]],
                pps: vec![vec![0x68, 0xce, 0x3c, 0x80]],
            },
        )
    }

    #[test]
    fn writes_readable_mp4_with_expected_duration() {
        let dir = std::env::temp_dir().join("clipforge-mp4mux-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake.mp4");
        let (params, sets) = fake_params();
        let packets = fake_packets(120, 60, 30);

        let duration = write_mp4(&path, &params, &sets, &packets).unwrap();
        let error = duration.abs_diff(Duration::from_secs(2));
        assert!(
            error < Duration::from_millis(2),
            "muxer duration {duration:?} not ~2 s"
        );

        let size = std::fs::metadata(&path).unwrap().len();
        let reader = std::io::BufReader::new(File::open(&path).unwrap());
        let mp4 = mp4::Mp4Reader::read_header(reader, size).unwrap();
        assert_eq!(mp4.tracks().len(), 1);
        let track = mp4.tracks().values().next().unwrap();
        assert_eq!(track.width(), 64);
        assert_eq!(track.height(), 48);
        assert_eq!(track.sample_count(), 120);
        // Container duration within one frame of 2 s.
        let container = mp4.duration();
        assert!(
            container >= Duration::from_millis(1980) && container <= Duration::from_millis(2020),
            "container duration {container:?}"
        );
        std::fs::remove_file(&path).ok();
    }

    /// `count` contiguous 1024-sample AAC packets at 48 kHz starting at `start`.
    fn fake_audio(count: u64, start: Duration) -> Vec<AudioPacket> {
        (0..count)
            .map(|k| AudioPacket {
                data: vec![k as u8; 300],
                pts: start + crate::audio::frames_to_duration(k * 1024, 48_000),
                duration: crate::audio::frames_to_duration(1024, 48_000),
            })
            .collect()
    }

    fn audio_params() -> AudioParams {
        AudioParams {
            sample_rate: 48_000,
            channels: 2,
            bitrate: 128_000,
            priming_frames: 0,
        }
    }

    fn at_samples(n: u64) -> Duration {
        crate::audio::frames_to_duration(n, 48_000)
    }

    fn plan_for(
        first_packet_at: u64,
        count: u64,
        priming: u32,
        video_start: u64,
    ) -> Option<AudioPlan> {
        let packets: Vec<_> = (0..count)
            .map(|k| AudioPacket {
                data: vec![0; 10],
                pts: at_samples(first_packet_at + k * 1024),
                duration: at_samples(1024),
            })
            .collect();
        plan_audio(
            &packets,
            &AudioParams {
                priming_frames: priming,
                ..audio_params()
            },
            at_samples(video_start),
        )
    }

    #[test]
    fn aligned_audio_without_encoder_delay_needs_no_edit_list() {
        // Audio starts together with the video and the encoder has no delay.
        let plan = plan_for(0, 100, 0, 0).unwrap();
        assert_eq!(plan.first, 0);
        assert!(plan.edits.is_empty());
    }

    #[test]
    fn encoder_delay_becomes_a_skip_at_the_start_of_the_track() {
        // Stream start: nothing before the video, 2112 samples of priming.
        // Packet 1 is the latest that leaves the decoder a warm-up block:
        // decoded position p of the file = input p - 1088, so skip 1088.
        let plan = plan_for(0, 100, 2112, 0).unwrap();
        assert_eq!(plan.first, 1);
        assert_eq!(plan.edits.len(), 1);
        assert_eq!(plan.edits[0].media_time, 1088);
        // Plays the remaining (99*1024 - 1088) samples, in movie ticks.
        assert_eq!(
            plan.edits[0].segment_duration,
            ((99i64 * 1024 - 1088) * 90_000 / 48_000) as u32
        );
    }

    #[test]
    fn a_mid_stream_clip_starts_a_packet_early_for_decoder_warmup() {
        // Video starts at sample 48_000 (1 s); priming 2112, warm-up 1024:
        // the track must start at the latest packet with pts <= x0 + 1088.
        let plan = plan_for(0, 200, 2112, 48_000).unwrap();
        let first_pts = plan.first as i64 * 1024;
        assert!(first_pts <= 48_000 + 1088 && 48_000 + 1088 - first_pts < 1024);
        let skip = plan.edits.last().unwrap().media_time as i64;
        assert!((1024..2048).contains(&skip), "skip {skip}");
        // The skip lands exactly on the video start.
        assert_eq!(first_pts + skip - 2112, 48_000);
    }

    #[test]
    fn audio_starting_after_the_video_gets_leading_silence() {
        // Audio begins 9600 samples (200 ms) after the video.
        let plan = plan_for(9_600, 100, 0, 0).unwrap();
        assert_eq!(plan.first, 0);
        assert_eq!(plan.edits.len(), 2);
        assert_eq!(plan.edits[0].media_time, -1);
        assert_eq!(
            plan.edits[0].segment_duration,
            (9_600i64 * 90_000 / 48_000) as u32
        );
        assert_eq!(plan.edits[1].media_time, 0);
    }

    #[test]
    fn audio_entirely_consumed_by_the_skip_is_dropped() {
        // One packet, but the video starts far into it with big priming.
        assert!(plan_for(0, 1, 2112, 0).is_none());
    }

    fn read_back(path: &Path) -> mp4::Mp4Reader<std::io::BufReader<File>> {
        let size = std::fs::metadata(path).unwrap().len();
        let reader = std::io::BufReader::new(File::open(path).unwrap());
        mp4::Mp4Reader::read_header(reader, size).unwrap()
    }

    #[test]
    fn writes_an_aac_track_next_to_the_video() {
        let dir = std::env::temp_dir().join("clipforge-mp4mux-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("with-audio.mp4");
        let (params, sets) = fake_params();
        let packets = fake_packets(120, 60, 30); // 2 s
        let audio = fake_audio(94, Duration::ZERO); // ≈ 2.005 s

        write_mp4_with_audio(
            &path,
            &params,
            &sets,
            &packets,
            Some((&audio_params(), &audio)),
        )
        .unwrap();

        let mp4 = read_back(&path);
        assert_eq!(mp4.tracks().len(), 2);
        let video = mp4.tracks().get(&1).unwrap();
        let audio_track = mp4.tracks().get(&2).unwrap();
        assert_eq!(video.sample_count(), 120);
        assert_eq!(audio_track.track_type().unwrap(), mp4::TrackType::Audio);
        assert_eq!(audio_track.sample_count(), 94);
        assert_eq!(audio_track.timescale(), 48_000);
        assert_eq!(
            audio_track.sample_freq_index().unwrap(),
            mp4::SampleFreqIndex::Freq48000
        );
        assert_eq!(
            audio_track.channel_config().unwrap(),
            mp4::ChannelConfig::Stereo
        );
        assert_eq!(
            audio_track.audio_profile().unwrap(),
            mp4::AudioObjectType::AacLowComplexity
        );
        let expected = Duration::from_secs_f64(94.0 * 1024.0 / 48_000.0);
        assert!(
            audio_track.duration().abs_diff(expected) < Duration::from_millis(2),
            "audio duration {:?}",
            audio_track.duration()
        );
        // Sample payloads survive the round trip, in order.
        let mut reader = read_back(&path);
        let first = reader.read_sample(2, 1).unwrap().unwrap();
        let last = reader.read_sample(2, 94).unwrap().unwrap();
        assert_eq!(first.bytes[0], 0);
        assert_eq!(last.bytes[0], 93);
        assert_eq!(last.start_time, 93 * 1024);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn audio_and_video_samples_are_interleaved_in_the_file() {
        let dir = std::env::temp_dir().join("clipforge-mp4mux-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("interleaved.mp4");
        let (params, sets) = fake_params();
        // 10 s: long enough for several 1-second chunks per track.
        let packets = fake_packets(600, 60, 60);
        let audio = fake_audio(469, Duration::ZERO);
        write_mp4_with_audio(
            &path,
            &params,
            &sets,
            &packets,
            Some((&audio_params(), &audio)),
        )
        .unwrap();

        let mp4 = read_back(&path);
        let offsets = |track_index: usize| -> Vec<u64> {
            let stbl = &mp4.moov.traks[track_index].mdia.minf.stbl;
            match (&stbl.co64, &stbl.stco) {
                (Some(co64), _) => co64.entries.clone(),
                (None, Some(stco)) => stco.entries.iter().map(|&o| u64::from(o)).collect(),
                (None, None) => panic!("track has no chunk offset table"),
            }
        };
        let (video, audio) = (offsets(0), offsets(1));
        assert!(
            video.len() > 3 && audio.len() > 3,
            "multiple chunks per track"
        );
        // Audio chunks start before the last video chunk and vice versa: the
        // two tracks alternate instead of one following the other.
        assert!(audio[0] < *video.last().unwrap());
        assert!(video[1] < *audio.last().unwrap());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_audio_slice_writes_a_video_only_file() {
        let dir = std::env::temp_dir().join("clipforge-mp4mux-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty-audio.mp4");
        let (params, sets) = fake_params();
        write_mp4_with_audio(
            &path,
            &params,
            &sets,
            &fake_packets(30, 60, 30),
            Some((&audio_params(), &[])),
        )
        .unwrap();
        assert_eq!(read_back(&path).tracks().len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_audio_formats_the_container_cannot_describe() {
        let (params, sets) = fake_params();
        let sink = std::env::temp_dir().join("clipforge-mp4mux-badaudio.mp4");
        let audio = fake_audio(2, Duration::ZERO);
        for bad in [
            AudioParams {
                sample_rate: 12_345,
                ..audio_params()
            },
            AudioParams {
                channels: 6,
                ..audio_params()
            },
        ] {
            assert!(matches!(
                write_mp4_with_audio(
                    &sink,
                    &params,
                    &sets,
                    &fake_packets(5, 60, 5),
                    Some((&bad, &audio))
                ),
                Err(MuxError::UnsupportedAudio(_))
            ));
        }
        std::fs::remove_file(&sink).ok();
    }

    #[test]
    fn rejects_empty_and_non_keyframe_start() {
        let (params, sets) = fake_params();
        let sink = std::env::temp_dir().join("clipforge-mp4mux-reject.mp4");
        assert!(matches!(
            write_mp4(&sink, &params, &sets, &[]),
            Err(MuxError::NoPackets)
        ));
        let mut packets = fake_packets(10, 60, 5);
        packets.remove(0); // now starts mid-GOP
        assert!(matches!(
            write_mp4(&sink, &params, &sets, &packets),
            Err(MuxError::FirstPacketNotKeyframe)
        ));
        std::fs::remove_file(&sink).ok();
    }

    #[test]
    fn rejects_missing_parameter_sets() {
        let (params, _) = fake_params();
        let sink = std::env::temp_dir().join("clipforge-mp4mux-nops.mp4");
        assert!(matches!(
            write_mp4(
                &sink,
                &params,
                &ParameterSets::default(),
                &fake_packets(5, 60, 5)
            ),
            Err(MuxError::MissingParameterSets)
        ));
        std::fs::remove_file(&sink).ok();
    }

    #[test]
    fn cursor_smoke_no_filesystem() {
        // Guard against the writer needing Seek weirdness: write to memory.
        let (params, sets) = fake_params();
        let packets = fake_packets(6, 60, 3);
        let mut writer = mp4::Mp4Writer::write_start(
            Cursor::new(Vec::new()),
            &mp4::Mp4Config {
                major_brand: (*b"isom").into(),
                minor_version: 512,
                compatible_brands: vec![(*b"isom").into()],
                timescale: 1000,
            },
        )
        .unwrap();
        writer
            .add_track(&mp4::TrackConfig {
                track_type: mp4::TrackType::Video,
                timescale: TRACK_TIMESCALE,
                language: "und".into(),
                media_conf: mp4::MediaConfig::AvcConfig(mp4::AvcConfig {
                    width: params.width as u16,
                    height: params.height as u16,
                    seq_param_set: sets.sps[0].clone(),
                    pic_param_set: sets.pps[0].clone(),
                }),
            })
            .unwrap();
        for p in &packets {
            writer
                .write_sample(
                    1,
                    &mp4::Mp4Sample {
                        start_time: to_ticks(p.pts),
                        duration: TRACK_TIMESCALE / 60,
                        rendering_offset: 0,
                        is_sync: p.is_keyframe,
                        bytes: bytes::Bytes::copy_from_slice(&p.data),
                    },
                )
                .unwrap();
        }
        writer.write_end().unwrap();
    }
}
