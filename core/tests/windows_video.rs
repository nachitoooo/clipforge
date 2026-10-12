//! Windows video integration tests: Media Foundation H.264 encoding, Windows
//! Graphics Capture of the real desktop, and the whole recording pipeline.
//! `#[ignore]`d because they need a Windows desktop session —
//! `cargo test -p clipforge-core --test windows_video -- --ignored --nocapture`.
//! The `windows-video` GitHub workflow runs them on a Windows runner and
//! keeps the clips they write (set `CLIPFORGE_TEST_OUT` to choose the folder).
//!
//! What a clip must satisfy is checked with an independent reader: Windows' own
//! MP4 source and H.264 decoder, through a Media Foundation source reader.
#![cfg(target_os = "windows")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clipforge_core::buffer::BufferConfig;
use clipforge_core::capture::clock::SharedEpoch;
use clipforge_core::capture::combined::WithAudio;
use clipforge_core::capture::scale::fit_even;
use clipforge_core::capture::test_pattern::{TestPatternConfig, TestPatternSource};
use clipforge_core::capture::windows::{
    primary_display_size, WindowsCaptureConfig, WindowsGraphicsCaptureSource,
};
use clipforge_core::capture::{FrameSource, RawFrame};
use clipforge_core::clip::mp4mux::{write_mp4, VideoParams};
use clipforge_core::clip::ClipService;
use clipforge_core::encode::mediafoundation::MediaFoundationEncoder;
use clipforge_core::encode::nv12::{bgra_to_nv12, nv12_len};
use clipforge_core::encode::{Codec, EncodedPacket, Encoder, EncoderConfig};
use clipforge_core::engine::CaptureEngine;
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::{
    MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Video,
    MFShutdown, MFStartup, MFVideoFormat_NV12, MFSTARTUP_FULL, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_PD_DURATION, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MF_SOURCE_READER_MEDIASOURCE, MF_VERSION,
};
use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;

fn out_dir() -> PathBuf {
    let dir = std::env::var_os("CLIPFORGE_TEST_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("clipforge-windows-video"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// What the Windows MP4 reader and H.264 decoder make of a file.
struct Decoded {
    width: u32,
    height: u32,
    duration: Duration,
    frames: u64,
    /// The first picture as NV12, top-down, `width` bytes per row.
    first_frame: Vec<u8>,
}

fn decode_with_windows(path: &Path) -> Decoded {
    // SAFETY: plain Media Foundation calls; the reader and samples are dropped
    // before `MFShutdown`.
    unsafe {
        MFStartup(MF_VERSION, MFSTARTUP_FULL).expect("MFStartup");
        let decode = || {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1).expect("attributes");
            let attributes = attributes.unwrap();
            attributes
                .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
                .expect("enable processing");
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &attributes)
                .expect("Windows can open the mp4");
            let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

            let wanted = MFCreateMediaType().unwrap();
            wanted
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .unwrap();
            wanted.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).unwrap();
            reader
                .SetCurrentMediaType(stream, None, &wanted)
                .expect("Windows can decode the H.264 stream");
            let size = reader
                .GetCurrentMediaType(stream)
                .unwrap()
                .GetUINT64(&MF_MT_FRAME_SIZE)
                .unwrap();
            let (width, height) = ((size >> 32) as u32, (size & 0xFFFF_FFFF) as u32);

            let duration_hns = reader
                .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
                .ok()
                .and_then(|v| u64::try_from(&v).ok())
                .unwrap_or(0);

            let mut frames = 0u64;
            let mut first_frame = Vec::new();
            loop {
                let (mut flags, mut sample) = (0u32, None);
                reader
                    .ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample))
                    .expect("read a decoded frame");
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(sample) = sample else { continue };
                frames += 1;
                if first_frame.is_empty() {
                    let buffer = sample.ConvertToContiguousBuffer().unwrap();
                    let (mut ptr, mut len) = (std::ptr::null_mut(), 0u32);
                    buffer.Lock(&mut ptr, None, Some(&mut len)).unwrap();
                    first_frame = std::slice::from_raw_parts(ptr, len as usize).to_vec();
                    buffer.Unlock().unwrap();
                }
            }
            Decoded {
                width,
                height,
                duration: Duration::from_nanos(duration_hns * 100),
                frames,
                first_frame,
            }
        };
        let decoded = decode();
        let _ = MFShutdown();
        decoded
    }
}

