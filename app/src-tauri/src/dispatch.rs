//! Hotkey/tray → command dispatch, kept free of Tauri types so it is unit
//! testable with mocked events (F-2.3 acceptance).

/// Everything a hotkey press or tray click can trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ToggleBuffer,
    SaveClip,
    OpenClipsFolder,
    Quit,
}

impl Command {
    /// Tray menu item ids (see `tray` setup in `lib.rs`).
    pub fn from_menu_id(id: &str) -> Option<Self> {
        match id {
            "toggle_buffer" => Some(Self::ToggleBuffer),
            "save_clip" => Some(Self::SaveClip),
            "open_folder" => Some(Self::OpenClipsFolder),
            "quit" => Some(Self::Quit),
            _ => None,
        }
    }
}

/// Side effects the dispatcher can request — implemented by the real app,
/// mocked in tests.
pub trait Actions {
    fn buffer_running(&self) -> bool;
    fn start_buffer(&self) -> Result<(), String>;
    fn stop_buffer(&self);
    /// Returns a human-readable description of the saved clip.
    fn save_clip(&self) -> Result<String, String>;
    fn open_clips_folder(&self);
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
        Command::SaveClip => match actions.save_clip() {
            Ok(description) => actions.notify("Clip guardado", &description),
            Err(e) => actions.notify("No se pudo guardar el clip", &e),
        },
        Command::OpenClipsFolder => actions.open_clips_folder(),
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
        calls: RefCell<Vec<String>>,
        notifications: RefCell<Vec<(String, String)>>,
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
        fn save_clip(&self) -> Result<String, String> {
            self.calls.borrow_mut().push("save".into());
            self.save_result.clone().unwrap_or(Ok("clip 30 s".into()))
        }
        fn open_clips_folder(&self) {
            self.calls.borrow_mut().push("open".into());
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
        assert_eq!(Command::from_menu_id("save_clip"), Some(Command::SaveClip));
        assert_eq!(
            Command::from_menu_id("open_folder"),
            Some(Command::OpenClipsFolder)
        );
        assert_eq!(Command::from_menu_id("quit"), Some(Command::Quit));
        assert_eq!(Command::from_menu_id("unknown"), None);
    }

    #[test]
    fn hotkey_save_clip_notifies_success() {
        let mock = Mock::default();
        dispatch(Command::SaveClip, &mock);
        assert_eq!(mock.calls.borrow().as_slice(), ["save"]);
        let notes = mock.notifications.borrow();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, "Clip guardado");
    }

    #[test]
    fn save_clip_failure_notifies_error() {
        let mock = Mock {
            save_result: Some(Err("buffer vacío".into())),
            ..Default::default()
        };
        dispatch(Command::SaveClip, &mock);
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
}
