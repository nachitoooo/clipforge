//! ClipForge core engine.
//!
//! Pipeline: [`capture`] (per-OS frame sources) → [`encode`] (hardware
//! encoders) → [`buffer`] (encoded-frame ring buffer) → [`clip`] (extract +
//! remux). System audio follows the same path through [`audio`] (AAC) and is
//! muxed into the clip as a second track. The ring buffer stores *encoded*
//! data so RAM stays bounded and saving a clip is a remux, never a
//! re-encode.

pub mod activity;
pub mod audio;
pub mod buffer;
pub mod capture;
pub mod clip;
pub mod clipboard;
pub mod encode;
pub mod engine;
pub mod screenshot;