/// Mean absolute difference between two byte slices.
fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    let sum: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    sum as f64 / a.len().min(b.len()) as f64
}

fn mux(
    path: &Path,
    config: &EncoderConfig,
    encoder: &dyn Encoder,
    packets: &[EncodedPacket],
) -> Duration {
    let params = VideoParams {
        codec: config.codec,
        width: config.width,
        height: config.height,
        fps: config.fps,
    };
    write_mp4(
        path,
        &params,
        &encoder.parameter_sets().expect("SPS/PPS after encoding"),
        packets,
    )
    .expect("mux")
}

/// A 24-bit BMP of a BGRA frame, so a captured picture can be looked at.
fn write_bmp(path: &Path, frame: &RawFrame) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let row = (w * 3).div_ceil(4) * 4;
    let mut bytes = Vec::with_capacity(54 + row * h);
    bytes.extend(b"BM");
    bytes.extend(((54 + row * h) as u32).to_le_bytes());
    bytes.extend([0, 0, 0, 0]);
    bytes.extend(54u32.to_le_bytes());
    bytes.extend(40u32.to_le_bytes());
    bytes.extend((w as i32).to_le_bytes());
    bytes.extend((h as i32).to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(24u16.to_le_bytes());
    bytes.extend([0u8; 24]);
    for y in (0..h).rev() {
        let mut line = Vec::with_capacity(row);
        for x in 0..w {
            line.extend(&frame.data[(y * w + x) * 4..(y * w + x) * 4 + 3]);
        }
        line.resize(row, 0);
        bytes.extend(line);
    }
    std::fs::write(path, bytes).unwrap();
}

/// Keep the screen changing while a test captures it: Graphics Capture only
/// delivers a frame when something on screen changed, and a CI desktop is idle.
fn wiggle_cursor(
    stop: Arc<std::sync::atomic::AtomicBool>,
    area: (u32, u32),
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut t = 0f64;
        while !stop.load(std::sync::atomic::Ordering::SeqCst) {
            let x = (area.0 as f64 * (0.5 + 0.4 * (t).cos())) as i32;
            let y = (area.1 as f64 * (0.5 + 0.4 * (t * 1.3).sin())) as i32;
            // SAFETY: plain call.
            let _ = unsafe { SetCursorPos(x, y) };
            t += 0.15;
            std::thread::sleep(Duration::from_millis(8));
        }
    })
}

// ---- the encoder alone -------------------------------------------------------

