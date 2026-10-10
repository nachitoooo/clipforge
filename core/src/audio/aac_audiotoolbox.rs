//! macOS AAC-LC encoding with AudioToolbox's `AudioConverter`.
//!
//! [`AudioToolboxAacEncoder`] implements [`AudioEncoder`]: interleaved `f32`
//! PCM goes in, raw AAC access units (what an mp4 `mp4a` track stores) come
//! out. The API is a C pull model — the converter asks an input callback for
//! PCM while producing each packet — so the encoder keeps a small pending
//! buffer and only calls into the converter when a full 1024-sample frame is
//! available.
//!
//! **Encoder delay.** AAC encoders delay the signal by ~2112 samples
//! (`kAudioConverterPrimeInfo`). Packets keep the plain capture time of the
//! input they encode; [`AudioEncoder::priming_frames`] reports the delay and
//! the muxer writes an edit list that skips it, so every player starts at
//! the right instant.

use std::ffi::c_void;
use std::ptr;
use std::time::Duration;

use super::{
    frames_to_duration, AudioChunk, AudioConfig, AudioEncoder, AudioError, AudioPacket,
    AAC_FRAME_SAMPLES,
};

type OSStatus = i32;
type AudioConverterRef = *mut c_void;

const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

const FORMAT_LINEAR_PCM: u32 = fourcc(b"lpcm");
const FORMAT_MPEG4_AAC: u32 = fourcc(b"aac ");
const FLAG_IS_FLOAT: u32 = 1 << 0;
const FLAG_IS_PACKED: u32 = 1 << 3;
/// `kMPEG4Object_AAC_LC`, carried in the format flags of the AAC description.
const MPEG4_OBJECT_AAC_LC: u32 = 2;

const PROP_ENCODE_BIT_RATE: u32 = fourcc(b"brat");
const PROP_PRIME_INFO: u32 = fourcc(b"prim");
const PROP_MAX_OUTPUT_PACKET_SIZE: u32 = fourcc(b"xops");

/// Returned from the input callback when it has nothing more to give *for
/// now*. Not an error: the converter hands back what it has produced so far.
const STATUS_NEED_MORE_INPUT: OSStatus = 0x6E6F_6D72; // 'nomr'

/// Priming of Apple's AAC-LC encoder, used if the property can't be read.
const DEFAULT_LEADING_FRAMES: u32 = 2112;
const FALLBACK_MAX_PACKET_BYTES: usize = 8192;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 1],
}

#[repr(C)]
#[derive(Default)]
struct AudioStreamPacketDescription {
    start_offset: i64,
    variable_frames_in_packet: u32,
    data_byte_size: u32,
}

#[repr(C)]
#[derive(Default)]
struct AudioConverterPrimeInfo {
    leading_frames: u32,
    trailing_frames: u32,
}

type InputProc = unsafe extern "C" fn(
    AudioConverterRef,
    *mut u32,
    *mut AudioBufferList,
    *mut *mut AudioStreamPacketDescription,
    *mut c_void,
) -> OSStatus;

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioConverterNew(
        source: *const AudioStreamBasicDescription,
        destination: *const AudioStreamBasicDescription,
        converter: *mut AudioConverterRef,
    ) -> OSStatus;
    fn AudioConverterDispose(converter: AudioConverterRef) -> OSStatus;
    fn AudioConverterReset(converter: AudioConverterRef) -> OSStatus;
    fn AudioConverterSetProperty(
        converter: AudioConverterRef,
        property: u32,
        size: u32,
        data: *const c_void,
    ) -> OSStatus;
    fn AudioConverterGetProperty(
        converter: AudioConverterRef,
        property: u32,
        size: *mut u32,
        data: *mut c_void,
    ) -> OSStatus;
    fn AudioConverterFillComplexBuffer(
        converter: AudioConverterRef,
        input_proc: InputProc,
        user_data: *mut c_void,
        io_output_packets: *mut u32,
        output: *mut AudioBufferList,
        packet_descriptions: *mut AudioStreamPacketDescription,
    ) -> OSStatus;
}

