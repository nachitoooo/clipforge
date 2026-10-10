# Improvement Backlog

Ideas and wishes not yet promoted to a milestone. Add entries immediately when
they come up (see CLAUDE.md → Improvement Backlog).

---

## Sidebar layout: Recientes / Biblioteca / Configuración
- **Related to:** Milestone 3 (UI)
- **Why:** Ignacio's target layout for the app (Medal-style navigation).
- **Details:** left sidebar with (top→bottom): a "clips recientes" icon showing
  clips as they are generated; a "biblioteca" icon that will later organize
  clips into folders; pinned at the bottom, the settings (configuración) icon.
- **Status (2026-10-09):** sidebar shell built in `feat/3/clips-gallery`
  (Recientes = gallery; Biblioteca and Configuración are "Próximamente"
  placeholders until folders and F-3.3 exist).
- **Added:** 2026-07-07

## Embed libobs as capture/encode engine
- **Related to:** F-1.3, F-1.4, F-2.4 (alternative implementation)
- **Why:** libobs (the OBS engine, C API callable from Rust) ships a
  battle-tested replay buffer + every hardware encoder. If the per-OS
  capture/encode plumbing gets expensive, embedding it buys years of edge-case
  fixes at the cost of build complexity and binary size.
- **Details:** evaluate after F-1.4 — compare frame-drop rate and dev effort
  against the native implementation before deciding for Windows parity.
- **Added:** 2026-07-07

## Automatic highlight detection
- **Related to:** new (post-M3)
- **Why:** the killer feature of Medal/Outplayed — clips that save themselves
  on kills/wins instead of requiring a hotkey press.
- **Details:** start simple: audio spike detection and/or per-game event hooks
  (e.g., parsing game log files); ML/vision detection is a later iteration.
- **Added:** 2026-07-07

## Microphone track
- **Related to:** F-2.5 (system audio is done), F-2.2 (remux)
- **Why:** a separate mic track lets users mute themselves after the fact.
- **Details:** system/game audio shipped in F-2.5. The mic would be a second
  AAC track: ScreenCaptureKit microphone output on macOS 15+ (needs the
  Microphone permission) / WASAPI capture of the default input on Windows.
  The pipeline already carries one audio stream end to end; a second one needs
  a second encoder, a second buffer lane and a second `trak` in the muxer.
- **Status (2026-10-09):** promoted to F-3.3 and implemented (mixed or
  separate track, configurable in Settings). Echo/noise handling is in the
  settings roadmap (wave 3).
- **Added:** 2026-07-07

## More settings ideas
- **Related to:** F-3.5 – F-3.8 (settings waves)
- **Why:** the settings roadmap covered the obvious settings; these are the
  ones that came up when asking "what else could go in Configuración".
- **Details:** full list with effort and notes in `docs/settings-roadmap.md`
  → "Ideas pool". Four of them were promoted to wave 1 (post-roll, pause the
  buffer + tray state, push-to-talk, show/hide cursor). Open decision: a size
  cap for sharing (Discord limits) needs re-encoding, which `CLAUDE.md` forbids
  for ffmpeg, so it needs an explicit exception first.
- **Added:** 2026-10-09

## Clip editor follow-ups
- **Related to:** F-3.2
- **Why:** the first editor cuts and drops tracks; these are what people will
  ask for next.
- **Details:**
  * Timeline zoom (a 2-minute clip gives ~9 px per second; fine cuts rely on
    the keyboard and the time boxes today).
  * Per-track volume and fades: needs decoding and re-encoding the AAC, which
    breaks the "never re-encode" rule — decide the exception first (same
    question as the size cap for sharing in "More settings ideas").
  * Name the tracks inside the file so the editor can label them exactly
    (today: first = Juego, second = Micrófono, by convention).
  * Preview every track (the webview plays only the first): decode to Web
    Audio, or expose `audioTracks` where the engine supports it.
  * Split one clip into several, join clips, "replace the original" after a cut.
  * Undo/redo and a draggable selection body (slide the whole cut).
- **Added:** 2026-10-09