#[tokio::test]
#[ignore = "needs Windows with the Media Foundation H.264 encoder"]
async fn the_encoder_makes_an_mp4_that_windows_decodes_back_to_the_same_picture() {
    let config = EncoderConfig {
        codec: Codec::H264,
        width: 1280,
        height: 720,
        fps: 60,
        bitrate: 8_000_000,
        keyframe_interval: 60,
    };
    let mut encoder = MediaFoundationEncoder::new(config.clone()).expect("H.264 encoder MFT");
    let mut source = TestPatternSource::new(TestPatternConfig {
        width: config.width,
        height: config.height,
        fps: config.fps,
        frame_limit: Some(120),
        realtime: false,
    });
    let mut rx = source.start().unwrap();
    let mut first_input = None;
    let mut packets = Vec::new();
    let started = Instant::now();
    while let Some(frame) = rx.recv().await {
        first_input.get_or_insert_with(|| frame.clone());
        packets.extend(encoder.encode(&frame).expect("encode"));
    }
    packets.extend(encoder.finish().expect("finish"));
    let took = started.elapsed();
    eprintln!(
        "encoded 120 frames of 720p in {took:?} ({:.0} fps)",
        120.0 / took.as_secs_f64()
    );

    assert_eq!(packets.len(), 120, "one packet per frame");
    assert!(packets[0].is_keyframe, "starts on an IDR");
    for (i, p) in packets
        .iter()
        .take(8)
        .chain(packets.iter().skip(58).take(4))
        .enumerate()
    {
        eprintln!(
            "packet #{i}: pts {:?} dts {:?} key {} len {}",
            p.pts,
            p.dts,
            p.is_keyframe,
            p.data.len()
        );
    }
    let bad: Vec<_> = packets
        .windows(2)
        .enumerate()
        .filter(|(_, p)| p[1].pts <= p[0].pts)
        .map(|(i, p)| (i, p[0].pts, p[1].pts))
        .take(5)
        .collect();
    assert!(
        bad.is_empty(),
        "pts must increase; first offenders (index, before, after): {bad:?}"
    );
    assert!(
        packets.iter().filter(|p| p.is_keyframe).count() >= 2,
        "a keyframe at least every {} frames",
        config.keyframe_interval
    );
    let sets = encoder.parameter_sets().expect("parameter sets");
    assert!(!sets.sps.is_empty() && !sets.pps.is_empty());

    let path = out_dir().join("encoder-test-pattern-720p60.mp4");
    let duration = mux(&path, &config, &encoder, &packets);
    assert!(
        duration.abs_diff(Duration::from_secs(2)) < Duration::from_millis(50),
        "{duration:?}"
    );

    let decoded = decode_with_windows(&path);
    eprintln!(
        "windows decoded {} frames, {}x{}, {:?}",
        decoded.frames, decoded.width, decoded.height, decoded.duration
    );
    assert_eq!((decoded.width, decoded.height), (1280, 720));
    assert_eq!(decoded.frames, 120, "every frame decodes");

    // The first decoded picture must be the first one we fed in: same colors
    // (the matrix and range were right) and the same way up.
    let input = first_input.unwrap();
    let mut expected = vec![0u8; nv12_len(1280, 720)];
    bgra_to_nv12(&input.data, 1280, 720, &mut expected);
    let luma = mean_abs_diff(&decoded.first_frame[..1280 * 720], &expected[..1280 * 720]);
    let chroma = mean_abs_diff(&decoded.first_frame[1280 * 720..], &expected[1280 * 720..]);
    eprintln!("first picture: mean luma error {luma:.2}, chroma error {chroma:.2}");
    assert!(luma < 8.0, "brightness differs by {luma}");
    assert!(chroma < 8.0, "color differs by {chroma}");
}

#[tokio::test]
#[ignore = "needs Windows with the Media Foundation H.264 encoder"]
async fn how_fast_the_encoder_runs_at_1080p() {
    let config = EncoderConfig::default(); // 1920x1080 @ 60, 12 Mb/s
    let mut encoder = MediaFoundationEncoder::new(config.clone()).expect("H.264 encoder MFT");
    let mut source = TestPatternSource::new(TestPatternConfig {
        width: config.width,
        height: config.height,
        fps: config.fps,
        frame_limit: Some(180),
        realtime: false,
    });
    let mut rx = source.start().unwrap();
    let mut packets = 0usize;
    let started = Instant::now();
    while let Some(frame) = rx.recv().await {
        packets += encoder.encode(&frame).expect("encode").len();
    }
    packets += encoder.finish().expect("finish").len();
    let took = started.elapsed();
    let fps = 180.0 / took.as_secs_f64();
    eprintln!(
        "1080p: 180 frames in {took:?} = {fps:.1} fps (needs 60 to keep up in real time), {} CPUs",
        std::thread::available_parallelism().map_or(1, |n| n.get())
    );
    assert_eq!(packets, 180);
    assert!(fps > 5.0, "far too slow to record with: {fps:.1} fps");
}

// ---- capturing the real desktop ----------------------------------------------

