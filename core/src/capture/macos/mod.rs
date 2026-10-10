//! macOS screen capture via ScreenCaptureKit.
//!
//! [`ScreenCaptureKitSource`] captures the main display and implements
//! [`FrameSource`]: SCK delivers BGRA `CMSampleBuffer`s on its own dispatch
//! queue; the handler repacks them into tightly-packed [`RawFrame`]s and
//! pushes them into the async stream. The callback never blocks — if the
//! consumer lags, frames are dropped and counted.
//!
//! The same stream also carries the system audio (everything the Mac plays,
//! minus ClipForge itself). Audio chunks are re-based onto the video epoch
//! with [`SharedEpoch`] and exposed through [`FrameSource::take_audio`].

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use screencapturekit::cm::CMTime;
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::error::SCError;
use screencapturekit::prelude::*;
use tokio::sync::mpsc;

use super::clock::SharedEpoch;
use super::{CaptureError, CaptureStats, FrameSource, RawFrame};
use crate::audio::AudioChunk;

/// Frames buffered between the SCK callback queue and the consumer. Small on
/// purpose: at 60 fps, 8 frames ≈ 130 ms of slack before we start dropping.
const CHANNEL_CAPACITY: usize = 8;

/// Audio chunks buffered between the SCK callback queue and the encoder
/// (SCK delivers roughly one buffer every 10-20 ms).
const AUDIO_CHANNEL_CAPACITY: usize = 128;
const AUDIO_SAMPLE_RATE: u32 = 48_000;
const AUDIO_CHANNELS: u16 = 2;

#[derive(Debug, Clone)]
pub struct ScreenCaptureConfig {
    /// Output size in pixels; `None` captures at the display's native size.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: u32,
    pub show_cursor: bool,
    /// Also capture system audio (48 kHz stereo), available through
    /// [`FrameSource::take_audio`].
    pub capture_audio: bool,
}

impl Default for ScreenCaptureConfig {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            fps: 60,
            show_cursor: true,
            capture_audio: true,
        }
    }
}

/// [`FrameSource`] for the main display, backed by ScreenCaptureKit.
pub struct ScreenCaptureKitSource {
    config: ScreenCaptureConfig,
    stream: Option<SCStream>,
    delivered: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    audio_rx: Option<mpsc::Receiver<AudioChunk>>,
    audio_dropped: Arc<AtomicU64>,
    epoch: Option<SharedEpoch>,
}

impl ScreenCaptureKitSource {
    pub fn new(config: ScreenCaptureConfig) -> Self {
        Self {
            config,
            stream: None,
            delivered: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
            audio_rx: None,
            audio_dropped: Arc::new(AtomicU64::new(0)),
            epoch: None,
        }
    }

    /// Audio chunks SCK produced that we had to drop (encoder lagging).
    pub fn audio_dropped(&self) -> u64 {
        self.audio_dropped.load(Ordering::SeqCst)
    }
}

fn map_sc_error(err: SCError) -> CaptureError {
    match err {
        // SCK reports a TCC (permission) denial as "no shareable content".
        SCError::NoShareableContent(_) => CaptureError::PermissionDenied,
        SCError::DisplayNotFound(_) => CaptureError::NoDisplay,
        other => CaptureError::Backend(other.to_string()),
    }
}

fn cmtime_nanos(t: CMTime) -> i64 {
    if t.timescale == 0 {
        return 0;
    }
    (t.value as i128 * 1_000_000_000 / t.timescale as i128) as i64
}

/// SCK output handler: repacks each complete frame and forwards it.
struct FrameForwarder {
    tx: mpsc::Sender<RawFrame>,
    delivered: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    /// pts of the first delivered frame, in ns. Timestamps handed downstream
    /// (video *and* audio) are relative to this epoch.
    epoch: SharedEpoch,
    /// Last relative timestamp delivered, in ns; ensures strict monotonicity.
    last_ns: AtomicI64,
    frame_number: AtomicU64,
}