/// What the input callback may hand to the converter during one
/// `AudioConverterFillComplexBuffer` call.
struct InputState {
    samples: *const f32,
    total_frames: usize,
    /// Frames already given away during this call.
    offset: usize,
    channels: u32,
    /// Pending data is exhausted *and* no more will come: tells the converter
    /// to flush instead of waiting for input.
    end_of_stream: bool,
}

unsafe extern "C" fn supply_input(
    _converter: AudioConverterRef,
    io_packets: *mut u32,
    io_data: *mut AudioBufferList,
    out_packet_descriptions: *mut *mut AudioStreamPacketDescription,
    user_data: *mut c_void,
) -> OSStatus {
    // SAFETY: `user_data` is the `InputState` that `fill_one` keeps alive for
    // the duration of the enclosing `AudioConverterFillComplexBuffer` call;
    // `io_packets` / `io_data` are valid for this callback per the API.
    let state = &mut *user_data.cast::<InputState>();
    let remaining = state.total_frames - state.offset;
    if remaining == 0 {
        *io_packets = 0;
        (*io_data).number_buffers = 1;
        (*io_data).buffers[0].data_byte_size = 0;
        (*io_data).buffers[0].data = ptr::null_mut();
        return if state.end_of_stream {
            0
        } else {
            STATUS_NEED_MORE_INPUT
        };
    }
    let give = (*io_packets as usize).min(remaining);
    (*io_data).number_buffers = 1;
    (*io_data).buffers[0] = AudioBuffer {
        number_channels: state.channels,
        data_byte_size: (give * state.channels as usize * std::mem::size_of::<f32>()) as u32,
        // The converter only reads from this buffer.
        data: state
            .samples
            .add(state.offset * state.channels as usize)
            .cast_mut()
            .cast(),
    };
    *io_packets = give as u32;
    if !out_packet_descriptions.is_null() {
        *out_packet_descriptions = ptr::null_mut();
    }
    state.offset += give;
    0
}

pub struct AudioToolboxAacEncoder {
    converter: AudioConverterRef,
    config: AudioConfig,
    /// Interleaved samples not yet consumed by the converter.
    pending: Vec<f32>,
    /// Timestamp of the first sample fed since the last (re)start.
    base: Option<Duration>,
    /// AAC packets produced since the last (re)start.
    packets_out: u64,
    leading_frames: u32,
    max_packet_bytes: usize,
    finished: bool,
}

// SAFETY: the converter is only ever touched through `&mut self`, so it is
// used by one thread at a time; AudioConverter has no thread affinity.
unsafe impl Send for AudioToolboxAacEncoder {}

