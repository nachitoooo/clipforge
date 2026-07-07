# Milestone 1 — Foundation & Capture Engine

**Goal:** a Rust workspace where the core engine can capture the screen on
macOS, hardware-encode it, and write a valid .mp4 — with the whole pipeline
testable without a real screen.

## F-1.1 — Repo bootstrap
- [x] Cargo workspace with `clipforge-core` crate compiles (`cargo build`)
- [x] Tauri 2 app scaffold in `app/` runs (`npm run tauri dev` opens a window)
- [x] CI: GitHub Actions matrix (macos-latest, windows-latest) runs `cargo test`
- [x] `cargo check --target x86_64-pc-windows-msvc` wired into CI from day one

## F-1.2 — Capture & encode abstractions
- [ ] `FrameSource` trait (async frame stream, BGRA + timestamp)
- [ ] `Encoder` trait (frame in → encoded packet out, keyframe flag exposed)
- [ ] `TestPatternSource`: synthetic 60 fps source with frame numbers embedded
- [ ] Unit tests: source→encoder pipeline produces monotonic timestamps

## F-1.3 — macOS capture (ScreenCaptureKit)
- [ ] `ScreenCaptureKitSource` implements `FrameSource` for the main display
- [ ] Screen-recording permission flow handled (clear error when denied)
- [ ] Ignored integration test `macos_capture_smoke`: captures 3 s of real screen
- [ ] Frame drop rate < 1% at 1080p60 on this Mac (measured, logged)

## F-1.4 — macOS hardware encoding (VideoToolbox)
- [ ] `VideoToolboxEncoder` implements `Encoder` (H.264, hardware-only)
- [ ] Encoded stream muxed to a valid .mp4 (plays in QuickTime)
- [ ] End-to-end test: TestPatternSource → encoder → mp4, duration matches
- [ ] Configurable bitrate/fps; sane 1080p60 defaults documented
