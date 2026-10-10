//! ClipForge core engine.
//!
//! Pipeline: [`capture`] (per-OS frame sources) → [`encode`] (hardware
//! encoders) → [`buffer`] (encoded-frame ring buffer) → [`clip`] (extract +
//! remux). The ring buffer stores *encoded* frames so RAM stays bounded and
//! saving a clip is a remux, never a re-encode.

pub mod audio;
pub mod buffer;
pub mod capture;
pub mod clip;
pub mod encode;
pub mod engine;
