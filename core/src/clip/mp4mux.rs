//! Mux encoded H.264 packets into an .mp4 file — a container write only,
//! never a re-encode. Packets must already be AVCC (4-byte length-prefixed),
//! which is what [`crate::encode`] encoders emit.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::time::Duration;

use thiserror::Error;

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
    let first = packets.first().ok_or(MuxError::NoPackets)?;
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

    let base_pts = first.pts;
    let frame_ticks = (TRACK_TIMESCALE / params.fps.max(1)).max(1);
    for (i, packet) in packets.iter().enumerate() {
        let start_time = to_ticks(packet.pts - base_pts);
        // Sample duration = gap to the next packet; nominal 1/fps for the
        // last one.
        let duration = match packets.get(i + 1) {
            Some(next) => (to_ticks(next.pts - base_pts) - start_time).max(1) as u32,
            None => frame_ticks,
        };
        writer.write_sample(
            1,
            &mp4::Mp4Sample {
                start_time,
                duration,
                rendering_offset: 0,
                is_sync: packet.is_keyframe,
                bytes: bytes::Bytes::copy_from_slice(&packet.data),
            },
        )?;
    }
    writer.write_end()?;

    let last = packets.last().expect("checked non-empty");
    Ok(last.pts - base_pts + Duration::from_secs(1) / params.fps.max(1))
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
