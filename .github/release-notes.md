## ClipForge

Game highlight clips: a replay buffer records your screen and sound in the
background, and a hotkey saves the last seconds (or minutes) as an `.mp4` you can
trim, organize and share. Everything stays on your computer. The app is in Spanish.

### New in 0.2.2

- **Much less RAM.** The replay buffer now holds only what your longest clip needs
  (34 seconds with the usual 30 s clip, about 50 MB) instead of a fixed two
  minutes (180 MB). Measured on a busy 1080p60 screen: about 160 MB less. The clips
  you save are the same, and it adjusts by itself when you change a clip length.
- **Copy each clip when it is saved** (Settings → Biblioteca, off by default): the
  new clip is already on the clipboard, ready to paste in Discord or Mail.
- The clip thumbnails load only near the screen, so a big library no longer keeps
  every one of them in memory.

### Downloads

| File | For | How |
|---|---|---|
| `ClipForge_*_x64-setup.exe` | Windows 10 (1803+) / 11 | Double-click, it installs and adds a Start menu entry |
| `ClipForge-portable-x64.exe` | Windows 10 (1803+) / 11 | Double-click, nothing to install (needs the WebView2 runtime, already part of Windows 11 and most Windows 10) |
| `ClipForge_*_aarch64.dmg` | Apple silicon Macs, macOS 13 or newer | Open it and drag ClipForge to Applications |

### How to use it

Open ClipForge: it starts recording the last two minutes in the background (the
tray icon shows it). When something good happens press **Ctrl+Shift+S**
(**⇧⌘S** on a Mac) and the clip appears in *Recientes*. Settings lets you change
the shortcut, the clip length, the microphone and more.

### Good to know

- **Not signed**, so the system warns you the first time:
  - Windows: *"Windows protected your PC"* → **More info** → **Run anyway**.
  - macOS: right-click ClipForge → **Open** (or, if it says the app is damaged, run
    `xattr -dr com.apple.quarantine /Applications/ClipForge.app`).
- It records the **main monitor**, shrunk to fit 1920×1080, and the sound the
  computer plays. The microphone is off until you turn it on.
- macOS asks for **Screen Recording** the first time (and **Microphone** only if
  you turn the microphone on). Screenshots on macOS need macOS 14 or newer.
- **On Windows the video is encoded by Microsoft's own H.264 encoder** (software,
  it needs no GPU and keeps up at 1080p60 on an ordinary processor). The GPU
  encoders (NVENC, AMF, Quick Sync) are not used yet, so a very busy game on a
  weak processor may cost a few frames. Windows has been tested end to end on a
  Windows machine in CI, not yet on a gaming PC: please report what you find.
- On Intel Macs there is no download yet.

### What it does

Replay buffer · game sound and microphone · global hotkeys with their own clip
lengths · lossless trim editor · library with folders, search, filters and game
detection · screenshots · copy a clip to paste it in Discord or Mail (or by itself
on saving) · light and dark themes · low RAM mode.
