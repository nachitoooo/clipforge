# ClipForge

Cross-platform (Windows + macOS) desktop app for game highlight clips: a
continuous replay buffer records gameplay in the background and a global
hotkey saves the last N seconds as a shareable .mp4.

**Stack:** Rust core (per-OS capture + hardware encoding) · Tauri 2 UI.

## How this repo works

- `CLAUDE.md` — the operating manual: stack, conventions, testing rules, build commands.
- `milestones/` — feature tracking with IDs and acceptance criteria; the single source of truth for progress.
- `milestones/backlog-improvements.md` — ideas parking lot.

Start by reading `CLAUDE.md`, then `milestones/milestone-1.md`.
