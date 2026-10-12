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
  until then, keep UI logic in testable TypeScript modules (`app/src/lib/*`,
  plus simulated-DOM view tests with `happy-dom`).
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
- **Encoding (per OS, behind the `Encoder` trait):**
  - Windows: Media Foundation H.264. Today Microsoft's own software encoder MFT
    (`CMSH264EncoderMFT`: synchronous, no GPU needed, ~150 fps at 1080p); the
    asynchronous hardware MFTs (NVENC/AMF/QSV) are not driven yet.
  - macOS: VideoToolbox H.264/HEVC (hardware)
- **Audio (system/game sound, per OS):**
  - macOS: ScreenCaptureKit audio (same stream as the video) → AudioToolbox AAC-LC
  - Windows: WASAPI loopback → Media Foundation AAC encoder
  - Packets are AAC and live in the ring buffer next to the video; the muxer
    aligns them with an `elst` edit list (`clip/edit_list.rs`).
- **Remux/trim/edit:** in-process Rust (`core/src/clip/trim.rs` + our own
  muxer) — no ffmpeg. Cuts are stream copies: the copy starts at the keyframe
  before the requested start and a video edit list hides the extra frames, so
  the start is frame accurate. Never re-encode; audio waveforms are decoded in
  Rust (`symphonia`).
- **UI:** Tauri 2 + TypeScript (clips gallery, library folders, trim editor, settings, hotkeys).
- **Global hotkeys:** `global-hotkey` crate.
- **CI:** GitHub Actions matrix (`windows-latest`, `macos-latest`).

## Architecture

```
core/                  # clipforge-core crate
  src/
    activity/          # foreground-application lookup + log/tracker (the "game" of a clip)
    clipboard/         # put a clip file on the system clipboard (NSPasteboard / CF_HDROP)
    screenshot/        # a PNG of the screen (SCScreenshotManager), saved next to the clips
    capture/           # FrameSource/AudioSource traits + test_pattern.rs, macos/, windows/
    audio/             # AudioEncoder trait, ContinuityGuard, AAC encoders, WASAPI
    encode/            # Encoder trait + videotoolbox.rs, mediafoundation.rs
    buffer/            # encoded-frame ring buffer (GOP-aligned eviction) + AAC lane
    clip/              # extract last-N-seconds, remux to mp4 (+audio tracks), trim/probe/waveform (trim.rs)
  tests/               # integration tests with TestPatternSource
app/                   # Tauri 2 app (src-tauri/ + src/ TypeScript UI)
milestones/            # feature tracking (see Milestone Tracking)
docs/                  # design notes, per-OS API research
scripts/               # dev helpers
```

Audio rule: the mp4 muxer stores sample *durations*, not gaps, so audio must
reach it gap-free — `ContinuityGuard` fills holes with silence before encoding.
Audio is best effort: a failing audio path must never stop the video.

Audio lanes: the replay buffer holds AAC in lanes — `Main` (game audio, or game
+ voice mixed) and `Mic` (voice on its own track). The microphone attaches to a
*running* capture (`CaptureEngine::attach_microphone`) either mixed in real time
by `audio/mixer.rs` (between the system capture and the encoder) or as its own
lane; it shares the video's `SharedEpoch` clock. Never mix at save time — that
would mean decoding and re-encoding.

Memory (`app/src-tauri/src/memory.rs`): the replay buffer's RAM is its length times
its bitrate (about 180 MB by default); *low RAM mode* (`settings.performance.lowRam`)
halves both. Fixed when the capture starts, so changing it restarts the capture
(`Settings::needs_capture_restart`) and the UI warns first.

Settings (`app/src-tauri/src/settings.rs`): every field has a serde default, one
validation gate (`Settings::validated`), and an apply path that is either live
(atomics) or restarts only the affected part. Anything that restarts the video
capture empties the buffer, so it needs a warning in the UI. Global
shortcuts (save clip, mute) are settings too and are (un)registered only through
`shortcuts.rs`. See `docs/settings-roadmap.md` for the checklist and the planned
settings.

The clips folder also holds **screenshots** (`.png`, `ClipInfo.kind`): everything
that works on a name uses `library::media_path` (clip or picture); only the editor
uses `library::clip_path` (video).

Library folders (`app/src-tauri/src/folders.rs`, `docs/library.md`): a folder is
a *label* kept in `library.json`, not a directory — clips stay in the flat clips
folder and a clip can be in several folders, so the UI still names a clip by its
file name. Anything that renames, deletes or copies a clip must keep the index in
step (see the command wrappers in `lib.rs`), and reads never write or prune it.

Smart library (`core/src/activity/`, `app/src-tauri/src/clipmeta.rs`,
`docs/library.md`): while the buffer runs a tracker notes which application is in
front (name only, local, switchable in Settings); a saved clip is tagged with the
one that held the screen longest and the game lives in `clipmeta.json`, a second
index with the same rules as the folder one (both on `persist::JsonStore`). The
hooks in `lib.rs` (`clip_renamed`, `clips_deleted`, `clip_copied`) must keep
**both** in step. With `library.autoFolders` on, a saved clip is also filed in the
folder named after its game (`FolderStore::file_under_game`). What is planned next is in `docs/medal-roadmap.md`.

Key design rule: **the ring buffer stores encoded frames** (not raw), so RAM
stays bounded (~100MB for 2 min of 1080p60) and saving a clip is a remux, not
a re-encode. Buffer eviction must respect GOP boundaries so every saved clip
starts on a keyframe.

## Development Environment

- This machine is a Mac: develop and test the macOS capture path and all
  platform-independent code (buffer, clip, UI) here.
- The Windows capture/encode path is verified on a real Windows runner:
  the `windows-video` workflow (screen capture, H.264, sound with a virtual audio
  cable, a clip decoded back by Windows) and `windows-app` (the built app started,
  a hotkey pressed, the clip checked). Ignored tests in `core/tests/windows_*.rs`.
  It builds and runs via GitHub Actions and a Windows machine/VM; keep it compiling with `cargo check --target
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
