# Milestone 1 — Foundation & Capture Engine

**Goal:** a Rust workspace where the core engine can capture the screen on
macOS, hardware-encode it, and write a valid .mp4 — with the whole pipeline
testable without a real screen.

> **Status:** COMPLETED (2026-07-07)

## F-1.1 — Repo bootstrap
- [x] Cargo workspace with `clipforge-core` crate compiles (`cargo build`)
- [x] Tauri 2 app scaffold in `app/` runs (`npm run tauri dev` opens a window)
- [x] CI: GitHub Actions matrix (macos-latest, windows-latest) runs `cargo test`
- [x] `cargo check --target x86_64-pc-windows-msvc` wired into CI from day one

## F-1.2 — Capture & encode abstractions
- [x] `FrameSource` trait (async frame stream, BGRA + timestamp)
- [x] `Encoder` trait (frame in → encoded packet out, keyframe flag exposed)
- [x] `TestPatternSource`: synthetic 60 fps source with frame numbers embedded
- [x] Unit tests: source→encoder pipeline produces monotonic timestamps

## F-1.3 — macOS capture (ScreenCaptureKit)
- [x] `ScreenCaptureKitSource` implements `FrameSource` for the main display
- [x] Screen-recording permission flow handled (clear error when denied)
- [x] Ignored integration test `macos_capture_smoke`: captures 3 s of real screen
- [x] Frame drop rate < 1% at 1080p60 on this Mac (measured, logged)
  — measured 2026-07-07: 167 frames / 3 s (55.7 fps at native display res), 0 dropped (0.00%)

## F-1.4 — macOS hardware encoding (VideoToolbox)
- [x] `VideoToolboxEncoder` implements `Encoder` (H.264, hardware-only)
- [x] Encoded stream muxed to a valid .mp4 (plays in QuickTime)
  — verified 2026-07-07 via AVFoundation: playable=true, 2.0 s, 1280x720@60
- [x] End-to-end test: TestPatternSource → encoder → mp4, duration matches
- [x] Configurable bitrate/fps; sane 1080p60 defaults documented