#[tokio::test]
#[ignore = "needs a Windows desktop session"]
async fn graphics_capture_delivers_frames_of_the_desktop() {
    let (screen_w, screen_h) = primary_display_size().expect("primary display");
    let (width, height) = fit_even(screen_w, screen_h, 1920, 1080);
    eprintln!("screen {screen_w}x{screen_h} -> frames {width}x{height}");

    let mut source = WindowsGraphicsCaptureSource::new(
        WindowsCaptureConfig {
            width,
            height,
            fps: 60,
            show_cursor: true,
        },
        SharedEpoch::new(),
    );
    let mut rx = source.start().expect("capture starts");
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let wiggler = wiggle_cursor(Arc::clone(&stop), (screen_w, screen_h));

    let mut frames: Vec<RawFrame> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => break,
            Err(_) => {}
        }
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    wiggler.join().unwrap();
    source.stop();

    let stats = source.stats();
    eprintln!(
        "{} frames in 4 s ({:.1} fps), stats {stats:?}",
        frames.len(),
        frames.len() as f64 / 4.0
    );
    assert!(
        frames.len() >= 20,
        "only {} frames: capture works but the screen did not update?",
        frames.len()
    );
    assert!(frames
        .iter()
        .all(|f| (f.width, f.height) == (width, height)));
    assert!(frames.iter().all(|f| f.data.len() == f.expected_len()));
    assert!(
        frames.windows(2).all(|p| p[1].timestamp > p[0].timestamp),
        "timestamps strictly increase"
    );
    assert_eq!(
        frames[0].timestamp,
        Duration::ZERO,
        "the first frame is the epoch"
    );
    assert!(
        frames
            .windows(2)
            .all(|p| p[1].timestamp - p[0].timestamp >= Duration::from_millis(10)),
        "no faster than the 60 fps asked for"
    );
    assert!(
        stats.drop_rate() < 0.05,
        "dropped {:.1}%",
        stats.drop_rate() * 100.0
    );

    // A real picture, not a blank one: more than one color on screen.
    let last = frames.last().unwrap();
    let first_pixel = &last.data[..4];
    assert!(
        last.data.chunks_exact(4).any(|p| p != first_pixel),
        "the captured frame is one flat color"
    );
    write_bmp(&out_dir().join("desktop-last-frame.bmp"), last);
}

#[tokio::test]
#[ignore = "needs a Windows desktop session and the Media Foundation H.264 encoder"]
async fn the_desktop_is_recorded_into_an_mp4_that_windows_plays() {
    let (screen_w, screen_h) = primary_display_size().expect("primary display");
    let (width, height) = fit_even(screen_w, screen_h, 1920, 1080);
    let config = EncoderConfig {
        width,
        height,
        bitrate: 8_000_000,
        ..EncoderConfig::default()
    };
    let mut encoder = MediaFoundationEncoder::new(config.clone()).expect("H.264 encoder MFT");
    let mut source = WindowsGraphicsCaptureSource::new(
        WindowsCaptureConfig {
            width,
            height,
            fps: config.fps,
            show_cursor: true,
        },
        SharedEpoch::new(),
    );
    let mut rx = source.start().expect("capture starts");
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let wiggler = wiggle_cursor(Arc::clone(&stop), (screen_w, screen_h));

    let mut packets = Vec::new();
    let mut inputs = 0u64;
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if let Ok(Some(frame)) = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            inputs += 1;
            packets.extend(encoder.encode(&frame).expect("encode"));
        }
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    wiggler.join().unwrap();
    source.stop();
    packets.extend(encoder.finish().expect("finish"));
    assert!(inputs >= 20, "only {inputs} frames captured");
    assert_eq!(
        packets.len() as u64,
        inputs,
        "one packet per captured frame"
    );

    let path = out_dir().join("desktop-capture.mp4");
    let duration = mux(&path, &config, &encoder, &packets);
    let decoded = decode_with_windows(&path);
    eprintln!(
        "{inputs} frames -> {} KiB, {duration:?}; windows decoded {} frames {}x{}",
        std::fs::metadata(&path).unwrap().len() / 1024,
        decoded.frames,
        decoded.width,
        decoded.height
    );
    assert_eq!((decoded.width, decoded.height), (width, height));
    assert_eq!(decoded.frames, inputs, "every captured frame plays back");
}

// ---- the whole recorder --------------------------------------------------------

