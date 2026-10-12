## ClipForge (preview)

Game highlight clips: a replay buffer records your screen and sound in the
background, and a hotkey saves the last seconds (or minutes) as an `.mp4` you can
trim, organize and share. Everything stays on your computer. The app is in Spanish.

### Downloads

| File | For | How |
|---|---|---|
| `ClipForge_*_x64-setup.exe` | Windows 10/11 | Double-click, it installs and adds a Start menu entry |
| `ClipForge-portable-x64.exe` | Windows 10/11 | Double-click, nothing to install (needs the WebView2 runtime, already part of Windows 11 and most Windows 10) |
| `ClipForge_*_aarch64.dmg` | Apple silicon Macs, macOS 13 or newer | Open it and drag ClipForge to Applications |

### ⚠️ Read this first

- **Recording works on macOS only, for now.** The Windows capture and encoder are
  not written yet ([milestone F-2.4](https://github.com/nachitoooo/clipforge/blob/main/milestones/milestone-2.md)).
  The Windows files open the interface (library, folders, settings, the clip editor
  on existing `.mp4` files) but the replay buffer cannot start and it will say so.
  They are here so the interface can be tried and the Windows work can be tested.
- **The files are not signed**, so the system will warn you the first time:
  - Windows: *"Windows protected your PC"* → **More info** → **Run anyway**.
  - macOS: right-click ClipForge → **Open** (or, if it says the app is damaged, run
    `xattr -dr com.apple.quarantine /Applications/ClipForge.app`).
- macOS asks for **Screen Recording** the first time (and **Microphone** only if you
  turn the microphone on). Screenshots need macOS 14 or newer; Intel Macs have no
  download yet.

### What it does today (macOS)

Replay buffer with hardware H.264 · game sound and microphone · global hotkeys with
their own clip lengths · lossless trim editor · library with folders, search, filters
and game detection · screenshots · copy a clip to paste it in Discord or Mail.
