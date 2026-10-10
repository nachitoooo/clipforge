# Settings roadmap

What the Configuración screen has today, how to add a setting without
surprises, and the settings planned next (in the order we would build them).

## Today (F-3.3)

| Setting | Where it lives | Applied |
|---|---|---|
| Microphone on/off | `settings.microphone.enabled` | live: only the microphone stream starts/stops |
| Microphone device | `settings.microphone.deviceId` (`null` = system default) | live: restarts only the microphone stream |
| Microphone mode (mixed / separate track) | `settings.microphone.mode` | live: restarts only the microphone stream |
| Microphone volume 0–200 % | `settings.microphone.gainPercent` | instantly (an atomic shared with the audio thread) |
| Mute + global shortcut | `settings.microphone.muted`, `muteShortcut` | instantly |
| Level meter | not stored; streamed only while the screen is open | — |

## How to add a setting

1. **Model** — add the field to `app/src-tauri/src/settings.rs` with a default.
   Every struct is `#[serde(default)]`, so old settings files keep loading.
2. **Validate** — clamp/normalize it in `Settings::validated`: it is the single
   gate for everything coming from the UI.
3. **Apply** — decide *live* or *restart*:
   * live (atomics, like the microphone gain): set it in
     `Recorder::apply_live_controls`;
   * needs a restart of something: express "what must change" as a small pure
     function like `mic::plan`, unit tested, and restart **only** that part.
   * if it restarts the video capture, the 2-minute buffer is lost (the
     timeline restarts): the UI must warn before applying.
4. **Expose** — extend `Settings` in `app/src/lib/settings.ts` and add a section
   or row in `app/src/ui/settings.ts`; keep logic in `app/src/lib/*` with tests.
5. **Test** — Rust unit tests for validation/planning, vitest for the UI logic
   (`ui/settings.test.ts` drives the real view in a simulated DOM with a fake
   backend), and an ignored real-hardware test if it touches a device.
6. **Document** — tick the item in `milestones/milestone-3.md`.

Rules we keep: the microphone (and any new input) is opt-in; a failing audio
path never stops the video; secrets and paths never go to the webview unless
the setting is about them.

## Planned settings

Effort: S = hours, M = about a day, L = several days. A wave is a suggested
order, not a promise.

### Wave 1 — expected by most people soon (F-3.5)

| Setting | Why | Effort | Notes |
|---|---|---|---|
| Buffer length (60 / 120 / 180 s) | History vs. RAM | S | `BufferConfig`; restarting the buffer loses what's recorded |
| Default clip length (15 / 30 / 60 / 120 s) | Fixed at 30 s today | S | `DEFAULT_CLIP_SECONDS` in `recorder.rs` |
| Clips folder | Fixed at `~/Movies/ClipForge` | S | The `asset` protocol scope in `tauri.conf.json` must follow it |
| Save-clip shortcut | Fixed at ⌘/Ctrl+Shift+S | M | Reuse the key recorder of the mute shortcut |
| Launch at login | Buffer already running when you play | S | Autostart plugin |
| Video quality (Low / Medium / High / Custom) | Resolution, fps, bitrate | M | `EncoderConfig`; restarts the video capture (warn: buffer is lost) |
| Post-roll (keep recording 0 / 5 / 10 / 15 s after the shortcut) | The clip includes how the play ended, not only what led to it | M | Save with a delay: wait N s, then extract. Today the clip ends at the key press |
| Pause the buffer with a shortcut | Privacy: type a password without it being recorded | S | Stop/start the capture (also empties the buffer); the tray already has the entry |
| Tray icon with state (recording / paused / error) | See if the buffer runs without opening the app | S | The tray exists |
| Push-to-talk | Microphone only while a key is held | M | `tauri-plugin-global-shortcut` reports `Released`; drives `MicControls.muted` |
| Show / hide the cursor in clips | Shooters without it, tutorials with it | S | ScreenCaptureKit `shows_cursor`; restarts the video capture (warn) |

### Wave 2 — comfort (F-3.6)

| Setting | Why | Effort | Notes |
|---|---|---|---|
| Game audio on/off and volume | Same controls the microphone has | S | Gain atomic on the main lane |
| Storage limit / delete old clips | Avoid filling the disk | M | Confirm before deleting; a trash, not `remove_file` |
| Notifications and sound on save | Quiet during a game | S | `dispatch.rs` already centralizes `notify` |
| Close button behavior (tray / quit) | Today it goes to the tray | S | `RunEvent::ExitRequested` |
| File name template | `{date}-{game}` | S | `ClipService::unique_clip_path` |

### Wave 3 — capture control (F-3.7)

