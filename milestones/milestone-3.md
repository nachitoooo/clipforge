# Milestone 3 — Clips Library UI

**Goal:** a Tauri UI to browse, preview, trim, and manage saved clips, plus
app settings.

## F-3.1 — Clips gallery
> **Progress (2026-10-09, branch `feat/3/clips-gallery`):** implemented and
> covered by unit tests (`library.rs` in Rust; `lib/*.test.ts` via `npm test`).
> Boxes stay open until each item is checked by hand in the running app — the
> thumbnails, playback and live-update paths have only been compiled, not seen.
- [ ] Grid of saved clips with auto-generated thumbnails
- [ ] Inline playback (click to preview)
- [ ] Delete / reveal-in-folder / rename actions
- [ ] Gallery updates live when a new clip is saved

## F-3.2 — Clip editor (trim + audio lanes)
> **Progress (2026-10-09, branch `feat/3/clip-editor`):** implemented. The cut
> is done in-process by our own muxer (`core/src/clip/trim.rs`, see
> [`docs/clip-editor.md`](../docs/clip-editor.md)) — no ffmpeg sidecar. The UI
> boxes stay open until someone checks them in the running Tauri window: they
> are covered by simulated-DOM tests and were driven in headless Chrome against
> a mock backend, never in the real webview.
- [ ] Timeline with in/out handles over the clip preview (drag, arrow keys, I/O keys, typed times, play the selection, loop)
- [x] Export the trimmed clip as a stream copy with a **frame-accurate** start (a video edit list hides the frames before the requested start; no re-encode)
  — verified 2026-10-09 with ffprobe/ffmpeg and with AVFoundation (QuickTime/Safari's engine): exact duration, frame count and first frame; audio tones land where the cut says, including after a second cut
- [x] Original clip preserved; trimmed copy saved alongside as `<name>-recorte.mp4` (never overwrites: `-recorte-2`, ...)
- [ ] When the clip has sound, one lane per audio track under the video lane, with its waveform; each track can be left out of the copy (a silent clip shows no audio lanes)
  — waveform decoded in Rust (symphonia); the real pipeline's two-track clips (real AAC, voice starting late) cut correctly, checked with AVFoundation
- [ ] Manual check in the app: open a clip from the gallery (scissors on the card, or "Recortar" in the player), cut it, play the copy in QuickTime; try a clip with a separate microphone track

## F-3.3 — Settings foundation + microphone
> **Progress (2026-10-09, branch `feat/3/settings-microphone`):** implemented
> and verified by tests (see below). Open boxes are things only a person at the
> keyboard (or a Windows machine) can check. Roadmap of the next settings:
> [`docs/settings-roadmap.md`](../docs/settings-roadmap.md).
- [x] Settings persisted as JSON in the app config folder: defaults for every field, atomic writes, a corrupt file is kept as `.bak`, one validation gate for everything the UI sends
- [x] Applied without restarting the app: volume and mute are instant; changing device or mode restarts only the microphone stream — never the video, so the buffer is kept
- [x] Microphone section: on/off (opt-in), device picker, volume 0–200 %, level meter, mute with a configurable global shortcut, clear error messages with retry
  — UI behavior covered by 24 simulated-DOM tests; the whole stack ran against the real microphone on macOS (`macos_mic` ignored tests)
- [x] Two ways to save the voice, chosen in Settings: mixed into the game track (plays everywhere) or on its own track (can be muted later)
  — verified 2026-10-09 by decoding clips with AVFoundation: mixed carries both tones in one track, separate keeps each tone on its own track
- [ ] Manual check on macOS: grant the microphone permission, talk and watch the meter, move the volume, mute with the shortcut, change device/mode, listen to the saved clip in QuickTime
- [ ] Windows: the microphone (`cpal`/WASAPI) is compiled and linted for `x86_64-pc-windows-msvc` but never run; `cargo test -p clipforge-core --test windows_audio -- --ignored` on a Windows machine, after F-2.4
- [ ] Launch-at-login toggle (moved to wave 1, F-3.5)

## F-3.5 — Settings, wave 1
- [ ] Buffer length (60 / 120 / 180 s)
- [ ] Default clip length (15 / 30 / 60 / 120 s)
- [ ] Clips folder
- [x] Save-clip shortcut, configurable in Settings → Atajos (record a key combination, "Restablecer" goes back to the default); applied at once, refused with a reason if the OS or another app owns it, kept apart from the mute shortcut; the tray hint and the gallery button show the current one
  — Rust tests (validation, repair of a hand-edited file, all-or-nothing registration with a fake registry) and simulated-DOM tests; pressing a custom shortcut in the real app is still to be tried by hand
- [ ] Launch at login
- [ ] Video quality presets (warn that changing it empties the buffer)
- [ ] Post-roll: keep recording 0 / 5 / 10 / 15 s after the save shortcut
- [ ] Pause the buffer with a shortcut + tray icon with state
- [ ] Push-to-talk for the microphone
- [ ] Show / hide the cursor in clips (warn that it empties the buffer)

## F-3.6 — Settings, wave 2
- [ ] Game audio on/off and volume
- [ ] Storage limit / delete old clips
- [ ] Notifications and sound on save
- [ ] Close button behavior
- [ ] File name template

## F-3.7 — Settings, wave 3
- [ ] Choose the monitor
- [ ] Capture a single window / game
- [ ] Per-application audio
- [ ] Start the buffer only for known games
- [ ] Microphone noise suppression and gate
- [ ] Language (es / en)
- [ ] Theme (dark / light / auto)

## F-3.8 — Settings, wave 4
- [ ] HEVC codec
- [ ] Hardware encoder selection
- [ ] Automatic highlight detection
- [ ] Share (clipboard, link)
- [ ] Diagnostics: stats, logs folder, reset, export/import settings

## F-3.4 — UI E2E
- [ ] tauri-driver + WebdriverIO smoke suite: open app, see gallery, change a setting
- [ ] E2E runs in CI on both OS targets
