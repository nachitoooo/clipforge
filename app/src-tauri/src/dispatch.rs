//! Hotkey/tray → command dispatch, kept free of Tauri types so it is unit
//! testable with mocked events (F-2.3 acceptance).

/// Everything a hotkey press or tray click can trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ToggleBuffer,
    /// Save the last this-many seconds; `None` = the length set in Settings
    /// (the tray entry and the window's button).
    SaveClip(Option<u64>),
    /// A picture of the screen, saved next to the clips.
    Screenshot,
    OpenClipsFolder,
    /// Silence or restore the microphone (hotkey or tray).
    ToggleMicMute,
    Quit,
}

/// What toggling the microphone mute did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteOutcome {
    Muted,
    Unmuted,
    /// The microphone is switched off in Settings, so there is nothing to mute.
    MicOff,
}

impl Command {
    /// Tray menu item ids (see `tray` setup in `lib.rs`).
    pub fn from_menu_id(id: &str) -> Option<Self> {
        match id {
            "toggle_buffer" => Some(Self::ToggleBuffer),
            "save_clip" => Some(Self::SaveClip(None)),
            "screenshot" => Some(Self::Screenshot),
            "open_folder" => Some(Self::OpenClipsFolder),
            "toggle_mic_mute" => Some(Self::ToggleMicMute),
            "quit" => Some(Self::Quit),
            _ => None,
        }
    }
}

/// What became of the clipboard when a clip was saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOutcome {
    /// Copying on save is switched off.
    NotAsked,
    Copied,
    Failed(String),
}

impl CopyOutcome {
    /// Run `copy` only when the setting asks for it. A clip that could not be
    /// copied is still a saved clip: the failure is reported, never raised.
    pub fn attempt(asked: bool, copy: impl FnOnce() -> Result<(), String>) -> Self {
        if !asked {
            return Self::NotAsked;
        }
        match copy() {
            Ok(()) => Self::Copied,
            Err(reason) => Self::Failed(reason),
        }
    }
}

/// What the "clip saved" notification says: the length and the file, then what
/// happened to the clipboard if it was asked to take the clip.
pub fn saved_description(seconds: f64, name: &str, copy: &CopyOutcome) -> String {
    let saved = format!("{seconds:.0} s → {name}");
    match copy {
        CopyOutcome::NotAsked => saved,
        CopyOutcome::Copied => format!("{saved} · copiado al portapapeles"),
        CopyOutcome::Failed(reason) => format!("{saved} · no se pudo copiar: {reason}"),
    }
}

/// Side effects the dispatcher can request — implemented by the real app,
/// mocked in tests.
pub trait Actions {
    fn buffer_running(&self) -> bool;
    fn start_buffer(&self) -> Result<(), String>;
    fn stop_buffer(&self);
    /// Saves the last `seconds` (the length from Settings when `None`) and
    /// returns a human-readable description of the saved clip.
    fn save_clip(&self, seconds: Option<u64>) -> Result<String, String>;
    /// Saves a screenshot and returns a human-readable description of it.
    fn take_screenshot(&self) -> Result<String, String>;
    fn open_clips_folder(&self);
    fn toggle_mic_mute(&self) -> Result<MuteOutcome, String>;
    fn quit(&self);
    fn notify(&self, title: &str, body: &str);
}