impl AudioToolboxAacEncoder {
    pub fn new(config: AudioConfig) -> Result<Self, AudioError> {
        if config.channels == 0 || config.channels > 2 {
            return Err(AudioError::Unsupported(format!(
                "{} channels (AAC-LC here is mono or stereo)",
                config.channels
            )));
        }
        let channels = u32::from(config.channels);
        let pcm = AudioStreamBasicDescription {
            sample_rate: f64::from(config.sample_rate),
            format_id: FORMAT_LINEAR_PCM,
            format_flags: FLAG_IS_FLOAT | FLAG_IS_PACKED,
            bytes_per_packet: 4 * channels,
            frames_per_packet: 1,
            bytes_per_frame: 4 * channels,
            channels_per_frame: channels,
            bits_per_channel: 32,
            reserved: 0,
        };
        let aac = AudioStreamBasicDescription {
            sample_rate: f64::from(config.sample_rate),
            format_id: FORMAT_MPEG4_AAC,
            format_flags: MPEG4_OBJECT_AAC_LC,
            bytes_per_packet: 0,
            frames_per_packet: AAC_FRAME_SAMPLES,
            bytes_per_frame: 0,
            channels_per_frame: channels,
            bits_per_channel: 0,
            reserved: 0,
        };

        let mut converter: AudioConverterRef = ptr::null_mut();
        // SAFETY: both descriptions are valid for the call; `converter` is a
        // valid out-pointer.
        let status = unsafe { AudioConverterNew(&pcm, &aac, &mut converter) };
        if status != 0 || converter.is_null() {
            return Err(AudioError::Unsupported(format!(
                "no AAC encoder for {} Hz x{} (AudioConverterNew: {status})",
                config.sample_rate, config.channels
            )));
        }
        // From here on `Drop` releases the converter, including on the
        // early-error paths below.
        let mut encoder = Self {
            converter,
            config,
            pending: Vec::new(),
            base: None,
            packets_out: 0,
            leading_frames: DEFAULT_LEADING_FRAMES,
            max_packet_bytes: FALLBACK_MAX_PACKET_BYTES,
            finished: false,
        };

        let bitrate = config.bitrate;
        // SAFETY: `bitrate` is a live u32 of the size we pass.
        let status = unsafe {
            AudioConverterSetProperty(
                converter,
                PROP_ENCODE_BIT_RATE,
                std::mem::size_of::<u32>() as u32,
                (&bitrate as *const u32).cast(),
            )
        };
        if status != 0 {
            return Err(AudioError::Unsupported(format!(
                "{bitrate} b/s is not a valid AAC bitrate here (status {status})"
            )));
        }

        let mut max_size: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: out-pointers are live and sized as declared.
        let status = unsafe {
            AudioConverterGetProperty(
                converter,
                PROP_MAX_OUTPUT_PACKET_SIZE,
                &mut size,
                (&mut max_size as *mut u32).cast(),
            )
        };
        if status == 0 && max_size > 0 {
            encoder.max_packet_bytes = max_size as usize;
        }

        let mut prime = AudioConverterPrimeInfo::default();
        let mut size = std::mem::size_of::<AudioConverterPrimeInfo>() as u32;
        // SAFETY: as above.
        let status = unsafe {
            AudioConverterGetProperty(
                converter,
                PROP_PRIME_INFO,
                &mut size,
                (&mut prime as *mut AudioConverterPrimeInfo).cast(),
            )
        };
        if status == 0 && prime.leading_frames > 0 {
            encoder.leading_frames = prime.leading_frames;
        }
        Ok(encoder)
    }

    fn channels(&self) -> usize {
        usize::from(self.config.channels)
    }

    /// Ask the converter for one packet. Returns the packet (if produced) and
    /// how many pending frames the converter consumed.
    fn fill_one(&mut self, end_of_stream: bool) -> Result<(Option<Vec<u8>>, usize), AudioError> {
        let channels = self.channels();
        let mut state = InputState {
            samples: self.pending.as_ptr(),
            total_frames: self.pending.len() / channels,
            offset: 0,
            channels: channels as u32,
            end_of_stream,
        };
        let mut out_bytes = vec![0u8; self.max_packet_bytes];
        let mut out_list = AudioBufferList {
            number_buffers: 1,
            buffers: [AudioBuffer {
                number_channels: channels as u32,
                data_byte_size: out_bytes.len() as u32,
                data: out_bytes.as_mut_ptr().cast(),
            }],
        };
        let mut description = AudioStreamPacketDescription::default();
        let mut packets: u32 = 1;

        // SAFETY: `state`, `out_list`, `out_bytes` and `description` outlive
        // the call; the callback only reads `pending` through `state`, which
        // is not modified until the call returns.
        let status = unsafe {
            AudioConverterFillComplexBuffer(
                self.converter,
                supply_input,
                (&mut state as *mut InputState).cast(),
                &mut packets,
                &mut out_list,
                &mut description,
            )
        };
        if packets == 0 && status != 0 && status != STATUS_NEED_MORE_INPUT {
            return Err(AudioError::Backend(format!(
                "AudioConverterFillComplexBuffer failed: {status}"
            )));
        }
        let packet = (packets > 0).then(|| {
            out_bytes.truncate(out_list.buffers[0].data_byte_size as usize);
            out_bytes
        });
        Ok((packet, state.offset))
    }

