# ClipForge

[![CI](https://github.com/nachitoooo/clipforge/actions/workflows/ci.yml/badge.svg)](https://github.com/nachitoooo/clipforge/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Open-source game highlight clips for Windows and macOS. A replay buffer records
your screen and sound in the background; press a hotkey and the last seconds
(or minutes) become an `.mp4` you can trim, organize and share. Think Medal or
ShadowPlay, without an account and without leaving your machine.

**Stack:** Rust core (per-OS capture and hardware encoding) · Tauri 2 + TypeScript UI.

## Download

Ready-to-run files are on the [Releases page](https://github.com/nachitoooo/clipforge/releases):
a Windows installer, a portable Windows `.exe` (double-click, nothing to install)
and a macOS `.dmg` (Apple silicon). Both systems record. The files are not signed,
so Windows SmartScreen and macOS Gatekeeper warn the first time; the release notes
say how to get past that.

## What it does today

- **Replay buffer** that keeps *encoded* frames (about 100 MB for two minutes of
  1080p60), so saving a clip is a remux, never a re-encode.
- **Global hotkeys**, configurable: one for the main clip length, up to four more
  with a length of their own (say F9 for the last 15 s, F10 for the last minute),
  plus mute for the microphone.
- **Game sound and microphone**, mixed or on separate tracks.
- **Gallery and editor**: previews, rename, delete, and a trim editor that cuts
  by stream copy with a frame-accurate start (no quality loss), with a waveform
  per audio track.
- **Library**: your own folders (a clip can live in several), search, filters
  by game, date and folder, and a game detected from the application that was in
  front while the clip was recorded (name only, kept on your computer, can be
  switched off).
- **Screenshots** of the screen (a hotkey you choose, the tray or a button), kept in
  the same library as the clips. macOS for now.
- **Copy** a clip or a screenshot to paste it in Discord, Mail and so on.
- **Low RAM mode** (Settings → Rendimiento): one minute of buffer at half the
  bitrate, about a quarter of the memory.
- **Light or dark** interface (or the system's) and a main color of your choice.
- Lives in the tray; nothing is ever uploaded anywhere.

## Status

| | macOS | Windows |
|---|---|---|
| Capture + encoding | ScreenCaptureKit + VideoToolbox (hardware) | Windows Graphics Capture + Media Foundation H.264 (software, no GPU needed) |
| Audio (game + microphone) | Working | Working (WASAPI loopback, cpal) |
| UI, library, editor, settings | Working | Working |
| Screenshots, copy to clipboard | Working | Working |

On Windows, every stage runs on a real Windows machine in CI — screen capture, the
encoder, the sound, a clip decoded back by Windows, and the installed app started,
a hotkey pressed and the clip checked (see the `windows-video` and `windows-app`
workflows). What CI can't give is a gaming PC: the encoder is Microsoft's software
one (about 150 fps at 1080p on a 4-core runner) and the hardware encoders (NVENC,
AMF, Quick Sync) are not driven yet.

Progress is tracked feature by feature in [`milestones/`](milestones/); what is
planned next (compared with what Medal offers) is in
[`docs/medal-roadmap.md`](docs/medal-roadmap.md) (in Spanish).

## Build and run

You need a recent stable [Rust](https://rustup.rs), Node.js, and the
[Tauri 2 prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```bash
# Run the desktop app
cd app && npm install && npm run tauri dev

# Tests
cargo test -p clipforge-core          # capture/encode/buffer/clip engine
cargo test -p clipforge-app --lib     # app logic (settings, shortcuts, library)
cd app && npm test                    # UI logic and views (vitest + happy-dom)
```

On macOS the first run asks for **Screen Recording** permission (System Settings →
Privacy & Security), and **Microphone** only if you switch the microphone on.
The engine tests don't need a screen: they run against a synthetic test-pattern
source. Tests that touch the real screen or devices are `#[ignore]`d and run
with `cargo test -- --ignored`.

## Layout

```
core/       clipforge-core: capture → encode → ring buffer → clip (no ffmpeg)
app/        Tauri 2 app: src-tauri/ (Rust) and src/ (TypeScript UI)
docs/       design notes (library, clip editor, settings, Windows setup)
milestones/ feature tracking with acceptance criteria
```

## Contributing

Issues and pull requests are welcome. [`CLAUDE.md`](CLAUDE.md) is the operating
manual for this repo (conventions, testing rules, architecture) and applies to
people as much as to tools:

- work on a branch (`feat/<milestone>/<name>` or `fix/<name>`) and open a PR;
- every feature comes with unit tests;
- clips are cut by stream copy, never re-encoded;
- keep `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt` clean.

Driving the Windows hardware encoders (NVENC, AMF, Quick Sync) is a good place to
help: they are asynchronous Media Foundation transforms, and CI has no GPU to test
them on. See [`docs/windows-setup.md`](docs/windows-setup.md).

## License

[MIT](LICENSE). The interface uses the [Inter](https://rsms.me/inter/) typeface
(SIL Open Font License 1.1).