impl FrameForwarder {
    fn forward(&self, sample: &CMSampleBuffer) -> Option<()> {
        // Idle/suspended frames (screen unchanged, display asleep) carry no
        // new image; skip them. A missing status attachment (seen on newer
        // macOS) is not a reason to discard a frame that has pixels.
        if sample.frame_status().is_some_and(|s| !s.is_complete()) {
            return None;
        }
        let pixel_buffer = sample.image_buffer()?;
        let pts_ns = cmtime_nanos(sample.output_presentation_timestamp());

        let epoch = self.epoch.get_or_set(pts_ns);
        let rel_ns = pts_ns - epoch;
        if rel_ns < 0 || rel_ns <= self.last_ns.load(Ordering::SeqCst) {
            // Out-of-order or duplicate pts: drop rather than break the
            // monotonic-timestamp contract (first frame is exempt via -1).
            if self.frame_number.load(Ordering::SeqCst) > 0 {
                self.dropped.fetch_add(1, Ordering::SeqCst);
                return None;
            }
        }

        let guard = pixel_buffer.lock(CVPixelBufferLockFlags::READ_ONLY).ok()?;
        let (width, height) = (guard.width(), guard.height());
        let stride = guard.bytes_per_row();
        let row_len = width * 4;
        let src = guard.as_slice();
        if src.len() < stride * height || row_len > stride {
            self.dropped.fetch_add(1, Ordering::SeqCst);
            return None;
        }
        // Repack to tightly-packed BGRA (SCK pads rows to stride).
        let mut data = Vec::with_capacity(row_len * height);
        for row in 0..height {
            let start = row * stride;
            data.extend_from_slice(&src[start..start + row_len]);
        }
        drop(guard);

        let frame = RawFrame {
            data,
            width: width as u32,
            height: height as u32,
            timestamp: Duration::from_nanos(rel_ns.max(0) as u64),
            frame_number: self.frame_number.load(Ordering::SeqCst),
        };

        match self.tx.try_send(frame) {
            Ok(()) => {
                self.last_ns.store(rel_ns, Ordering::SeqCst);
                self.frame_number.fetch_add(1, Ordering::SeqCst);
                self.delivered.fetch_add(1, Ordering::SeqCst);
                Some(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::SeqCst);
                None
            }
            Err(mpsc::error::TrySendError::Closed(_)) => None,
        }
    }
}

impl SCStreamOutputTrait for FrameForwarder {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if std::env::var_os("CLIPFORGE_CAPTURE_DEBUG").is_some() {
            eprintln!(
                "[capture-debug] type={of_type:?} status={:?} has_image={}",
                sample.frame_status(),
                sample.image_buffer().is_some()
            );
        }
        if of_type == SCStreamOutputType::Screen {
            self.forward(&sample);
        }
    }
}

/// Turn the buffers of a captured audio sample into interleaved `f32`.
///
/// SCK delivers 32-bit float PCM, normally *planar* (one single-channel
/// buffer per channel); an interleaved single buffer is handled too. `None`
/// for any other layout, so unexpected audio is skipped rather than garbled.
fn interleave_f32(buffers: &[(u32, &[u8])], channels: usize) -> Option<Vec<f32>> {
    let to_f32 = |bytes: &[u8]| -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    };
    match buffers {
        [(n, bytes)] if *n as usize == channels => Some(to_f32(bytes)),
        planar if planar.len() == channels && planar.iter().all(|(n, _)| *n == 1) => {
            let planes: Vec<Vec<f32>> = planar.iter().map(|(_, b)| to_f32(b)).collect();
            let frames = planes.iter().map(Vec::len).min()?;
            let mut out = Vec::with_capacity(frames * channels);
            for i in 0..frames {
                out.extend(planes.iter().map(|plane| plane[i]));
            }
            Some(out)
        }
        _ => None,
    }
}

/// SCK output handler for system audio: converts each sample buffer into an
/// [`AudioChunk`] on the video epoch and forwards it without ever blocking.
struct AudioForwarder {
    tx: mpsc::Sender<AudioChunk>,
    epoch: SharedEpoch,
    dropped: Arc<AtomicU64>,
}

impl AudioForwarder {
    fn forward(&self, sample: &CMSampleBuffer) -> Option<()> {
        // Audio from before the first video frame has no place in a clip.
        let timestamp = self
            .epoch
            .relative(cmtime_nanos(sample.output_presentation_timestamp()))?;
        let list = sample.audio_buffer_list()?;
        let buffers: Vec<(u32, &[u8])> =
            list.iter().map(|b| (b.number_channels, b.data())).collect();
        // Only 32-bit float PCM is understood: any other sample width would
        // turn into noise, so verify the byte count before trusting it.
        let frames = usize::try_from(sample.num_samples()).ok()?;
        let total_bytes: usize = buffers.iter().map(|(_, bytes)| bytes.len()).sum();
        if frames == 0 || total_bytes != frames * usize::from(AUDIO_CHANNELS) * 4 {
            self.dropped.fetch_add(1, Ordering::SeqCst);
            return None;
        }
        let samples = interleave_f32(&buffers, usize::from(AUDIO_CHANNELS))?;
        let chunk = AudioChunk {
            samples,
            channels: AUDIO_CHANNELS,
            sample_rate: AUDIO_SAMPLE_RATE,
            timestamp,
        };
        match self.tx.try_send(chunk) {
            Ok(()) => Some(()),
            Err(_) => {
                self.dropped.fetch_add(1, Ordering::SeqCst);
                None
            }
        }
    }
}

impl SCStreamOutputTrait for AudioForwarder {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if of_type == SCStreamOutputType::Audio {
            self.forward(&sample);
        }
    }
}