    /// Stamp a raw converter packet: packet `k` encodes the input starting
    /// `k * 1024` samples after the first one.
    fn timestamped(&mut self, data: Vec<u8>) -> AudioPacket {
        let k = self.packets_out;
        self.packets_out += 1;
        let frame = u64::from(AAC_FRAME_SAMPLES);
        AudioPacket {
            data,
            pts: self.base.unwrap_or_default()
                + frames_to_duration(k * frame, self.config.sample_rate),
            duration: frames_to_duration(frame, self.config.sample_rate),
        }
    }

    /// Encode every complete 1024-sample frame in `pending`.
    fn pump(&mut self) -> Result<Vec<AudioPacket>, AudioError> {
        let channels = self.channels();
        let mut out = Vec::new();
        while self.pending.len() / channels >= AAC_FRAME_SAMPLES as usize {
            let (packet, consumed) = self.fill_one(false)?;
            self.pending.drain(..consumed * channels);
            match packet {
                Some(data) => out.push(self.timestamped(data)),
                None if consumed == 0 => break, // no progress: wait for more input
                None => {}
            }
        }
        Ok(out)
    }
}

impl AudioEncoder for AudioToolboxAacEncoder {
    fn encode(&mut self, chunk: &AudioChunk) -> Result<Vec<AudioPacket>, AudioError> {
        if self.finished {
            return Err(AudioError::Backend("encode after finish".into()));
        }
        if chunk.sample_rate != self.config.sample_rate || chunk.channels != self.config.channels {
            return Err(AudioError::BadChunk(format!(
                "{} Hz x{} chunk for {} Hz x{} encoder",
                chunk.sample_rate, chunk.channels, self.config.sample_rate, self.config.channels
            )));
        }
        if self.base.is_none() {
            self.base = Some(chunk.timestamp);
        }
        self.pending.extend_from_slice(&chunk.samples);
        self.pump()
    }

    fn finish(&mut self) -> Result<Vec<AudioPacket>, AudioError> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        let channels = self.channels();
        let frame = AAC_FRAME_SAMPLES as usize;
        let frames = self.pending.len() / channels;
        if !frames.is_multiple_of(frame) {
            // Pad the tail with silence so the last real samples get encoded.
            self.pending
                .resize((frames / frame + 1) * frame * channels, 0.0);
        }
        let mut out = self.pump()?;
        for _ in 0..8 {
            let (packet, _) = self.fill_one(true)?;
            match packet {
                Some(data) => out.push(self.timestamped(data)),
                None => break,
            }
        }
        Ok(out)
    }

    fn resync(&mut self) {
        // SAFETY: the converter is valid until `drop`.
        unsafe { AudioConverterReset(self.converter) };
        self.pending.clear();
        self.base = None;
        self.packets_out = 0;
    }

    fn priming_frames(&self) -> u32 {
        self.leading_frames
    }

    fn config(&self) -> &AudioConfig {
        &self.config
    }
}

