# Improvement Backlog

Ideas and wishes not yet promoted to a milestone. Add entries immediately when
they come up (see CLAUDE.md → Improvement Backlog).

---

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

## Microphone + game audio tracks
- **Related to:** F-1.4 (encoder), F-2.2 (remux)
- **Why:** clips without game audio are near-useless for sharing; separate mic
  track lets users mute themselves after the fact.
- **Details:** audio capture per OS (WASAPI loopback / ScreenCaptureKit audio),
  encode AAC, mux as separate tracks.
- **Added:** 2026-07-07
