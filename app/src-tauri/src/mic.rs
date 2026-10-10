//! Microphone logic that doesn't need a microphone: deciding *what to do*
//! when settings change, and turning raw facts into the status the UI shows.
//! Kept free of Tauri and audio-hardware types so it is unit testable; the
//! `Recorder` does the actual attaching.

use std::time::Duration;

use serde::Serialize;

use crate::settings::{MicMode, MicrophoneSettings};

/// How long the microphone must be live, unmuted and perfectly silent before
/// the UI suggests the permission may be missing. Real microphones always
/// carry a noise floor; a denied one delivers exact zeros.
pub const NO_SIGNAL_AFTER: Duration = Duration::from_secs(3);

/// What is attached right now (the part of the settings that needs a restart
/// of the microphone stream when it changes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMic {
    pub device_id: Option<String>,
    pub mode: MicMode,
}

impl AppliedMic {
    pub fn from_settings(s: &MicrophoneSettings) -> Self {
        Self {
            device_id: s.device_id.clone(),
            mode: s.mode,
        }
    }
}

/// What to do about the microphone after a settings change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicAction {
    /// Already as wanted (volume and mute are applied live regardless).
    Nothing,
    /// Switch it off.
    Detach,
    /// (Re)start it with the wanted device and mode.
    Attach,
}

/// Decide the action. The microphone only runs while the replay buffer does
/// (it shares the video's clock); starting the buffer applies the settings.
pub fn plan(
    wanted: &MicrophoneSettings,
    attached: Option<&AppliedMic>,
    buffer_running: bool,
) -> MicAction {
    let should_run = wanted.enabled && buffer_running;
    match (should_run, attached) {
        (false, None) => MicAction::Nothing,
        (false, Some(_)) => MicAction::Detach,
        (true, Some(current)) if *current == AppliedMic::from_settings(wanted) => {
            MicAction::Nothing
        }
        (true, _) => MicAction::Attach,
    }
}

/// What the Settings screen shows about the microphone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicStatus {
    pub enabled: bool,
    /// A microphone stream is attached and healthy.
    pub active: bool,
    pub muted: bool,
    /// Why it isn't working although it is enabled.
    pub error: Option<String>,
    /// Attached and unmuted, yet nothing but digital silence for a while:
    /// usually a missing microphone permission.
    pub no_signal: bool,
    /// False on platforms without microphone support.
    pub supported: bool,
}

/// The raw facts [`status`] is computed from.
pub struct MicFacts {
    pub enabled: bool,
    pub muted: bool,
    pub attached: bool,
    /// Why the last attach attempt failed.
    pub attach_error: Option<String>,
    /// A stream error reported while running (device unplugged, ...).
    pub stream_error: Option<String>,
    /// How long it has been attached.
    pub attached_for: Option<Duration>,
    /// Whether anything above digital silence ever came in.
    pub heard_signal: bool,
    pub supported: bool,
}

pub fn status(f: MicFacts) -> MicStatus {
    let error = if f.enabled {
        f.attach_error.or(f.stream_error)
    } else {
        None
    };
    let active = f.enabled && f.attached && error.is_none();
    let no_signal = active
        && !f.muted
        && !f.heard_signal
        && f.attached_for.is_some_and(|d| d >= NO_SIGNAL_AFTER);
    MicStatus {
        enabled: f.enabled,
        active,
        muted: f.muted,
        error,
        no_signal,
        supported: f.supported,
    }
}