#[test]
#[ignore = "needs a Windows desktop session and the Media Foundation H.264 encoder"]
fn the_recorder_saves_a_clip_of_what_was_on_screen() {
    let (screen_w, screen_h) = primary_display_size().expect("primary display");
    let (width, height) = fit_even(screen_w, screen_h, 1920, 1080);
    let config = EncoderConfig {
        width,
        height,
        bitrate: 8_000_000,
        ..EncoderConfig::default()
    };

    let epoch = SharedEpoch::new();
    let video = WindowsGraphicsCaptureSource::new(
        WindowsCaptureConfig {
            width,
            height,
            fps: config.fps,
            show_cursor: true,
        },
        epoch.clone(),
    );
    // Audio is best effort: a runner may have no sound device at all.
    let audio = clipforge_core::audio::wasapi::WasapiLoopbackSource::new(
        clipforge_core::audio::AudioConfig::default(),
        epoch,
    );
    let source = WithAudio::new(video, audio);
    let encoder = MediaFoundationEncoder::new(config).expect("H.264 encoder MFT");
    let audio_encoder: Option<Box<dyn clipforge_core::audio::AudioEncoder>> =
        clipforge_core::audio::aac_mediafoundation::MediaFoundationAacEncoder::new(
            clipforge_core::audio::AudioConfig::default(),
        )
        .map(|e| Box::new(e) as _)
        .map_err(|e| eprintln!("no AAC encoder here: {e}"))
        .ok();

    let mut engine = CaptureEngine::new(BufferConfig::default());
    let clips = Arc::new(ClipService::new(engine.buffer(), out_dir().join("clips")));
    engine
        .start_with_audio(
            Box::new(source),
            Box::new(encoder),
            audio_encoder,
            Arc::clone(&clips),
        )
        .expect("the engine starts");

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let wiggler = wiggle_cursor(Arc::clone(&stop), (screen_w, screen_h));
    for second in 1..=5 {
        std::thread::sleep(Duration::from_secs(1));
        eprintln!(
            "after {second} s: {:?}, running {}",
            engine.stats(),
            engine.is_running()
        );
    }
    let saved = match clips.save_last(Duration::from_secs(3)) {
        Ok(saved) => saved,
        Err(e) => panic!("no clip: {e}; engine {:?}", engine.stats()),
    };
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    wiggler.join().unwrap();
    engine.stop();

    let stats = engine.stats();
    eprintln!("engine stats: {stats:?}");
    eprintln!(
        "saved {} ({} packets, audio {}, {:?})",
        saved.path.display(),
        saved.packets,
        saved.audio_packets,
        saved.duration
    );
    assert_eq!(stats.encode_errors, 0);
    assert!(saved.packets >= 20, "{} video packets", saved.packets);

    let decoded = decode_with_windows(&saved.path);
    eprintln!(
        "windows decoded {} frames {}x{} over {:?}",
        decoded.frames, decoded.width, decoded.height, decoded.duration
    );
    assert_eq!((decoded.width, decoded.height), (width, height));
    assert_eq!(decoded.frames, saved.packets as u64);
}

// ---- a still picture -------------------------------------------------------------

