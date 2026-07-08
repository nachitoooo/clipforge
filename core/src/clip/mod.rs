//! Clip output: mux encoded packets to .mp4 ([`mp4mux`]), and — in
//! Milestone 2 — extract last-N-seconds from the ring buffer and trim.
//!
//! Muxing is done in-process with the pure-Rust `mp4` crate (packets are
//! already AVCC, so this is a container write, not a re-encode). The ffmpeg
//! sidecar remains reserved for stream-copy trimming of existing files.

pub mod mp4mux;