/// Execute `command` against `actions`, notifying outcomes. This is the one
/// place that decides *what happens* on a hotkey press or tray click.
pub fn dispatch(command: Command, actions: &impl Actions) {
    match command {
        Command::ToggleBuffer => {
            if actions.buffer_running() {
                actions.stop_buffer();
                actions.notify("ClipForge", "Buffer detenido");
            } else {
                match actions.start_buffer() {
                    Ok(()) => actions.notify("ClipForge", "Buffer de replay iniciado"),
                    Err(e) => actions.notify("ClipForge — error", &e),
                }
            }
        }
        Command::SaveClip(seconds) => match actions.save_clip(seconds) {
            Ok(description) => actions.notify("Clip guardado", &description),
            Err(e) => actions.notify("No se pudo guardar el clip", &e),
        },
        Command::Screenshot => match actions.take_screenshot() {
            Ok(description) => actions.notify("Captura guardada", &description),
            Err(e) => actions.notify("No se pudo tomar la captura", &e),
        },
        Command::OpenClipsFolder => actions.open_clips_folder(),
        Command::ToggleMicMute => match actions.toggle_mic_mute() {
            Ok(MuteOutcome::Muted) => actions.notify("Micrófono", "Micrófono silenciado"),
            Ok(MuteOutcome::Unmuted) => actions.notify("Micrófono", "Micrófono activado"),
            Ok(MuteOutcome::MicOff) => actions.notify(
                "Micrófono",
                "El micrófono está desactivado: activalo en Configuración",
            ),
            Err(e) => actions.notify("No se pudo cambiar el micrófono", &e),
        },
        Command::Quit => actions.quit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Mock {
        running: std::cell::Cell<bool>,
        save_result: Option<Result<String, String>>,
        screenshot_result: Option<Result<String, String>>,
        calls: RefCell<Vec<String>>,
        notifications: RefCell<Vec<(String, String)>>,
        mute_result: Option<Result<MuteOutcome, String>>,
    }

    impl Actions for Mock {
        fn buffer_running(&self) -> bool {
            self.running.get()
        }
        fn start_buffer(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("start".into());
            self.running.set(true);
            Ok(())
        }
        fn stop_buffer(&self) {
            self.calls.borrow_mut().push("stop".into());
            self.running.set(false);
        }
        fn save_clip(&self, seconds: Option<u64>) -> Result<String, String> {
            self.calls.borrow_mut().push(match seconds {
                Some(s) => format!("save {s}"),
                None => "save".into(),
            });
            self.save_result.clone().unwrap_or(Ok("clip 30 s".into()))
        }
        fn take_screenshot(&self) -> Result<String, String> {
            self.calls.borrow_mut().push("screenshot".into());
            self.screenshot_result
                .clone()
                .unwrap_or(Ok("2940×1912 → captura.png".into()))
        }
        fn open_clips_folder(&self) {
            self.calls.borrow_mut().push("open".into());
        }
        fn toggle_mic_mute(&self) -> Result<MuteOutcome, String> {
            self.calls.borrow_mut().push("mute".into());
            self.mute_result.clone().unwrap_or(Ok(MuteOutcome::Muted))
        }
        fn quit(&self) {
            self.calls.borrow_mut().push("quit".into());
        }
        fn notify(&self, title: &str, body: &str) {
            self.notifications
                .borrow_mut()
                .push((title.into(), body.into()));
        }
    }

    #[test]
    fn menu_ids_map_to_commands() {
        assert_eq!(
            Command::from_menu_id("toggle_buffer"),
            Some(Command::ToggleBuffer)
        );
        assert_eq!(
            Command::from_menu_id("save_clip"),
            Some(Command::SaveClip(None)),
            "the tray saves the length set in Settings"
        );
        assert_eq!(
            Command::from_menu_id("open_folder"),
            Some(Command::OpenClipsFolder)
        );
        assert_eq!(
            Command::from_menu_id("toggle_mic_mute"),
            Some(Command::ToggleMicMute)
        );
        assert_eq!(Command::from_menu_id("quit"), Some(Command::Quit));
        assert_eq!(Command::from_menu_id("unknown"), None);
    }

    #[test]
    fn the_clipboard_is_left_alone_unless_asked() {
        let mut touched = false;
        let outcome = CopyOutcome::attempt(false, || {
            touched = true;
            Ok(())
        });
        assert_eq!(outcome, CopyOutcome::NotAsked);
        assert!(!touched, "switched off: the copy must not even run");
    }

    #[test]
    fn a_copy_that_works_is_reported_as_copied() {
        assert_eq!(CopyOutcome::attempt(true, || Ok(())), CopyOutcome::Copied);
    }

    #[test]
    fn a_copy_that_fails_keeps_its_reason_instead_of_failing_the_save() {
        assert_eq!(
            CopyOutcome::attempt(true, || Err("el clip ya no existe".into())),
            CopyOutcome::Failed("el clip ya no existe".into())
        );
    }

    #[test]
    fn the_notification_names_the_clip_and_what_happened_to_the_clipboard() {
        let plain = saved_description(29.6, "clip.mp4", &CopyOutcome::NotAsked);
        assert_eq!(plain, "30 s → clip.mp4");
        assert_eq!(
            saved_description(30.0, "clip.mp4", &CopyOutcome::Copied),
            "30 s → clip.mp4 · copiado al portapapeles"
        );
        let failed =
            saved_description(30.0, "clip.mp4", &CopyOutcome::Failed("sin permiso".into()));
        assert!(failed.starts_with("30 s → clip.mp4"), "{failed}");
        assert!(
            failed.contains("no se pudo copiar: sin permiso"),
            "{failed}"
        );
    }

    #[test]
    fn hotkey_save_clip_notifies_success() {
        let mock = Mock::default();
        dispatch(Command::SaveClip(None), &mock);
        assert_eq!(mock.calls.borrow().as_slice(), ["save"]);
        let notes = mock.notifications.borrow();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, "Clip guardado");
    }

    #[test]
    fn a_shortcut_with_its_own_length_asks_for_that_length() {
        let mock = Mock::default();
        dispatch(Command::SaveClip(Some(15)), &mock);
        dispatch(Command::SaveClip(Some(120)), &mock);
        assert_eq!(mock.calls.borrow().as_slice(), ["save 15", "save 120"]);
        assert_eq!(mock.notifications.borrow().len(), 2);
    }

    #[test]
    fn the_screenshot_command_is_in_the_tray_menu_and_announces_the_result() {
        assert_eq!(
            Command::from_menu_id("screenshot"),
            Some(Command::Screenshot)
        );
        let mock = Mock::default();
        dispatch(Command::Screenshot, &mock);
        assert_eq!(mock.calls.borrow().as_slice(), ["screenshot"]);
        let notes = mock.notifications.borrow();
        assert_eq!(notes[0].0, "Captura guardada");
        assert!(notes[0].1.contains("captura.png"));
    }

    #[test]
    fn a_screenshot_that_fails_says_why() {
        let mock = Mock {
            screenshot_result: Some(Err("permiso denegado".into())),
            ..Default::default()
        };
        dispatch(Command::Screenshot, &mock);
        let notes = mock.notifications.borrow();
        assert_eq!(notes[0].0, "No se pudo tomar la captura");
        assert!(notes[0].1.contains("permiso denegado"));
    }

    #[test]
    fn save_clip_failure_notifies_error() {
        let mock = Mock {
            save_result: Some(Err("buffer vacío".into())),
            ..Default::default()
        };
        dispatch(Command::SaveClip(None), &mock);
        let notes = mock.notifications.borrow();
        assert_eq!(notes[0].0, "No se pudo guardar el clip");
        assert!(notes[0].1.contains("buffer vacío"));
    }

    #[test]
    fn toggle_starts_then_stops() {
        let mock = Mock::default();
        dispatch(Command::ToggleBuffer, &mock);
        dispatch(Command::ToggleBuffer, &mock);
        assert_eq!(mock.calls.borrow().as_slice(), ["start", "stop"]);
        let notes = mock.notifications.borrow();
        assert!(notes[0].1.contains("iniciado"));
        assert!(notes[1].1.contains("detenido"));
    }

    #[test]
    fn open_and_quit_pass_through_without_notifying() {
        let mock = Mock::default();
        dispatch(Command::OpenClipsFolder, &mock);
        dispatch(Command::Quit, &mock);
        assert_eq!(mock.calls.borrow().as_slice(), ["open", "quit"]);
        assert!(mock.notifications.borrow().is_empty());
    }

    #[test]
    fn mic_mute_hotkey_announces_each_outcome() {
        for (result, expected) in [
            (Ok(MuteOutcome::Muted), "silenciado"),
            (Ok(MuteOutcome::Unmuted), "activado"),
            (Ok(MuteOutcome::MicOff), "desactivado"),
        ] {
            let mock = Mock {
                mute_result: Some(result),
                ..Default::default()
            };
            dispatch(Command::ToggleMicMute, &mock);
            assert_eq!(mock.calls.borrow().as_slice(), ["mute"]);
            let notes = mock.notifications.borrow();
            assert_eq!(notes.len(), 1);
            assert!(notes[0].1.contains(expected), "{:?}", notes[0]);
        }
    }

    #[test]
    fn mic_mute_failure_is_reported_not_swallowed() {
        let mock = Mock {
            mute_result: Some(Err("disco lleno".into())),
            ..Default::default()
        };
        dispatch(Command::ToggleMicMute, &mock);
        let notes = mock.notifications.borrow();
        assert_eq!(notes[0].0, "No se pudo cambiar el micrófono");
        assert!(notes[0].1.contains("disco lleno"));
    }
}