impl FrameSource for ScreenCaptureKitSource {
    fn start(&mut self) -> Result<mpsc::Receiver<RawFrame>, CaptureError> {
        if self.stream.is_some() {
            return Err(CaptureError::AlreadyStarted);
        }
        if self.config.fps == 0 {
            return Err(CaptureError::Backend("fps must be non-zero".into()));
        }

        let content = SCShareableContent::get().map_err(map_sc_error)?;
        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or(CaptureError::NoDisplay)?;

        let width = self.config.width.unwrap_or_else(|| display.width());
        let height = self.config.height.unwrap_or_else(|| display.height());

        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();
        let mut sc_config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(self.config.show_cursor)
            .with_minimum_frame_interval(&CMTime::new(1, self.config.fps as i32));
        if self.config.capture_audio {
            sc_config = sc_config
                .with_captures_audio(true)
                .with_sample_rate(AUDIO_SAMPLE_RATE as i32)
                .with_channel_count(i32::from(AUDIO_CHANNELS))
                // Don't record ClipForge's own notification sounds.
                .with_excludes_current_process_audio(true);
        }

        self.delivered.store(0, Ordering::SeqCst);
        self.dropped.store(0, Ordering::SeqCst);
        self.audio_dropped.store(0, Ordering::SeqCst);

        let epoch = SharedEpoch::new();
        self.epoch = Some(epoch.clone());
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let handler = FrameForwarder {
            tx,
            delivered: Arc::clone(&self.delivered),
            dropped: Arc::clone(&self.dropped),
            epoch: epoch.clone(),
            last_ns: AtomicI64::new(-1),
            frame_number: AtomicU64::new(0),
        };

        let mut stream = SCStream::new(&filter, &sc_config);
        stream.add_output_handler(handler, SCStreamOutputType::Screen);
        self.audio_rx = None;
        if self.config.capture_audio {
            let (audio_tx, audio_rx) = mpsc::channel(AUDIO_CHANNEL_CAPACITY);
            stream.add_output_handler(
                AudioForwarder {
                    tx: audio_tx,
                    epoch,
                    dropped: Arc::clone(&self.audio_dropped),
                },
                SCStreamOutputType::Audio,
            );
            self.audio_rx = Some(audio_rx);
        }
        stream.start_capture().map_err(|e| match map_sc_error(e) {
            CaptureError::Backend(msg) if msg.to_lowercase().contains("declined") => {
                CaptureError::PermissionDenied
            }
            other => other,
        })?;

        self.stream = Some(stream);
        Ok(rx)
    }

    fn stop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.stop_capture();
        }
    }

    fn stats(&self) -> CaptureStats {
        CaptureStats {
            delivered: self.delivered.load(Ordering::SeqCst),
            dropped: self.dropped.load(Ordering::SeqCst),
        }
    }

    fn take_audio(&mut self) -> Option<mpsc::Receiver<AudioChunk>> {
        self.audio_rx.take()
    }

    fn epoch(&self) -> Option<SharedEpoch> {
        self.epoch.clone()
    }
}

impl Drop for ScreenCaptureKitSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn planar_buffers_are_interleaved_frame_by_frame() {
        let left = bytes(&[1.0, 2.0, 3.0]);
        let right = bytes(&[-1.0, -2.0, -3.0]);
        let out = interleave_f32(&[(1, &left), (1, &right)], 2).unwrap();
        assert_eq!(out, [1.0, -1.0, 2.0, -2.0, 3.0, -3.0]);
    }

    #[test]
    fn an_interleaved_buffer_passes_through() {
        let both = bytes(&[0.5, -0.5, 0.25, -0.25]);
        assert_eq!(
            interleave_f32(&[(2, &both)], 2).unwrap(),
            [0.5, -0.5, 0.25, -0.25]
        );
    }

    #[test]
    fn planes_of_different_length_use_the_shorter_one() {
        let left = bytes(&[1.0, 2.0, 3.0]);
        let right = bytes(&[9.0, 8.0]);
        assert_eq!(
            interleave_f32(&[(1, &left), (1, &right)], 2).unwrap(),
            [1.0, 9.0, 2.0, 8.0]
        );
    }

    #[test]
    fn unexpected_layouts_are_skipped_not_garbled() {
        let one = bytes(&[1.0, 2.0]);
        assert!(interleave_f32(&[], 2).is_none());
        assert!(
            interleave_f32(&[(1, &one)], 2).is_none(),
            "mono for a stereo stream"
        );
        assert!(interleave_f32(&[(1, &one), (1, &one), (1, &one)], 2).is_none());
        assert!(interleave_f32(&[(2, &one), (2, &one)], 2).is_none());
    }

    #[test]
    fn partial_trailing_bytes_are_ignored() {
        let mut left = bytes(&[1.0, 2.0]);
        left.push(0xAA); // a torn 5th byte
        let right = bytes(&[3.0, 4.0]);
        assert_eq!(
            interleave_f32(&[(1, &left), (1, &right)], 2).unwrap(),
            [1.0, 3.0, 2.0, 4.0]
        );
    }
}