| Setting | Why | Effort | Notes |
|---|---|---|---|
| Choose the monitor | Multi-display machines | M | `ScreenCaptureKitSource` takes the first display |
| Capture a single window / game | Privacy | L | SCK content filters / Windows Graphics Capture |
| Per-application audio | Game without Spotify or Discord | L | SCK app filters / Windows process loopback |
| Start the buffer only for known games | Zero cost outside games | L | Process detection + editable list |
| Mic noise suppression and gate | Keyboard, fan; also tames echo | L | RNNoise or similar |
| Language (es / en) | Spanish only today | M | Extract UI strings |
| Theme (dark / light / auto) | Preference | S | CSS variables are centralized |

### Wave 4 — advanced (F-3.8)

| Setting | Why | Effort | Notes |
|---|---|---|---|
| HEVC codec | Smaller files at equal quality | L | The muxer needs an `hvcC` box; the `mp4` crate doesn't write one |
| Pick the hardware encoder (NVENC / AMF / QSV) | Diagnostics on Windows | M | Media Foundation |
| Automatic highlight detection | Clips without pressing the hotkey | L | Backlog item |
| Share (clipboard, link) | One-step sharing | L | Needs a service |
| Diagnostics: stats, logs folder, reset, export/import settings | Support | M | `EngineStats` already exposes counters |

## Ideas pool (not scheduled)

Ideas that are in no wave yet (added 2026-10-09). Promote one by moving it into
a wave and a `milestone-3.md` checkbox. Anything that restarts the video
capture needs the "buffer will be emptied" warning.

### Capture and clip

| Idea | Why | Effort | Notes |
|---|---|---|---|
| Exclude apps from the capture (chat, password manager, bank) | Privacy: they never show up in a clip | M | SCK `excluding_applications`; not the same as "capture a single window" |
| Pause on screen lock, sleep or battery | Save battery, don't record what isn't a game | M | OS events; express the decision as a pure, tested function like `mic::plan` |
| Mark a moment (extra shortcut leaving a marker) | "That was good": cut from there later | M | Markers in the clip or in the gallery |
| Per-game profile (quality, clip length, microphone mode) | Each game with its own settings | L | Needs game detection (wave 3) |
| Settings profiles (Competitive / Saver / Streaming) | Switch a group of settings at once | M | A profile is a partial `Settings`; validation doesn't change |

### Audio

| Idea | Why | Effort | Notes |
|---|---|---|---|
| Microphone automatic gain control (AGC) | An even voice without touching the slider | M | New stage in `MicProcessor`, deterministic tests |
| Test the microphone (record 3 s and play it back) | Check it sounds right before playing | S | The meter exists; this adds listening |
| Pick the output device to capture (Windows) | Playing through USB headphones, not the speakers | M | WASAPI loopback of the chosen device |
| Choose or turn off the "clip saved" sound | Don't startle during a game | S | Joins wave 2 notifications |

### Everyday use

| Idea | Why | Effort | Notes |
|---|---|---|---|
| After saving: open folder / copy to clipboard / nothing | Paste the clip into chat in one step | S | Copy-file-to-clipboard differs per OS |
| Start minimized to the tray | With launch at login, don't open the window | S | Goes with the autostart setting |
| Configurable shortcuts for everything (open app, pause, mark) | One place for every shortcut | M | Generalizes the key recorder in `lib/hotkey.ts` |
| Check for updates / beta channel | The app keeps itself current | M | Tauri updater; needs signing and hosting |
| Accessibility (text size, reduced motion) | Comfortable use | S | CSS variables are centralized |

### Library

| Idea | Why | Effort | Notes |
|---|---|---|---|
| Sub-folders per game or date | Automatic order | S | `ClipService::unique_clip_path`; comes with the name template |
| Favorites / protect from automatic deletion | The storage limit must not delete what matters | M | Depends on the storage limit (wave 2) |
| Size cap for sharing (10 / 25 / 50 MB) | Clips that fit in a chat message | L | **Conflicts with a rule**: needs re-encoding, while `CLAUDE.md` says ffmpeg only stream-copies. The exception has to be decided first |

### Performance and system

| Idea | Why | Effort | Notes |
|---|---|---|---|
| Pick the GPU (laptops with two) | Encode on the dedicated or the integrated one | M | Media Foundation / VideoToolbox per OS |
| Limit CPU/GPU use or fps under load | Don't get in the game's way | L | Needs measuring; tied to video quality |
| Optional error reports | Support | M | Strictly opt-in; never send paths or captures |

## Known limits worth remembering

* Anything that restarts the **video** capture empties the replay buffer.
* In "separate track" mode many players only play the first audio track.
* Without headphones the microphone hears the game: the screen says so; noise
  suppression (wave 3) helps later.
* Microphone permission: on macOS during development it is granted to the
  Terminal; a packaged app asks with the text in `src-tauri/Info.plist`.
