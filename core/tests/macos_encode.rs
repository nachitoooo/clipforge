//! End-to-end encode test on macOS: TestPatternSource → VideoToolboxEncoder
//! → .mp4, verifying packet contract and container duration. Runs on any Mac
//! with a hardware H.264 encoder (Apple Silicon always has one); skips
//! gracefully where hardware encoding is unavailable (e.g. some CI VMs).

#![cfg(target_os = "macos")]

use std::time::Duration;

use clipforge_core::capture::test_pattern::{TestPatternConfig, TestPatternSource};
use clipforge_core::capture::FrameSource;
use clipforge_core::clip::mp4mux::{write_mp4, VideoParams};
use clipforge_core::encode::videotoolbox::VideoToolboxEncoder;
use clipforge_core::encode::{Codec, EncodeError, Encoder, EncoderConfig};

const FRAMES: u64 = 120; // 2 s at 60 fps
const CONFIG: EncoderConfig = EncoderConfig {
    codec: Codec::H264,
    width: 1280,
    height: 720,
    fps: 60,
    bitrate: 8_000_000,
    keyframe_interval: 60,
};

#[tokio::test]
async fn test_pattern_to_mp4_duration_matches() {
    let mut encoder = match VideoToolboxEncoder::new(CONFIG.clone()) {
        Ok(encoder) => encoder,
        Err(EncodeError::HardwareUnavailable(codec)) => {
            eprintln!("SKIP: no hardware {codec:?} encoder on this machine");
            return;
        }
        Err(e) => panic!("encoder construction failed: {e}"),
    };

    let mut source = TestPatternSource::new(TestPatternConfig {
        width: CONFIG.width,
        height: CONFIG.height,
        fps: CONFIG.fps,
        frame_limit: Some(FRAMES),
        realtime: false,
    });

    let mut rx = source.start().expect("source starts");
    let mut packets = Vec::new();
    while let Some(frame) = rx.recv().await {
        packets.extend(encoder.encode(&frame).expect("encode"));
    }
    packets.extend(encoder.finish().expect("finish"));

    // Packet contract: nothing lost, starts on a keyframe, pts monotonic.
    assert_eq!(packets.len() as u64, FRAMES, "one packet per frame");
    assert!(packets[0].is_keyframe, "stream must start with an IDR");
    for pair in packets.windows(2) {
        assert!(pair[1].pts > pair[0].pts, "monotonic pts");
    }
    let keyframes = packets.iter().filter(|p| p.is_keyframe).count();
    assert!(
        keyframes >= 2,
        "expected a keyframe at least every {} frames, got {keyframes} total",
        CONFIG.keyframe_interval
    );

    let sets = encoder.parameter_sets().expect("SPS/PPS after encoding");
    assert!(!sets.sps.is_empty() && !sets.pps.is_empty());

    // Mux and verify duration through an independent reader.
    let dir = std::env::temp_dir().join("clipforge-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test_pattern_720p60.mp4");
    let params = VideoParams {
        codec: CONFIG.codec,
        width: CONFIG.width,
        height: CONFIG.height,
        fps: CONFIG.fps,
    };
    let duration = write_mp4(&path, &params, &sets, &packets).expect("mux");
    assert!(
        duration.abs_diff(Duration::from_secs(2)) < Duration::from_millis(2),
        "muxer-reported duration {duration:?} not ~2 s"
    );

    let size = std::fs::metadata(&path).unwrap().len();
    let reader = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
    let mp4 = mp4::Mp4Reader::read_header(reader, size).unwrap();
    let container = mp4.duration();
    assert!(
        container >= Duration::from_millis(1980) && container <= Duration::from_millis(2020),
        "container duration {container:?} should be ~2 s"
    );
    let track = mp4.tracks().values().next().unwrap();
    assert_eq!(track.sample_count() as u64, FRAMES);
    assert_eq!(track.width(), CONFIG.width as u16);
    assert_eq!(track.height(), CONFIG.height as u16);

    println!(
        "e2e: wrote {} ({} KiB, {FRAMES} frames, {keyframes} keyframes, {container:?})",
        path.display(),
        size / 1024
    );
}