/// A microphone offered in the device picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Linear gain for a percentage setting (100 % = 1.0).
pub fn gain_from_percent(percent: u32) -> f32 {
    percent as f32 / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wanted(enabled: bool) -> MicrophoneSettings {
        MicrophoneSettings {
            enabled,
            ..Default::default()
        }
    }

    fn applied(device: Option<&str>, mode: MicMode) -> AppliedMic {
        AppliedMic {
            device_id: device.map(String::from),
            mode,
        }
    }

    #[test]
    fn disabled_and_detached_needs_nothing() {
        assert_eq!(plan(&wanted(false), None, true), MicAction::Nothing);
        assert_eq!(plan(&wanted(false), None, false), MicAction::Nothing);
    }

    #[test]
    fn switching_off_detaches() {
        let current = applied(None, MicMode::Mixed);
        assert_eq!(
            plan(&wanted(false), Some(&current), true),
            MicAction::Detach
        );
    }

    #[test]
    fn switching_on_attaches_only_while_the_buffer_runs() {
        assert_eq!(plan(&wanted(true), None, true), MicAction::Attach);
        // No buffer, no clock to share: start of the buffer will apply it.
        assert_eq!(plan(&wanted(true), None, false), MicAction::Nothing);
    }

    #[test]
    fn the_buffer_stopping_detaches_a_running_microphone() {
        let current = applied(None, MicMode::Mixed);
        assert_eq!(
            plan(&wanted(true), Some(&current), false),
            MicAction::Detach
        );
    }

    #[test]
    fn unchanged_device_and_mode_leave_the_stream_alone() {
        let current = applied(None, MicMode::Mixed);
        assert_eq!(
            plan(&wanted(true), Some(&current), true),
            MicAction::Nothing
        );
    }

    #[test]
    fn changing_device_or_mode_restarts_the_microphone() {
        let mut s = wanted(true);
        s.device_id = Some("coreaudio:USB".into());
        let default_mic = applied(None, MicMode::Mixed);
        assert_eq!(plan(&s, Some(&default_mic), true), MicAction::Attach);

        let mut s = wanted(true);
        s.mode = MicMode::Separate;
        assert_eq!(plan(&s, Some(&default_mic), true), MicAction::Attach);
    }

    fn facts() -> MicFacts {
        MicFacts {
            enabled: true,
            muted: false,
            attached: true,
            attach_error: None,
            stream_error: None,
            attached_for: Some(Duration::from_secs(10)),
            heard_signal: true,
            supported: true,
        }
    }

    #[test]
    fn a_healthy_microphone_is_active() {
        let s = status(facts());
        assert!(s.active && !s.no_signal && s.error.is_none());
    }

    #[test]
    fn a_disabled_microphone_reports_no_errors_even_if_stale() {
        let s = status(MicFacts {
            enabled: false,
            attached: false,
            attach_error: Some("viejo".into()),
            ..facts()
        });
        assert!(!s.active && s.error.is_none() && !s.enabled);
    }

    #[test]
    fn an_attach_failure_is_surfaced_and_means_inactive() {
        let s = status(MicFacts {
            attached: false,
            attach_error: Some("permiso denegado".into()),
            ..facts()
        });
        assert!(!s.active);
        assert_eq!(s.error.as_deref(), Some("permiso denegado"));
    }

    #[test]
    fn a_stream_error_while_running_deactivates_it() {
        let s = status(MicFacts {
            stream_error: Some("el micrófono se desconectó".into()),
            ..facts()
        });
        assert!(!s.active);
        assert!(s.error.unwrap().contains("desconectó"));
    }

    #[test]
    fn silence_is_only_suspicious_after_a_while_and_when_not_muted() {
        let silent = MicFacts {
            heard_signal: false,
            ..facts()
        };
        assert!(
            !status(MicFacts {
                attached_for: Some(Duration::from_secs(1)),
                ..silent
            })
            .no_signal
        );

        let long_silent = MicFacts {
            heard_signal: false,
            attached_for: Some(NO_SIGNAL_AFTER),
            ..facts()
        };
        assert!(status(long_silent).no_signal);

        let muted = MicFacts {
            heard_signal: false,
            muted: true,
            attached_for: Some(Duration::from_secs(30)),
            ..facts()
        };
        assert!(!status(muted).no_signal, "muted silence is expected");
    }

    #[test]
    fn gain_percent_maps_to_linear_gain() {
        assert_eq!(gain_from_percent(100), 1.0);
        assert_eq!(gain_from_percent(0), 0.0);
        assert_eq!(gain_from_percent(200), 2.0);
        assert_eq!(gain_from_percent(50), 0.5);
    }

    #[test]
    fn status_json_is_camel_case() {
        let json = serde_json::to_value(status(facts())).unwrap();
        assert!(json["noSignal"].is_boolean());
        assert!(json["supported"].is_boolean());
    }
}