#[test]
#[ignore = "needs a Windows desktop session"]
fn a_screenshot_is_a_png_of_the_whole_screen() {
    use clipforge_core::screenshot::{capture_png, png_dimensions};

    let (screen_w, screen_h) = primary_display_size().expect("primary display");
    let path = out_dir().join("captura-test.png");
    let shot = capture_png(&path).expect("a screenshot");
    eprintln!(
        "screenshot {}x{} of a {screen_w}x{screen_h} screen",
        shot.width, shot.height
    );

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(png_dimensions(&bytes), Some((shot.width, shot.height)));
    assert_eq!(
        (shot.width, shot.height),
        (screen_w, screen_h),
        "at the screen's own size"
    );
    assert!(bytes.len() > 5_000, "{} bytes", bytes.len());
    assert!(
        !std::fs::read_dir(path.parent().unwrap()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")),
        "no temporary file left"
    );
}

#[test]
#[ignore = "needs a Windows desktop session"]
fn a_still_screen_still_gives_a_first_frame() {
    // Nobody moves the mouse: capture must not wait for the screen to change.
    let started = Instant::now();
    let (data, width, height) =
        clipforge_core::capture::windows::capture_one_frame(Duration::from_secs(3))
            .expect("a frame of an idle screen");
    eprintln!(
        "first frame of an idle screen after {:?}",
        started.elapsed()
    );
    assert_eq!(data.len(), width as usize * height as usize * 4);
}

// ---- the recorder, with the sound of the computer -----------------------------------

/// What Windows' decoder makes of the first audio track: 16-bit PCM samples,
/// interleaved, with their rate and channel count.
struct DecodedAudio {
    sample_rate: u32,
    channels: u32,
    samples: Vec<i16>,
}

fn decode_audio_with_windows(path: &Path) -> DecodedAudio {
    use windows::Win32::Media::MediaFoundation::{
        MFAudioFormat_PCM, MFMediaType_Audio, MF_MT_AUDIO_NUM_CHANNELS,
        MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
    };
    // SAFETY: plain Media Foundation calls; everything is dropped before MFShutdown.
    unsafe {
        MFStartup(MF_VERSION, MFSTARTUP_FULL).expect("MFStartup");
        let decode = || {
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None)
                .expect("Windows can open the mp4");
            let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
            let wanted = MFCreateMediaType().unwrap();
            wanted
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                .unwrap();
            wanted.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).unwrap();
            reader
                .SetCurrentMediaType(stream, None, &wanted)
                .expect("Windows can decode the AAC track");
            let current = reader.GetCurrentMediaType(stream).unwrap();
            let sample_rate = current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap();
            let channels = current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap();
            let mut samples = Vec::new();
            loop {
                let (mut flags, mut sample) = (0u32, None);
                reader
                    .ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample))
                    .expect("read decoded audio");
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(sample) = sample else { continue };
                let buffer = sample.ConvertToContiguousBuffer().unwrap();
                let (mut ptr, mut len) = (std::ptr::null_mut(), 0u32);
                buffer.Lock(&mut ptr, None, Some(&mut len)).unwrap();
                let bytes = std::slice::from_raw_parts(ptr, len as usize);
                samples.extend(
                    bytes
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]])),
                );
                buffer.Unlock().unwrap();
            }
            DecodedAudio {
                sample_rate,
                channels,
                samples,
            }
        };
        let decoded = decode();
        let _ = MFShutdown();
        decoded
    }
}

fn tone_amplitude(audio: &DecodedAudio, hz: f32) -> f32 {
    let (mut re, mut im, mut n) = (0f64, 0f64, 0usize);
    for (i, frame) in audio
        .samples
        .chunks_exact(audio.channels as usize)
        .enumerate()
    {
        let mono =
            frame.iter().map(|&s| f64::from(s) / 32768.0).sum::<f64>() / f64::from(audio.channels);
        let phase =
            2.0 * std::f64::consts::PI * f64::from(hz) * i as f64 / f64::from(audio.sample_rate);
        re += mono * phase.cos();
        im += mono * phase.sin();
        n += 1;
    }
    (2.0 * (re * re + im * im).sqrt() / n.max(1) as f64) as f32
}