impl Drop for AudioToolboxAacEncoder {
    fn drop(&mut self) {
        // SAFETY: the converter was created in `new` and is disposed once.
        unsafe { AudioConverterDispose(self.converter) };
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::audio::test_tone::{TestToneConfig, TestToneSource};
    use crate::clip::mp4mux::{write_mp4_with_audio, AudioParams, VideoParams};
    use crate::encode::{Codec, EncodedPacket, ParameterSets};

    const RATE: u32 = 48_000;

    fn encoder() -> AudioToolboxAacEncoder {
        AudioToolboxAacEncoder::new(AudioConfig::default()).expect("AAC encoder available")
    }

    /// 10 ms stereo chunks of `f(frame_index)` covering `seconds`.
    fn chunks(seconds: f64, base: Duration, f: impl Fn(u64) -> f32) -> Vec<AudioChunk> {
        let total = (seconds * f64::from(RATE)) as u64;
        (0..total / 480)
            .map(|i| {
                let first = i * 480;
                AudioChunk {
                    samples: (first..first + 480)
                        .flat_map(|n| {
                            let s = f(n);
                            [s, s]
                        })
                        .collect(),
                    channels: 2,
                    sample_rate: RATE,
                    timestamp: base + frames_to_duration(first, RATE),
                }
            })
            .collect()
    }

    fn encode_all(enc: &mut AudioToolboxAacEncoder, chunks: &[AudioChunk]) -> Vec<AudioPacket> {
        let mut packets = Vec::new();
        for chunk in chunks {
            packets.extend(enc.encode(chunk).unwrap());
        }
        packets.extend(enc.finish().unwrap());
        packets
    }

    fn tone(n: u64) -> f32 {
        TestToneSource::sample_at(&TestToneConfig::default(), n)
    }

    #[test]
    fn produces_valid_gap_free_aac_packets() {
        let mut enc = encoder();
        let packets = encode_all(&mut enc, &chunks(2.0, Duration::ZERO, tone));

        // 2 s = 93.75 frames of 1024: the tail is padded, then flushed.
        assert!(
            (92..=98).contains(&packets.len()),
            "{} packets",
            packets.len()
        );
        for p in &packets {
            assert!(
                (1..2000).contains(&p.data.len()),
                "implausible AAC size {}",
                p.data.len()
            );
            assert_eq!(p.duration, frames_to_duration(1024, RATE));
        }
        for pair in packets.windows(2) {
            let step = pair[1].pts - pair[0].pts;
            let nominal = frames_to_duration(1024, RATE);
            assert!(
                step.abs_diff(nominal) < Duration::from_micros(2),
                "step {step:?}"
            );
        }
    }

    #[test]
    fn busy_audio_averages_near_the_requested_bitrate() {
        // A pure tone is cheap to encode; noise forces the encoder to spend
        // its budget, so the 128 kb/s setting is actually exercised.
        let noise = |n: u64| ((n.wrapping_mul(2_654_435_761) >> 8) % 2000) as f32 / 3333.0 - 0.3;
        let mut enc = encoder();
        let packets = encode_all(&mut enc, &chunks(2.0, Duration::ZERO, noise));
        let bytes: usize = packets.iter().map(|p| p.data.len()).sum();
        let kbps = bytes as f64 * 8.0 / 2.0 / 1000.0;
        assert!((90.0..180.0).contains(&kbps), "{kbps} kb/s");
    }

    #[test]
    fn first_pts_is_the_first_sample_and_priming_is_reported() {
        let mut enc = encoder();
        let base = Duration::from_millis(500);
        let packets = encode_all(&mut enc, &chunks(1.0, base, tone));
        assert_eq!(packets[0].pts, base);
        // Apple's AAC-LC reports 2112 samples of delay.
        assert!(
            (1024..=4096).contains(&enc.priming_frames()),
            "{}",
            enc.priming_frames()
        );
    }

    #[test]
    fn rejects_mismatched_chunks_and_bad_configs() {
        let mut enc = encoder();
        let mut bad = chunks(0.1, Duration::ZERO, tone).remove(0);
        bad.sample_rate = 44_100;
        assert!(matches!(enc.encode(&bad), Err(AudioError::BadChunk(_))));

        assert!(AudioToolboxAacEncoder::new(AudioConfig {
            channels: 6,
            ..Default::default()
        })
        .is_err());
    }

    #[test]
    fn encode_after_finish_is_an_error_and_finish_is_idempotent() {
        let mut enc = encoder();
        enc.finish().unwrap();
        assert!(enc.finish().unwrap().is_empty());
        assert!(enc
            .encode(&chunks(0.1, Duration::ZERO, tone).remove(0))
            .is_err());
    }

    #[test]
    fn resync_restarts_the_timeline() {
        let mut enc = encoder();
        for c in chunks(0.5, Duration::ZERO, tone) {
            enc.encode(&c).unwrap();
        }
        enc.resync();
        let later = Duration::from_secs(90);
        let mut packets = Vec::new();
        for c in chunks(0.5, later, tone) {
            packets.extend(enc.encode(&c).unwrap());
        }
        assert!(!packets.is_empty());
        assert_eq!(packets[0].pts, later);
    }

    // ---- end to end through the muxer and the OS decoder ------------------

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("clipforge-aac-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// Mux `audio` under a dummy video track that starts at `video_start`
    /// and lasts `video_secs` (the muxer needs a video track).
    fn mux(
        path: &Path,
        audio: &[AudioPacket],
        priming: u32,
        video_start: Duration,
        video_secs: f64,
    ) {
        let frames = (video_secs * 60.0) as u64;
        let video: Vec<EncodedPacket> = (0..frames)
            .map(|n| {
                let pts = video_start + Duration::from_nanos(n * 1_000_000_000 / 60);
                EncodedPacket {
                    data: {
                        let mut d = vec![0, 0, 0, 5];
                        d.extend_from_slice(&[if n == 0 { 0x65 } else { 0x41 }; 5]);
                        d
                    },
                    pts,
                    dts: pts,
                    is_keyframe: n == 0,
                }
            })
            .collect();
        write_mp4_with_audio(
            path,
            &VideoParams {
                codec: Codec::H264,
                width: 64,
                height: 48,
                fps: 60,
            },
            &ParameterSets {
                sps: vec![vec![0x67, 0x42, 0xc0, 0x0d, 0x96, 0x54, 0x0a, 0x0f, 0xc8]],
                pps: vec![vec![0x68, 0xce, 0x3c, 0x80]],
            },
            &video,
            Some((
                &AudioParams {
                    priming_frames: priming,
                    ..AudioParams::from(AudioConfig::default())
                },
                audio,
            )),
        )
        .unwrap();
    }

    /// The AVFoundation decoder (`tests/support/avf_decode.swift`), compiled
    /// once per test run. `None` where there is no Swift toolchain.
    fn avf_decoder() -> Option<&'static Path> {
        static BIN: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        BIN.get_or_init(|| {
            let dir = scratch("avf");
            std::fs::create_dir_all(&dir).ok()?;
            let source = dir.join("avf_decode.swift");
            let bin = dir.join(format!("avf_decode-{}", std::process::id()));
            std::fs::write(
                &source,
                include_str!("../../tests/support/avf_decode.swift"),
            )
            .ok()?;
            let ok = Command::new("swiftc")
                .arg("-O")
                .arg(&source)
                .arg("-o")
                .arg(&bin)
                .status()
                .ok()?
                .success();
            ok.then_some(bin)
        })
        .as_deref()
    }

    /// Decode the clip's audio like QuickTime/Safari would (honoring the edit
    /// list) and return the left channel on the clip's timeline. `None` if
    /// the decoder can't be built here.
    fn decode_left_channel(mp4: &Path) -> Option<Vec<f32>> {
        let decoder = avf_decoder()?;
        let raw = mp4.with_extension("f32");
        let output = Command::new(decoder).arg(mp4).arg(&raw).output().ok()?;
        assert!(
            output.status.success(),
            "avf_decode failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = std::fs::read(&raw).unwrap();
        std::fs::remove_file(&raw).ok();
        Some(
            bytes
                .chunks_exact(8) // interleaved L,R f32
                .map(|f| f32::from_le_bytes(f[..4].try_into().unwrap()))
                .collect(),
        )
    }

    /// Energy-weighted mean position of `pcm`, in seconds.
    fn centroid(pcm: &[f32]) -> f64 {
        let (mut energy, mut weighted) = (0.0f64, 0.0f64);
        for (n, s) in pcm.iter().enumerate() {
            let e = f64::from(*s) * f64::from(*s);
            energy += e;
            weighted += e * n as f64;
        }
        assert!(energy > 0.0, "decoded audio is silent");
        weighted / energy / f64::from(RATE)
    }

    /// A 20 ms burst of deterministic noise at `at` seconds, silence elsewhere.
    fn burst_at(at: f64) -> impl Fn(u64) -> f32 {
        move |n| {
            let t = n as f64 / f64::from(RATE);
            if (at..at + 0.02).contains(&t) {
                (((n.wrapping_mul(2_654_435_761)) >> 8) % 2000) as f32 / 1000.0 - 1.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn the_os_decoder_recovers_the_tone() {
        let mut enc = encoder();
        let packets = encode_all(&mut enc, &chunks(2.0, Duration::ZERO, tone));
        let path = scratch("tone.mp4");
        mux(&path, &packets, enc.priming_frames(), Duration::ZERO, 2.0);
        let Some(pcm) = decode_left_channel(&path) else {
            eprintln!("no Swift toolchain; skipping decode check");
            return;
        };
        std::fs::remove_file(&path).ok();

        let seconds = pcm.len() as f64 / f64::from(RATE);
        assert!((1.9..2.2).contains(&seconds), "decoded {seconds:.3} s");

        // Middle second: amplitude 0.5 sine → RMS ≈ 0.354, 440 Hz.
        let mid = &pcm[RATE as usize / 2..RATE as usize * 3 / 2];
        let rms = (mid.iter().map(|s| s * s).sum::<f32>() / mid.len() as f32).sqrt();
        assert!((0.30..0.40).contains(&rms), "rms {rms}");
        let crossings = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((430..=450).contains(&crossings), "{crossings} Hz");
    }

    #[test]
    fn decoded_audio_lines_up_with_the_timeline() {
        // A burst at exactly t = 1.000 s. After encode → mux (with its edit
        // list) → OS decode, its energy must sit at ~1.010 s (the burst's
        // centre). Ignoring the encoder delay would move it ~44 ms.
        let mut enc = encoder();
        let packets = encode_all(&mut enc, &chunks(2.0, Duration::ZERO, burst_at(1.0)));
        let path = scratch("burst.mp4");
        mux(&path, &packets, enc.priming_frames(), Duration::ZERO, 2.0);
        let Some(pcm) = decode_left_channel(&path) else {
            eprintln!("no Swift toolchain; skipping alignment check");
            return;
        };
        std::fs::remove_file(&path).ok();

        let c = centroid(&pcm);
        assert!(
            (c - 1.010).abs() < 0.004,
            "burst centroid at {c:.4} s, expected 1.0100 s"
        );
    }

    #[test]
    fn a_clip_cut_from_the_middle_of_the_stream_lines_up_too() {
        // Mimic ClipService: a 4 s stream, a clip whose video starts at 1.5 s,
        // and the audio packets the buffer would hand over (100 ms before the
        // video start up to 100 ms after its end). The burst at 2.0 s must
        // land 0.5 s into the clip.
        let mut enc = encoder();
        let all = encode_all(&mut enc, &chunks(4.0, Duration::ZERO, burst_at(2.0)));
        let start = Duration::from_millis(1500);
        let end = Duration::from_millis(3000);
        let cut: Vec<AudioPacket> = all
            .iter()
            .filter(|p| {
                p.pts + Duration::from_millis(100) >= start
                    && p.pts <= end + Duration::from_millis(100)
            })
            .cloned()
            .collect();
        let path = scratch("cut.mp4");
        mux(&path, &cut, enc.priming_frames(), start, 1.5);
        let Some(pcm) = decode_left_channel(&path) else {
            eprintln!("no Swift toolchain; skipping cut check");
            return;
        };
        std::fs::remove_file(&path).ok();

        let c = centroid(&pcm);
        assert!(
            (c - 0.510).abs() < 0.004,
            "burst centroid at {c:.4} s, expected 0.5100 s"
        );
    }

    #[test]
    fn audio_that_starts_after_the_video_is_delayed_not_shifted() {
        // Audio begins 200 ms after the video does; a burst 0.3 s into the
        // audio must play at 0.5 s of the clip (an empty edit covers the lead).
        let mut enc = encoder();
        let audio_start = Duration::from_millis(200);
        let packets = encode_all(&mut enc, &chunks(2.0, audio_start, burst_at(0.3)));
        let path = scratch("late-audio.mp4");
        mux(&path, &packets, enc.priming_frames(), Duration::ZERO, 2.0);
        let Some(pcm) = decode_left_channel(&path) else {
            eprintln!("no Swift toolchain; skipping late-audio check");
            return;
        };
        std::fs::remove_file(&path).ok();

        let c = centroid(&pcm);
        assert!(
            (c - 0.510).abs() < 0.006,
            "burst centroid at {c:.4} s, expected 0.5100 s"
        );
    }
}
