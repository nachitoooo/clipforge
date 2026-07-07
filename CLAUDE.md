# ClipForge — Project Instructions

Cross-platform desktop app (Windows + macOS) to record game highlight clips:
a continuous replay buffer captures gameplay in the background and a global
hotkey saves the last N seconds as a clip.

## Branch Conventions

- `main` — stable, production-ready code. Never commit directly to main.
- `feat/<milestone>/<feature-name>` — new features (e.g., `feat/1/capture-macos`, `feat/2/replay-buffer`)
- `fix/<description>` — bug fixes (e.g., `fix/hotkey-double-fire`)
- All work is done in feature branches and merged to `main` via PR.
- Commit messages: concise, imperative mood (e.g., "Add ScreenCaptureKit frame source", "Fix ring buffer wraparound").

## Testing Requirements

- **Unit tests are mandatory** for every feature (`cargo test`). No feature is
  complete without them.
- The capture/encode pipeline must be testable without a real screen: all
  sources implement the `FrameSource` trait, and tests use the synthetic
  `TestPatternSource` (generated frames with embedded frame numbers) to verify
  buffering, timing, and encoding end-to-end.
- Ring buffer, clip extraction, and trimming logic must have property-style
  tests around boundaries (wraparound, exact-duration clips, empty buffer).
- UI E2E: `tauri-driver` + WebdriverIO once the UI stabilizes (Milestone 3);
  until then, keep UI logic in testable TypeScript modules.
- Platform-specific capture code (`core/src/capture/{windows,macos}`) is
  exercised by ignored integration tests (`cargo test -- --ignored`) that run
  only on the matching OS in CI.

## Milestone Tracking

- Features are tracked in `milestones/*.md`. Each feature has a unique ID (e.g., `F-1.3`).
- **When a feature is completed**, update its acceptance criteria checkboxes from `[ ]` to `[x]` in the corresponding milestone file.
- **When ALL features in a milestone are completed**, add a `> **Status:** COMPLETED (YYYY-MM-DD)` line at the top of the milestone file, below the metadata block.
- Milestones are the single source of truth for project progress.

## Improvement Backlog

- **File:** `milestones/backlog-improvements.md`
- When the user mentions an idea, wish, or improvement ("estaría bueno…", "we should add…"), **add it immediately** to the backlog — do not wait for an explicit instruction.
- Each entry: short title, related milestone/feature (or "new"), why it matters, details, date added.
- Periodically, backlog items get promoted into milestone acceptance criteria.

## Tech Stack

- **Core:** Rust (stable), Cargo workspace. Crate `clipforge-core` holds the
  capture → encode → ring-buffer engine.
- **Capture (per OS, behind the `FrameSource` trait):**
  - Windows: Windows.Graphics.Capture via the `windows` crate
  - macOS: ScreenCaptureKit via the `screencapturekit` crate
- **Encoding (per OS, behind the `Encoder` trait, always hardware):**
  - Windows: Media Foundation H.264/HEVC (NVENC/AMF/QSV resolved by MF)
  - macOS: VideoToolbox H.264/HEVC
- **Remux/trim:** `ffmpeg` sidecar binary (never for encoding — only stream copy).
- **UI:** Tauri 2 + TypeScript (clips gallery, trim editor, settings, hotkeys).
- **Global hotkeys:** `global-hotkey` crate.
- **CI:** GitHub Actions matrix (`windows-latest`, `macos-latest`).

## Architecture

```
core/                  # clipforge-core crate
  src/
    capture/           # FrameSource trait + test_pattern.rs, macos/, windows/
    encode/            # Encoder trait + videotoolbox.rs, mediafoundation.rs
    buffer/            # encoded-frame ring buffer (GOP-aligned eviction)
    clip/              # extract last-N-seconds, remux to mp4, trim
  tests/               # integration tests with TestPatternSource
app/                   # Tauri 2 app (src-tauri/ + src/ TypeScript UI)
milestones/            # feature tracking (see Milestone Tracking)
docs/                  # design notes, per-OS API research
scripts/               # dev helpers
```

Key design rule: **the ring buffer stores encoded frames** (not raw), so RAM
stays bounded (~100MB for 2 min of 1080p60) and saving a clip is a remux, not
a re-encode. Buffer eviction must respect GOP boundaries so every saved clip
starts on a keyframe.

## Development Environment

- This machine is a Mac: develop and test the macOS capture path and all
  platform-independent code (buffer, clip, UI) here.
- The Windows capture/encode path builds and runs via GitHub Actions and a
  Windows machine/VM; keep it compiling with `cargo check --target
  x86_64-pc-windows-msvc` (cross-check only, no link).

## Build & Run

```bash
# Core engine tests
cargo test -p clipforge-core

# Run the desktop app (from app/)
cd app && npm install && npm run tauri dev

# macOS-only capture integration test (records 3 s of the real screen)
cargo test -p clipforge-core -- --ignored macos_capture_smoke
```

- macOS: screen recording permission must be granted to the terminal/IDE the
  first time the capture test runs (System Settings → Privacy → Screen Recording).
