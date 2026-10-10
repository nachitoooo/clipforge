# Milestone 2 — Replay Buffer & Clip Hotkey

**Goal:** the defining feature — the app runs in the background, keeps the
last N seconds in memory, and a global hotkey saves them as a clip file.

## F-2.1 — Encoded ring buffer
- [x] Ring buffer stores encoded packets, bounded by configured duration (default 120 s)
- [x] Eviction is GOP-aligned: buffer always starts at a keyframe
- [x] Property tests: wraparound, exact-duration extraction, empty buffer, clock jumps
- [x] Memory stays under budget for 2 min of 1080p60 (measured in test)
  — measured 2026-07-07: peak 182 MiB for the 120 s window (budget 256 MiB)

## F-2.2 — Clip extraction
- [x] "Save last N seconds" extracts from buffer and remuxes to .mp4 (no re-encode)
- [x] Clip saved to configurable clips folder with timestamped filename
- [x] Extraction under 500 ms for a 30 s clip (measured)
  — measured 2026-07-07: 16.9 ms for a 30 s / 41 MiB clip (1920 packets)
- [x] Capture keeps running uninterrupted during extraction
  — measured 2026-07-07: worst push wait 8 ms across 71 concurrent saves

## F-2.3 — Global hotkey & background mode
- [x] Global hotkey (default configurable) triggers clip save from any app/game
  — default Cmd/Ctrl+Shift+S; runtime re-mapping arrives with the Settings UI (M3)
- [x] Tray/menu-bar icon: start/stop buffer, save clip, open clips folder
- [x] OS notification confirms "Clip saved" with duration
- [x] Unit tests for hotkey→command dispatch (mocked hotkey events)
  — verified 2026-07-07: app runs in background (window closed ≠ exit), buffer
  auto-starts and fills during screen activity; physical hotkey press pending
  a quick manual check (automation can't inject keystrokes without
  Accessibility permission)

## F-2.4 — Windows parity
- [ ] `WindowsGraphicsCaptureSource` implements `FrameSource`
- [ ] `MediaFoundationEncoder` implements `Encoder` (hardware H.264)
- [ ] Ignored integration tests pass on a Windows machine / CI
- [ ] Same clip pipeline (buffer/extract/hotkey) verified on Windows

## F-2.5 — Audio tracks (system audio)
> Added 2026-10-09 (promoted from the backlog: clips were silent). Microphone
> as a separate track is **not** part of this feature — see the backlog.
- [x] Audio pipeline types and traits (`AudioChunk`, `AudioPacket`, `AudioEncoder`, `AudioSource`) with a deterministic mock encoder and a synthetic tone source for tests
- [x] `ContinuityGuard`: capture gaps are filled with silence, overlaps trimmed, huge jumps restart the timeline (the mp4 muxer stores durations, not gaps, so a hole would desync everything after it)
- [x] AAC packets are buffered next to the video, bounded in time, with discontinuity handling; extraction returns the audio around the video span
- [x] Clips carry an AAC track aligned to the video through an `elst` edit list (encoder delay, decoder warm-up for clips cut mid-stream, audio that starts after the video)
  — verified 2026-10-09 by decoding with AVFoundation: a burst at a known instant lands within 4 ms, for full-stream, mid-stream and late-start audio
- [x] macOS: system audio via ScreenCaptureKit (excluding ClipForge's own sounds) → AudioToolbox AAC-LC 128 kb/s, same epoch as the video
  — measured 2026-10-09: 5 s of real capture → 239 AAC packets, saved clip has a 48 kHz stereo AAC track; video drop rate still 0.000%
- [x] Audio never takes the video down: no AAC encoder → silent clips; a `WithAudio` source whose audio fails to start keeps recording video
- [ ] Windows: WASAPI loopback (`audio/wasapi.rs`) + Media Foundation AAC encoder (`audio/aac_mediafoundation.rs`)
  — written, cross-checked and linted for `x86_64-pc-windows-msvc`, **never run**. Needs F-2.4 (Windows video) wired in and `cargo test -p clipforge-core --test windows_audio -- --ignored` on a Windows machine; the MF encoder delay (`MF_AAC_PRIMING_FRAMES`) is an assumption to calibrate
- [ ] Manual check on macOS with real game audio: play something, press the hotkey, listen to the clip (the automated real-capture test ran on a silent system, so audio *content* is covered by unit tests and a byte-count check, not by ear)
