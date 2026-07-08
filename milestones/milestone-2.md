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
