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

## F-3.2 — Trim editor
- [ ] Timeline with in/out handles over the clip preview
- [ ] Export trimmed clip (ffmpeg stream copy, keyframe-snapped)
- [ ] Original clip preserved; trimmed copy saved alongside

## F-3.3 — Settings
- [ ] Buffer duration, resolution/fps/bitrate, clips folder, hotkey remap
- [ ] Settings persisted and applied without app restart
- [ ] Launch-at-login toggle (per OS)

## F-3.4 — UI E2E
- [ ] tauri-driver + WebdriverIO smoke suite: open app, see gallery, change a setting
- [ ] E2E runs in CI on both OS targets
