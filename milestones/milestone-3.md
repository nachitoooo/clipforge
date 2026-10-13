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

## F-3.9 — Library (folders)
> **Progress (2026-10-10, branch `feat/3/library-folders`):** implemented; design
> in [`docs/library.md`](../docs/library.md). A folder is a label over the flat
> clips folder (index in `library.json`): a clip can be in several folders and
> filing never moves the file. The UI boxes stay open until someone checks them
> in the running Tauri window: they are covered by simulated-DOM tests and were
> driven in headless Chrome against a mock backend, never in the real webview.
- [x] Folder index: create, rename, edit (name, color, icon), delete; names unique ignoring capitals; atomic persistence, corrupt file kept as `.bak`, hand-edited file tidied on load
  — Rust unit tests (`folders.rs`); the Tauri command wiring in `lib.rs` has only been compiled
- [x] A clip can be in several folders; renaming, deleting or trimming a clip keeps the index in step (the trimmed copy joins the original's folders); reads never write or prune
- [x] Deleting a folder: keep the clips, or delete only the ones no other folder holds; a failure keeps the folder and says how many clips could not go
- [ ] Biblioteca view: folder grid, "Sin carpeta" section with the unfiled clips, inside a folder (breadcrumb, take a clip out), create / rename in place / edit / delete with the right confirmation
- [ ] Put clips in folders: *Carpetas…* picker on every clip card (several folders, new folder on the spot) and drag & drop onto a folder card
- [ ] Recientes shows each clip's folder labels and updates when folders change in the Biblioteca
- [ ] Windows: drag and drop with `dragDropEnabled: false` (WebView2), never run
- [ ] Manual check in the app: make folders, file clips both ways, rename / edit / delete (both outcomes), trim a clip that is in a folder

## F-3.10 — Smart library
> **Progress (2026-10-11, branch `feat/3/smart-library`):** implemented; design in
> [`docs/library.md`](../docs/library.md) (section "Smart library"). Each clip is
> tagged with the application that held the screen while it was recorded (its
> game); Recientes, the Biblioteca and every folder can be searched, filtered and
> sorted. The tags of Medal's "biblioteca inteligente" are the folders. The UI
> boxes stay open until someone checks them in the running Tauri window: they are
> covered by simulated-DOM tests and were driven in headless Chrome against a mock
> backend, never in the real webview. What comes next is planned in
> [`docs/medal-roadmap.md`](../docs/medal-roadmap.md).
- [x] Foreground-application lookup and activity log (`core/src/activity/`): the application that held the screen longest over a clip's span, ignoring ClipForge and the desktop shells; a tracker thread that stops promptly and forgets when switched off
  — unit tests with a fake source; verified 2026-10-11 against the real macOS session (`macos_activity`, ignored test)
- [x] Per-clip facts in `clipmeta.json` (game), keyed by file name, reads never write or prune, all-or-nothing writes; renaming, deleting or trimming a clip keeps them in step; folders and clip facts share one `JsonStore`
  — Rust unit tests; the Tauri command wiring in `lib.rs` has only been compiled
- [x] Game detection is a setting (`library.detectGame`, on by default), applied live
- [ ] Filter bar over Recientes, the Biblioteca root (results over every clip, matching folders) and inside a folder: search (title, game, folder names), game, folder, date, order; "N de M clips"; clear; the focus stays in the search box while the list reloads
- [ ] Each card names its game; clicking it opens «Juego del clip» to correct it for one clip or for every clip of that game
- [ ] Thumbnails are not reloaded while typing in the search box (cards are kept between renders)
- [ ] Settings → Biblioteca: the switch and its privacy note
- [ ] Windows: the foreground-window lookup (`activity/windows.rs`) is compiled and linted by the CI cross-check but never run (F-2.4 is open)
- [ ] Manual check in the app with a real game: play, save a clip with the hotkey, the tray and the button, see the game on the card; alt-tab to another app mid-clip

## F-3.11 — Automatic folders per game
> **Progress (2026-10-12):** implemented; see "Automatic folders per game" in
> [`docs/library.md`](../docs/library.md). Needs the game of F-3.10. The UI box
> stays open until it is checked in the running Tauri window.
- [x] `library.autoFolders` (off by default): a saved clip joins the folder named after its game, made on the spot if missing; a folder the user made is used untouched; same name in other capitals is the same folder; the 200-folder limit and a bad name only skip the filing, never the save
  — Rust unit tests (`folders.rs`: new, reuse, capitals, user folder untouched, several folders, stable color, limit, restart, deleted-then-recreated); the call from `AppActions::save_clip` has only been compiled
- [x] Settings → Biblioteca: the switch, what it does, unavailable (and says why) while game detection is off
- [ ] Manual check in the app with a real game: save with the hotkey, the tray and the button; the folder appears in the Biblioteca with its clip; delete it and save another
- [ ] Not yet: file the clips that already exist (a one-time action)

## F-3.12 — Copy a clip to the clipboard
> **Progress (2026-10-12):** implemented. A clip is put on the system clipboard as
> a *file*, so pasting in Discord, Slack, Mail, Messages or a folder takes it. No
> server, nothing leaves the machine. The UI boxes stay open until someone checks
> them in the running Tauri window.
- [x] `core/src/clipboard/`: `copy_file(path)` — macOS `NSPasteboard` with the file's URL, Windows `CF_HDROP` (`DROPFILES` payload built by a pure, tested function); a relative path, a missing file or a folder is refused before the clipboard is touched
  — verified 2026-10-12 against the real macOS pasteboard server (`macos_clipboard`, ignored tests, on a private pasteboard: the file comes back as a file URL, accents and spaces intact, a second copy replaces the first); the Windows code is compiled and linted by the CI cross-check, never run (F-2.4)
- [x] `copy_clip` command (name checked with `library::clip_path`, off the UI thread); «Copiar» on every card (hover actions, now 28 px so seven fit the narrowest card) and in the player
- [x] Size warning: after copying, if the clip doesn't fit Discord without Nitro (20 MB) or a mail (25 MB), the toast says so with the size and the limits (`lib/share.ts`, an editable table — the limits change; Discord's were read in its FAQ on 2026-10-12, the 25 MB of a mail is Gmail's usual limit, not re-checked)
- [ ] Manual check: copy a clip from a card and from the player, paste it in Discord (desktop and web), Mail/Messages and a Finder folder
- [ ] Windows: `CF_HDROP` never run (F-2.4)
- [ ] Not yet: drag a clip out of the window into another app (needs a native drag session), "Exportar para Discord" (needs re-encoding, see the roadmap)

## F-3.13 — Screenshots
> **Progress (2026-10-12):** implemented on macOS. A picture of the whole screen
> is saved as `captura-<date>-<time>.png` next to the clips and lives in the same
> library: Recientes, the Biblioteca, folders, search, the game and automatic
> folders all treat it like a clip, except that it can't be trimmed. The UI boxes
> stay open until someone checks them in the running Tauri window.
- [x] `core/src/screenshot/`: `capture_png(path)` — macOS `SCScreenshotManager` (14+) of the first display at its real pixel size (points × scale: 2940×1912 on a Retina laptop, like ⇧⌘3), without ClipForge's own windows; written to a `.part` file and renamed, so a failure leaves nothing the library would list; `captura-…png` names with `-2`, `-3`… for the same second
  — verified 2026-10-12 against the real screen (`macos_screenshot`, ignored tests: the PNG's own header says the size the call reported, no temporary file left, two in a row are two files); the rest is unit-tested. Windows says it isn't available yet (waits for F-2.4)
- [x] A shortcut of its own (`shortcuts.screenshot`, **none by default**: nothing is registered that nobody asked for), the tray entry «Tomar captura de pantalla» and the «Captura» button of Recientes; a notification says the size and the name
  — Rust tests (validation, repair of a hand-edited file, registration/switching with the others, dispatch); the real hotkey, tray and button are still to be tried by hand
- [x] The library holds `.png` too: `ClipInfo.kind` (`clip` | `screenshot`), `media_path` for what both kinds share (rename keeping the extension, delete, folders, game, copy) and `clip_path` for what only a video can do (the editor refuses a picture)
- [x] UI: a picture card (the image, its size, no play, no trim), a viewer, the «Tipo» filter (appears with the first screenshot), «N clips y M capturas», and «Copiar» works on a picture; Settings → Atajos → «Captura de pantalla» (record, remove)
- [ ] Manual check in the app: take a screenshot with the shortcut, the tray and the button (the first time macOS may ask for Screen Recording); see it appear in Recientes; open it, rename it, put it in a folder, copy it and paste it in a chat; make sure ClipForge's own window is not in the picture
- [ ] Windows: not implemented (needs one frame of Graphics Capture, F-2.4)
- [ ] Not yet: choosing the monitor (it is the first display, like the replay buffer), a region, an editor for pictures, folder counts that say «capturas» (they still say «clips»)

## F-3.14 — Low RAM mode
> **Progress (2026-10-12):** implemented. Settings → Rendimiento → «Modo poca
> memoria (RAM)»: the replay buffer keeps one minute at 6 Mb/s instead of two
> minutes at 12 Mb/s, about 45 MB instead of 180 MB. The UI box stays open until
> it is checked in the running app (and the real RAM measured under a game).
- [x] `memory.rs`: the two modes as plain numbers (length, byte cap, bitrate), the normal one equal to the engine's defaults, an estimate of what a full buffer weighs; `ReplayBuffer::set_config` resizes a buffer (a smaller window drops whole GOPs at once)
- [x] `settings.performance.lowRam` (off by default, old files get it off); applied when the capture starts on macOS and on Windows (bitrate and buffer); changing it restarts the video capture and empties the buffer, only if the buffer is running
- [x] Settings: a panel with the switch, the real numbers of both modes (asked to the backend, so there is one source of truth), and a confirmation before anything changes — cancel leaves it as it was
- [ ] Manual check in the app: switch it on and off, see the restart, watch the memory in Activity Monitor / Task Manager while recording (measured since, on macOS, see F-3.18: the whole app was ~190–215 MB on a quiet desktop, with the window open and a two-minute buffer)
- [ ] Not yet: a free choice of buffer length and quality (the wave 1 settings), and a bitrate that adapts to the screen size

## F-3.15 — Visual pass
> **Progress (2026-10-12):** done in CSS, with two tiny DOM touches. Not a UI
> library: ClipForge's views are plain TypeScript, and a React kit (Arc was
> looked at) would mean rewriting every view and its tests. The look is now set by
> tokens in `:root` of `app/src/styles.css`.
- [x] Surfaces in layers (window, card, raised, input), hairline borders instead of flat grey ones, one radius scale, shadows for cards and for what floats (dialogs, the player), a faint warm glow behind the page title
- [x] Inter bundled (Latin subset, 48 KB, SIL OFL 1.1) so Windows and macOS show the same letters; thin dark scrollbars; orange text selection
- [x] Selectors drawn by us (no native arrow); a selector that is narrowing the list is tinted (`is-set`, tested); a magnifier in the search box
- [x] Rail: marker on the active item, glow on the logo; cards lift on hover, a shade under the duration badge, actions that slide in; day titles with a rule; primary buttons with a gradient and glow; key-cap shortcut buttons; panel titles on a tinted tile; dialogs and the player pop in over a blurred backdrop
- [x] Looked at in headless Chrome over a mock backend: Recientes, Biblioteca, a folder, Configuración, the player, the editor, the folder / game / low RAM dialogs, filters on, empty library, and a 780 px window
- [ ] Manual check in the real app, in WebView2 (Windows) and WKWebView (macOS): the font, the blurred backdrops, the scrollbars and the native dropdown list of the selectors
- [ ] Not yet: a labelled (wide) sidebar, thumbnails that preview more than one frame, motion between views (the light theme and the choice of color came next: F-3.16)

## F-3.16 — Light theme, main color, drop-down filters
> **Progress (2026-10-12):** implemented. Settings → Apariencia: the theme
> (Sistema / Oscuro / Claro) and the main color (nine swatches or any `#rrggbb`),
> both applied at once. The filters of Recientes and Biblioteca are drop-down
> lists of our own instead of the browser's `<select>`. The solid orange fill is
> gone from the buttons: the main action is a colored line and text.
- [x] `settings.appearance { theme, accent }` in Rust: defaults (dark, `#ff7a1a`), old files get them, `Settings::validated` lowercases the color and puts an unknown theme or a bad color back to the default, changing it never restarts the capture
- [x] `lib/theme.ts` (pure, tested): the same color rule as Rust, WCAG contrast, `readableAccent` (the accent as text or a line is darkened on light / lightened on dark until it reads at 4.5:1 — yellow on white would not), `accentVars`, `resolveScheme`
- [x] `ui/appearance.ts`: `data-theme` and the accent as custom properties on the root; `system` follows the operating system while the app is open; applied at start and on every `settings-changed`
- [x] Stylesheet: a set of tokens per theme (`:root` and `:root[data-theme="light"]`), the accent derived with `color-mix` (soft tint, focus ring, glow); glass over pictures, the video stage and the thumbnails stay dark in both themes
- [x] Settings → Apariencia (first panel): segmented theme control, swatches, a hex box that saves only a complete color and puts the real one back on leaving; tests for each path, including a failed save
- [x] `ui/dropdown.ts` (tested): trigger + floating list with a tick on the choice, arrows / Home / End / Enter / Space / Escape / Tab, typing to jump, opens upwards when there is no room, closes on an outside click, scroll or resize, one open at a time, Escape does not reach a player or editor behind it; the filter bar uses it and a filter that narrows the list is drawn in the accent
- [x] Flat buttons: no fill from the accent, and no background, movement or hover change at rest, under the pointer or pressed; the current section, the chosen theme and the other selected buttons are told by the accent color and a line (checked in headless Chrome: identical computed style at rest / hover / pressed). Cards still lift on hover; they are not buttons
- [x] Looked at in headless Chrome over a mock backend: dark and light, orange / blue / green, the lists open, filters on, the editor, a dialog, and `system` with the OS switched both ways
- [ ] Manual check in the real app (WebView2 and WKWebView): the theme and color change, the lists near the window's edge, `system` when the OS theme changes
- [ ] Not yet: the two selects in Settings (microphone, clip length) are still the browser's; the system's own title bar and the tray menu do not follow the theme

## F-3.17 — Copy each clip when it is saved
> **Progress (2026-10-12):** implemented. Settings → Biblioteca → «Copiar cada clip
> al portapapeles al guardarlo»: when a clip is saved (shortcut, tray or button) it is
> already on the clipboard as a file, ready to paste in Discord, Mail or a folder.
> Off by default, because it replaces whatever was copied before.
- [x] `settings.library.copyOnSave` (default off, old files get it off, never restarts the capture); a Settings switch with a note that it replaces the clipboard and that screenshots are not copied
- [x] `AppActions::save_clip` copies the new file after the clip is registered, the same path for the hotkey, the tray and the button; `dispatch::CopyOutcome::attempt` only runs the copy when asked, and a copy that fails is reported in the notification («no se pudo copiar: …») and never fails the save; `saved_description` builds the text («30 s → clip.mp4 · copiado al portapapeles»)
  — Rust tests (the copy is not run when off, its reason is kept when it fails, the three texts, the setting's default / old file / persistence / JSON name) and simulated-DOM tests; `copy_file` itself was verified against the real macOS pasteboard in F-3.12
- [ ] Manual check in the app: switch it on, save with the shortcut, the tray and the button, paste in Discord and in a Finder folder (the wiring in `lib.rs` has no test of its own: it needs a running Tauri app)
- [ ] Windows: `CF_HDROP` never run (F-2.4)
- [ ] Not yet: copying screenshots too, an option «abrir la carpeta» after saving

## F-3.18 — Less RAM
> **Progress (2026-10-12):** implemented and measured on macOS. The replay buffer
> is no longer a fixed two minutes: it holds what the longest save shortcut needs
> plus a margin of two GOPs (34 s with the usual 30 s clip). Measured with the real
> screen and the real engine, a busy screen at 1080p60 (a temporary probe, not kept
> in the repo), 150 s each: a 120 s buffer held 175.5 MB of video (11.9 Mb/s) and the
> process footprint reached ~250 MB; a 34 s buffer held 47.3 MB and the footprint
> stayed at 76–95 MB, about 160 MB less. The clip you get is the same.
- [x] `memory::MemoryPlan::sized_for`: buffer = longest clip + 4 s (never less than 10 s, never more than the mode's ceiling: 2 min, or 1 min in low RAM); the byte cap shrinks with it but stays at twice what the buffer should weigh, still a safety net; `Settings::longest_clip_seconds` takes the main shortcut and the extra ones
- [x] Applied live: `Recorder::apply_live_controls` resizes the eviction window when a clip length changes (shorter gives memory back at once, longer fills as time goes by) and when the capture starts; no restart. `memory_plans` takes the settings so the Rendimiento panel shows the real numbers, asks again after a change, and says why the buffer is as long as it is
- [x] Windows packets: `annexb::parse_access_unit` sized each picture by growing a vector, which left up to half of every packet as unused capacity; now one exact allocation (the new test fails without it: 50 048 bytes of capacity for 35 010 of data)
- [x] Clip thumbnails (`ui/lazyvideo.ts`): a `<video>` per card stayed loaded for ever once scrolled to, a few MB each (the first dozen cost ~44 MB in the real window: 38 → 82 MB with 40 clips). They now load 400 px from the scroller and unload 1600 px away; checked in headless Chrome with 80 clips (never more than 30 loaded, always a picture on the cards that show)
- [x] Tried and dropped: ScreenCaptureKit's `queueDepth` 8 → 3 changed no measurable footprint (the dirty IOSurface memory was 11 MB in both runs) and risked dropped frames
- [ ] Windows: not measured (the packet fix is unit-tested and compiled by the CI cross-check)
- [ ] Manual check with a real game and Activity Monitor / Task Manager
- [ ] Not yet: a frame pool for the raw frames (each 8 MB frame is allocated and freed 60 times a second), capturing in NV12 on macOS (a third of the bytes of BGRA), thumbnails as small pictures instead of paused videos, a fixed buffer length chosen by the user

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
- [x] Default clip length (15 s / 30 s / 1 min / 2 min; any value from 5 s to the buffer's 2 min): `shortcuts.clipSeconds` is what the main shortcut, the tray entry and the window's button save, and the three show it
  — Rust tests (clamping, old files get 30 s, tray command); the tray label and the shortcut with a real press are still to be tried by hand
- [x] Extra save shortcuts, each with its own length (up to 4: F9 for the last 15 s, F10 for the last minute…): same key rules as the main one, all different from each other and from the mute shortcut, moved all-or-nothing with the others, a length change under the same keys needs no OS call; the Atajos panel adds, re-records, changes the length of and removes them
  — Rust tests (validation, repair of a hand-edited file, `switch` with N shortcuts and rollback) and simulated-DOM tests; driven in headless Chrome against a mock backend, never in the real webview
- [ ] Clips folder
- [x] Save-clip shortcut, configurable in Settings → Atajos (record a key combination or a single key that types nothing — F1–F24, Insert, Inicio, Fin, RePág, AvPág, Pausa, ImprPant — "Restablecer" goes back to the default); applied at once, refused with a reason if the OS or another app owns it, kept apart from the mute shortcut; the tray hint and the gallery button show the current one
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
