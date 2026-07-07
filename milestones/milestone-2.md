# Milestone 2 — Replay Buffer & Clip Hotkey

**Goal:** the defining feature — the app runs in the background, keeps the
last N seconds in memory, and a global hotkey saves them as a clip file.

## F-2.1 — Encoded ring buffer
- [ ] Ring buffer stores encoded packets, bounded by configured duration (default 120 s)
- [ ] Eviction is GOP-aligned: buffer always starts at a keyframe
- [ ] Property tests: wraparound, exact-duration extraction, empty buffer, clock jumps
- [ ] Memory stays under budget for 2 min of 1080p60 (measured in test)

## F-2.2 — Clip extraction
- [ ] "Save last N seconds" extracts from buffer and remuxes to .mp4 (no re-encode)
- [ ] Clip saved to configurable clips folder with timestamped filename
- [ ] Extraction under 500 ms for a 30 s clip (measured)
- [ ] Capture keeps running uninterrupted during extraction

## F-2.3 — Global hotkey & background mode
- [ ] Global hotkey (default configurable) triggers clip save from any app/game
- [ ] Tray/menu-bar icon: start/stop buffer, save clip, open clips folder
- [ ] OS notification confirms "Clip saved" with duration
- [ ] Unit tests for hotkey→command dispatch (mocked hotkey events)

## F-2.4 — Windows parity
- [ ] `WindowsGraphicsCaptureSource` implements `FrameSource`
- [ ] `MediaFoundationEncoder` implements `Encoder` (hardware H.264)
- [ ] Ignored integration tests pass on a Windows machine / CI
- [ ] Same clip pipeline (buffer/extract/hotkey) verified on Windows
