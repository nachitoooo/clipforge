mod dispatch;
mod recorder;

use dispatch::{dispatch, Actions, Command};
use recorder::{Recorder, DEFAULT_CLIP_SECONDS};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, RunEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// Global hotkey for "save clip": Cmd+Shift+S (macOS) / Ctrl+Shift+S.
fn save_clip_shortcut() -> Shortcut {
    #[cfg(target_os = "macos")]
    let modifiers = Modifiers::SUPER | Modifiers::SHIFT;
    #[cfg(not(target_os = "macos"))]
    let modifiers = Modifiers::CONTROL | Modifiers::SHIFT;
    Shortcut::new(Some(modifiers), Code::KeyS)
}

/// The real [`Actions`] implementation: routes dispatcher decisions to the
/// recorder, OS notifications, and the opener plugin.
struct AppActions {
    app: AppHandle,
}

impl AppActions {
    fn recorder(&self) -> tauri::State<'_, Recorder> {
        self.app.state::<Recorder>()
    }
}

impl Actions for AppActions {
    fn buffer_running(&self) -> bool {
        self.recorder().is_running()
    }

    fn start_buffer(&self) -> Result<(), String> {
        self.recorder().start()
    }

    fn stop_buffer(&self) {
        self.recorder().stop();
    }

    fn save_clip(&self) -> Result<String, String> {
        let saved = self.recorder().save_clip(DEFAULT_CLIP_SECONDS)?;
        Ok(format!(
            "{:.0} s → {}",
            saved.duration.as_secs_f64(),
            saved
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ))
    }

    fn open_clips_folder(&self) {
        let dir = self.recorder().clips_dir();
        std::fs::create_dir_all(&dir).ok();
        if let Err(e) = self
            .app
            .opener()
            .open_path(dir.to_string_lossy(), None::<&str>)
        {
            eprintln!("open clips folder: {e}");
        }
    }

    fn quit(&self) {
        self.app.exit(0);
    }

    fn notify(&self, title: &str, body: &str) {
        if let Err(e) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            eprintln!("notification: {e}");
        }
    }
}

fn run_command(app: &AppHandle, command: Command) {
    dispatch(command, &AppActions { app: app.clone() });
}

// ---- Commands for the UI (Milestone 3 sidebar consumes these) ----

#[tauri::command]
fn buffer_running(recorder: tauri::State<'_, Recorder>) -> bool {
    recorder.is_running()
}

#[tauri::command]
fn toggle_buffer(app: AppHandle) {
    run_command(&app, Command::ToggleBuffer);
}

#[tauri::command]
fn save_clip(app: AppHandle) {
    run_command(&app, Command::SaveClip);
}

#[tauri::command]
fn open_clips_folder(app: AppHandle) {
    run_command(&app, Command::OpenClipsFolder);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed && *shortcut == save_clip_shortcut()
                    {
                        run_command(app.app_handle(), Command::SaveClip);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            buffer_running,
            toggle_buffer,
            save_clip,
            open_clips_folder
        ])
        .setup(|app| {
            // Clips folder: ~/Movies/ClipForge (or the OS video dir).
            let clips_dir = app
                .path()
                .video_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("ClipForge");
            app.manage(Recorder::new(clips_dir));

            // Tray menu (menu-bar icon on macOS).
            let toggle =
                MenuItemBuilder::with_id("toggle_buffer", "Iniciar/Detener buffer").build(app)?;
            let save = MenuItemBuilder::with_id("save_clip", "Guardar clip (últimos 30 s)")
                .accelerator("CmdOrCtrl+Shift+S")
                .build(app)?;
            let open =
                MenuItemBuilder::with_id("open_folder", "Abrir carpeta de clips").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Salir de ClipForge").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&toggle)
                .item(&save)
                .separator()
                .item(&open)
                .separator()
                .item(&quit)
                .build()?;
            TrayIconBuilder::with_id("clipforge-tray")
                .icon(app.default_window_icon().expect("app icon").clone())
                .tooltip("ClipForge")
                .menu(&menu)
                .on_menu_event(|app, event| {
                    if let Some(command) = Command::from_menu_id(event.id().as_ref()) {
                        run_command(app, command);
                    }
                })
                .build(app)?;

            // Global hotkey.
            app.global_shortcut().register(save_clip_shortcut())?;

            // Background mode: start the replay buffer right away; a denied
            // screen-recording permission surfaces as a notification, not a
            // crash.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let actions = AppActions { app: handle };
                match actions.start_buffer() {
                    Ok(()) => actions.notify("ClipForge", "Buffer de replay iniciado"),
                    Err(e) => actions.notify("ClipForge — no se pudo iniciar la captura", &e),
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // Keep running in the background when the window is closed; only
            // an explicit exit (tray → Salir) terminates the app.
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
