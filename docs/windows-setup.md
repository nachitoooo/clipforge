# Setting up a Windows dev machine

Prerequisites to clone and work on ClipForge from Windows (F-2.4 onward).

## Install

1. **Visual Studio Build Tools 2022** (or full VS) with the
   **"Desktop development with C++"** workload — provides MSVC and the
   Windows SDK that both Rust and Tauri need.
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools
   ```
2. **Rust (stable)** via rustup:
   ```powershell
   winget install Rustlang.Rustup
   rustup default stable
   ```
3. **Node.js LTS**:
   ```powershell
   winget install OpenJS.NodeJS.LTS
   ```
4. **WebView2 Runtime** — preinstalled on Windows 11; on Windows 10:
   ```powershell
   winget install Microsoft.EdgeWebView2Runtime
   ```
5. (Recommended) **GitHub CLI**: `winget install GitHub.cli`, then `gh auth login`.

## Clone & verify

```powershell
git clone https://github.com/nachitoooo/clipforge.git
cd clipforge

# Core engine tests (buffer, clip extraction, muxer — all platform-independent)
cargo test -p clipforge-core

# App unit tests (hotkey/tray dispatch)
cargo test -p clipforge-app --lib

# Run the desktop app (window + tray; records the primary monitor)
cd app
npm install
npm run tauri dev
```

The buffer starts by itself and records the primary monitor (scaled to fit
1920×1080) with the sound the computer plays. If it can't start, the window says why.

To try the Windows-only integration tests by hand (they need a desktop session):

```powershell
cargo test --release -p clipforge-core --test windows_video -- --ignored --nocapture
cargo test --release -p clipforge-core --test windows_audio -- --ignored --nocapture
```

The sound tests need an output device. On a machine without one (a CI runner, a
bare VM) install a virtual cable — the `windows-video` workflow shows how
(`choco install vb-cable`, trust its publisher, run its setup).

## Notes

- `.cargo/config.toml` in the repo only adds macOS link flags
  (`[target.aarch64-apple-darwin]` / `x86_64-apple-darwin`); it is inert on
  Windows.
- Windows capture/encode work happens in `core/src/capture/windows/` and
  `core/src/encode/mediafoundation.rs` behind the same `FrameSource`/`Encoder`
  traits the macOS path implements (see `capture/macos/`, `encode/videotoolbox.rs`
  as reference implementations).
- Platform-specific integration tests are `#[ignore]`d and run with
  `cargo test -p clipforge-core -- --ignored` on the matching OS. For Windows the
  GitHub workflows `windows-video` and `windows-app` run them on a runner and keep
  the clips and screenshots they produce as artifacts.
- Audio (F-2.5) on Windows — WASAPI loopback (`core/src/audio/wasapi.rs`), the
  Media Foundation AAC encoder (`core/src/audio/aac_mediafoundation.rs`) and the
  microphone (`core/src/audio/mic.rs`, `cpal` on WASAPI) — is verified in CI with a
  virtual audio cable. On a real machine the microphone also needs Windows'
  "Let desktop apps access your microphone" privacy setting turned on.