#[test]
#[ignore = "needs a Windows desktop session, an audio output device and the Media Foundation encoders"]
fn the_recorder_saves_a_clip_with_the_sound_that_was_playing() {
    let (screen_w, screen_h) = primary_display_size().expect("primary display");
    let (width, height) = fit_even(screen_w, screen_h, 1920, 1080);
    let config = EncoderConfig {
        width,
        height,
        bitrate: 8_000_000,
        ..EncoderConfig::default()
    };

    // a 440 Hz tone, played by another process into the default output device
    let wav = out_dir().join("tone-440.wav");
    {
        let rate = 48_000u32;
        let mut pcm = Vec::new();
        for n in 0..rate * 8 {
            let v = (0.5
                * (2.0 * std::f32::consts::PI * 440.0 * n as f32 / rate as f32).sin()
                * 32767.0) as i16;
            pcm.extend(v.to_le_bytes());
            pcm.extend(v.to_le_bytes());
        }
        let mut file = Vec::new();
        file.extend(b"RIFF");
        file.extend((36 + pcm.len() as u32).to_le_bytes());
        file.extend(b"WAVEfmt ");
        file.extend(16u32.to_le_bytes());
        file.extend([1, 0, 2, 0]);
        file.extend(rate.to_le_bytes());
        file.extend((rate * 4).to_le_bytes());
        file.extend([4, 0, 16, 0]);
        file.extend(b"data");
        file.extend((pcm.len() as u32).to_le_bytes());
        file.extend(pcm);
        std::fs::write(&wav, file).unwrap();
    }

    let epoch = SharedEpoch::new();
    let video = WindowsGraphicsCaptureSource::new(
        WindowsCaptureConfig {
            width,
            height,
            fps: config.fps,
            show_cursor: true,
        },
        epoch.clone(),
    );
    let audio = clipforge_core::audio::wasapi::WasapiLoopbackSource::new(
        clipforge_core::audio::AudioConfig::default(),
        epoch,
    );
    let source = WithAudio::new(video, audio);
    let encoder = MediaFoundationEncoder::new(config).expect("H.264 encoder MFT");
    let audio_encoder: Box<dyn clipforge_core::audio::AudioEncoder> = Box::new(
        clipforge_core::audio::aac_mediafoundation::MediaFoundationAacEncoder::new(
            clipforge_core::audio::AudioConfig::default(),
        )
        .expect("AAC encoder"),
    );

    let mut engine = CaptureEngine::new(BufferConfig::default());
    let clips = Arc::new(ClipService::new(
        engine.buffer(),
        out_dir().join("clips-with-sound"),
    ));
    engine
        .start_with_audio(
            Box::new(source),
            Box::new(encoder),
            Some(audio_encoder),
            Arc::clone(&clips),
        )
        .expect("the engine starts");
    let mut player = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(New-Object Media.SoundPlayer '{}').PlaySync()",
                wav.display()
            ),
        ])
        .spawn()
        .expect("powershell plays the tone");

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let wiggler = wiggle_cursor(Arc::clone(&stop), (screen_w, screen_h));
    std::thread::sleep(Duration::from_secs(6));
    let saved = clips
        .save_last(Duration::from_secs(4))
        .expect("a clip is saved");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    wiggler.join().unwrap();
    let _ = player.kill();
    let _ = player.wait();
    engine.stop();

    let stats = engine.stats();
    eprintln!("engine {stats:?}");
    eprintln!(
        "saved {} ({} video packets, {} audio packets, {:?})",
        saved.path.display(),
        saved.packets,
        saved.audio_packets,
        saved.duration
    );
    assert_eq!(stats.encode_errors, 0);
    assert_eq!(stats.audio_errors, 0);
    assert!(
        saved.audio_packets > 100,
        "{} audio packets: the sound was not recorded",
        saved.audio_packets
    );

    let video = decode_with_windows(&saved.path);
    assert_eq!(video.frames, saved.packets as u64);
    let sound = decode_audio_with_windows(&saved.path);
    let seconds =
        sound.samples.len() as f64 / f64::from(sound.channels) / f64::from(sound.sample_rate);
    let (at_440, at_1000) = (
        tone_amplitude(&sound, 440.0),
        tone_amplitude(&sound, 1000.0),
    );
    eprintln!(
        "windows decoded audio: {} Hz x{}, {seconds:.2} s; 440 Hz {at_440:.3}, 1 kHz {at_1000:.3}; video {:?}",
        sound.sample_rate, sound.channels, video.duration
    );
    assert_eq!(sound.sample_rate, 48_000);
    // Sound only exists from the moment something plays (the tone starts a
    // little after the recording does), so the audio may be shorter than the
    // video by that lead-in, never longer.
    assert!(
        seconds <= saved.duration.as_secs_f64() + 0.3
            && seconds >= saved.duration.as_secs_f64() - 2.0,
        "the sound lasts {seconds:.2} s, the clip {:?}",
        saved.duration
    );
    assert!(at_440 > 0.02, "the tone is not in the clip: {at_440}");
    assert!(
        at_440 > at_1000 * 10.0,
        "440 Hz should dominate: {at_440} vs {at_1000}"
    );
}
