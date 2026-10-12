//! F-2.2 tests: save-last-N-seconds through ClipService — file naming,
//! extraction latency (<500 ms for a 30 s clip), and capture continuing
//! uninterrupted while a clip is being written.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clipforge_core::buffer::{BufferConfig, ReplayBuffer};
use clipforge_core::clip::mp4mux::VideoParams;
use clipforge_core::clip::{ClipError, ClipService, StreamInfo};
use clipforge_core::encode::{Codec, EncodedPacket, ParameterSets};

const FPS: u64 = 60;
const GOP: u64 = 120;

fn pts_of(n: u64) -> Duration {
    Duration::from_nanos(n * 1_000_000_000 / FPS)
}

/// Realistic 1080p60 packet sizes (~12 Mb/s) with a valid AVCC shell.
fn packet(n: u64) -> EncodedPacket {
    let is_keyframe = n.is_multiple_of(GOP);
    let payload = if is_keyframe { 80_000 } else { 22_000 };
    let mut data = (payload as u32).to_be_bytes().to_vec();
    data.push(if is_keyframe { 0x65 } else { 0x41 });
    data.extend(std::iter::repeat_n(0u8, payload - 1));
    EncodedPacket {
        data,
        pts: pts_of(n),
        dts: pts_of(n),
        is_keyframe,
    }
}

fn stream_info() -> StreamInfo {
    StreamInfo {
        video: VideoParams {
            codec: Codec::H264,
            width: 1920,
            height: 1080,
            fps: FPS as u32,
        },
        parameter_sets: ParameterSets {
            sps: vec![vec![0x67, 0x42, 0xc0, 0x28, 0x96, 0x54, 0x03, 0xc0]],
            pps: vec![vec![0x68, 0xce, 0x3c, 0x80]],
        },
    }
}

fn filled_service(seconds: u64, dir: &std::path::Path) -> (Arc<Mutex<ReplayBuffer>>, ClipService) {
    let buffer = Arc::new(Mutex::new(ReplayBuffer::new(BufferConfig::default())));
    {
        let mut locked = buffer.lock().unwrap();
        for n in 0..(seconds * FPS) {
            locked.push(packet(n));
        }
    }
    let service = ClipService::new(Arc::clone(&buffer), dir);
    service.set_stream_info(stream_info());
    (buffer, service)
}

fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("clipforge-clip-tests").join(name);
    std::fs::remove_dir_all(&dir).ok();
    dir
}

#[test]
fn saves_timestamped_clip_into_configured_folder() {
    let dir = test_dir("naming");
    let (_buffer, service) = filled_service(60, &dir);

    let saved = service.save_last(Duration::from_secs(15)).unwrap();
    assert!(saved.path.starts_with(&dir), "clip in configured folder");
    let name = saved.path.file_name().unwrap().to_string_lossy();
    assert!(
        name.starts_with("clip-") && name.ends_with(".mp4"),
        "timestamped name, got {name}"
    );
    assert!(saved.bytes > 0);
    assert!(
        saved.duration >= Duration::from_secs(15) && saved.duration <= Duration::from_secs(15 + 2),
        "duration {:?}",
        saved.duration
    );

    // Wrote a real, readable mp4.
    let size = std::fs::metadata(&saved.path).unwrap().len();
    let reader = std::io::BufReader::new(std::fs::File::open(&saved.path).unwrap());
    let mp4 = mp4::Mp4Reader::read_header(reader, size).unwrap();
    assert_eq!(mp4.tracks().len(), 1);

    // A second save in the same second gets a distinct name.
    let saved2 = service.save_last(Duration::from_secs(15)).unwrap();
    assert_ne!(saved.path, saved2.path, "no filename collision");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn empty_buffer_and_missing_stream_info_error_cleanly() {
    let dir = test_dir("errors");
    let buffer = Arc::new(Mutex::new(ReplayBuffer::new(BufferConfig::default())));
    let service = ClipService::new(Arc::clone(&buffer), &dir);

    // No stream info yet.
    assert!(matches!(
        service.save_last(Duration::from_secs(10)),
        Err(ClipError::NotReady)
    ));

    // Stream info but empty buffer.
    service.set_stream_info(stream_info());
    assert!(matches!(
        service.save_last(Duration::from_secs(10)),
        Err(ClipError::BufferEmpty)
    ));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn thirty_second_clip_saves_in_under_500ms() {
    let dir = test_dir("latency");
    // Full 120 s buffer — worst case for the extraction scan.
    let (_buffer, service) = filled_service(120, &dir);

    let started = Instant::now();
    let saved = service.save_last(Duration::from_secs(30)).unwrap();
    let elapsed = started.elapsed();

    println!(
        "clip extraction: 30 s clip ({} packets, {:.1} MiB) saved in {:?}",
        saved.packets,
        saved.bytes as f64 / (1024.0 * 1024.0),
        elapsed
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "extraction took {elapsed:?} (budget 500 ms)"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn capture_keeps_running_during_extraction() {
    let dir = test_dir("concurrency");
    let (buffer, service) = filled_service(120, &dir);

    // A "capture thread" keeps pushing at 60 fps while we save; every push
    // must stay fast (the service must not hold the buffer lock during
    // muxing/disk I/O).
    let stop = Arc::new(AtomicBool::new(false));
    let pusher = {
        let buffer = Arc::clone(&buffer);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut n = 120 * FPS;
            let mut waits = Vec::new();
            while !stop.load(Ordering::SeqCst) {
                let t = Instant::now();
                buffer.lock().unwrap().push(packet(n));
                waits.push(t.elapsed());
                n += 1;
                std::thread::sleep(Duration::from_millis(1000 / FPS));
            }
            waits
        })
    };

    let mut saves = 0u32;
    let mut total_save_time = Duration::ZERO;
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(600) {
        let t = Instant::now();
        service.save_last(Duration::from_secs(30)).unwrap();
        total_save_time += t.elapsed();
        saves += 1;
    }
    stop.store(true, Ordering::SeqCst);
    let mut waits = pusher.join().unwrap();
    waits.sort();
    let pushes = waits.len();
    let worst_push = *waits.last().unwrap();
    let typical_push = waits[pushes * 9 / 10]; // the 90th percentile
    let avg_save = total_save_time / saves;

    println!(
        "concurrency: {saves} saves (avg {avg_save:?}) while {pushes} pushes ran; \
         worst push wait {worst_push:?}, 90th percentile {typical_push:?}"
    );
    assert!(saves >= 1 && pushes > 0);
    // What we guarantee is lock discipline: the buffer lock is never held
    // while a clip is muxed/written. If it were, the saves would run back to
    // back (as they do here) and *most* pushes would wait for one. A single
    // slow push proves nothing: on a shared CI runner the scheduler alone can
    // park a thread for tens of ms (this flaked at 67 ms with 48 ms saves). So
    // the bound is on the 90th percentile of the waits, not on the worst one.
    let threshold = Duration::from_millis(20).max(avg_save / 4);
    assert!(
        typical_push < threshold,
        "pushes stalled (90th percentile {typical_push:?}, worst {worst_push:?}) during clip \
         saves (avg save {avg_save:?}) — the save path is likely holding the buffer lock during I/O"
    );
    std::fs::remove_dir_all(&dir).ok();
}
