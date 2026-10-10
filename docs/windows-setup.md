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

# Run the desktop app (window + tray; capture arrives with F-2.4)
cd app
npm install
npm run tauri dev
```

Until F-2.4 lands, starting the buffer on Windows reports
"la captura en este sistema operativo llega con F-2.4" — expected.

## Notes

- `.cargo/config.toml` in the repo only adds macOS link flags
  (`[target.aarch64-apple-darwin]` / `x86_64-apple-darwin`); it is inert on
  Windows.
- Windows capture/encode work happens in `core/src/capture/windows/` and
  `core/src/encode/mediafoundation.rs` behind the same `FrameSource`/`Encoder`
  traits the macOS path implements (see `capture/macos/`, `encode/videotoolbox.rs`
  as reference implementations).
- Platform-specific integration tests are `#[ignore]`d and run with
  `cargo test -p clipforge-core -- --ignored` on the matching OS.
- Audio (F-2.5) has Windows implementations that have **not been run yet**:
  WASAPI loopback capture (`core/src/audio/wasapi.rs`) and the Media Foundation
  AAC encoder (`core/src/audio/aac_mediafoundation.rs`). On a Windows machine,
  run `cargo test -p clipforge-core --test windows_audio -- --ignored` and
  report anything that fails: those tests were written without access to
  Windows. They only become audible in the app once F-2.4 (Windows video
  capture) is wired in.
