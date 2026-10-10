mod dispatch;
mod library;
mod mic;
mod recorder;
mod settings;

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dispatch::{dispatch, Actions, Command, MuteOutcome};
use mic::{MicDevice, MicStatus};
use recorder::{BufferStatus, Recorder, DEFAULT_CLIP_SECONDS};
use settings::{Settings, SettingsStore};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent};
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

/// Shortcuts owned by other features: a user-chosen one must not collide.
fn reserved_shortcuts() -> Vec<Shortcut> {
    vec![save_clip_shortcut()]
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

    fn emit_buffer_status(&self) {
        self.app
            .emit("buffer-changed", self.recorder().status())
            .ok();
    }

    fn settings(&self) -> tauri::State<'_, Arc<SettingsStore>> {
        self.app.state::<Arc<SettingsStore>>()
    }
}

/// Apply the microphone settings off the calling thread (opening a device can
/// block on the permission prompt) and tell the UI how it went.
fn apply_microphone_in_background(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let status = app.state::<Recorder>().apply_microphone();
        app.emit("mic-status", &status).ok();
    });
}

impl Actions for AppActions {
    fn buffer_running(&self) -> bool {
        self.recorder().is_running()
    }

    fn start_buffer(&self) -> Result<(), String> {
        let result = self.recorder().start();
        // Remember a failure so a UI that loads later can still explain it.
        self.recorder().record_start_result(&result);
        self.emit_buffer_status();
        if result.is_ok() {
            // The microphone shares the video's clock, so it can only join
            // once the capture is running.
            apply_microphone_in_background(&self.app);
        }
        result
    }

    fn stop_buffer(&self) {
        self.recorder().stop();
        self.emit_buffer_status();
        self.app
            .emit("mic-status", self.recorder().mic_status())
            .ok();
    }

    fn save_clip(&self) -> Result<String, String> {
        let saved = self.recorder().save_clip(DEFAULT_CLIP_SECONDS)?;
        let name = saved
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // The gallery refreshes itself on this event (F-3.1 "live update").
        self.app.emit("clip-saved", &name).ok();
        Ok(format!("{:.0} s → {name}", saved.duration.as_secs_f64()))
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

    fn toggle_mic_mute(&self) -> Result<MuteOutcome, String> {
        let store = self.settings();
        let mut next = store.get();
        if !next.microphone.enabled {
            return Ok(MuteOutcome::MicOff);
        }
        next.microphone.muted = !next.microphone.muted;
        let saved = store.replace(next.validated(&reserved_shortcuts())?)?;
        self.app.emit("settings-changed", &saved).ok();
        // Muting only flips an atomic: apply it right here, no device work.
        let status = self.recorder().apply_microphone();
        self.app.emit("mic-status", &status).ok();
        Ok(if saved.microphone.muted {
            MuteOutcome::Muted
        } else {
            MuteOutcome::Unmuted
        })
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
fn buffer_status(recorder: tauri::State<'_, Recorder>) -> BufferStatus {
    recorder.status()
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

#[tauri::command]
fn list_clips(recorder: tauri::State<'_, Recorder>) -> Result<Vec<library::ClipInfo>, String> {
    library::list_clips(&recorder.clips_dir())
}

#[tauri::command]
fn rename_clip(
    recorder: tauri::State<'_, Recorder>,
    name: String,
    new_name: String,
) -> Result<String, String> {
    library::rename_clip(&recorder.clips_dir(), &name, &new_name)
}

#[tauri::command]
fn delete_clip(recorder: tauri::State<'_, Recorder>, name: String) -> Result<(), String> {
    library::delete_clip(&recorder.clips_dir(), &name)
}

#[tauri::command]
fn reveal_clip(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    name: String,
) -> Result<(), String> {
    let path = library::clip_path(&recorder.clips_dir(), &name)?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| format!("no se pudo abrir la carpeta: {e}"))
}

#[tauri::command]
fn get_settings(store: tauri::State<'_, Arc<SettingsStore>>) -> Settings {
    store.get()
}

/// Move the mute hotkey. The new one is registered first, so if the OS or
/// another app refuses it nothing has changed yet.
fn swap_mute_shortcut(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    let new_shortcut = Shortcut::from_str(new).map_err(|_| format!("atajo no válido: «{new}»"))?;
    shortcuts.register(new_shortcut).map_err(|e| {
        format!("no se pudo registrar el atajo «{new}» (¿lo usa otra aplicación?): {e}")
    })?;
    if let Ok(old_shortcut) = Shortcut::from_str(old) {
        shortcuts.unregister(old_shortcut).ok();
    }
    Ok(())
}

#[tauri::command]
fn update_settings(
    app: AppHandle,
    store: tauri::State<'_, Arc<SettingsStore>>,
    settings: Settings,
) -> Result<Settings, String> {
    let validated = settings.validated(&reserved_shortcuts())?;
    let old_shortcut = store.get().microphone.mute_shortcut;
    let new_shortcut = validated.microphone.mute_shortcut.clone();
    if old_shortcut != new_shortcut {
        swap_mute_shortcut(&app, &old_shortcut, &new_shortcut)?;
    }
    let saved = match store.replace(validated) {
        Ok(saved) => saved,
        Err(e) => {
            if old_shortcut != new_shortcut {
                swap_mute_shortcut(&app, &new_shortcut, &old_shortcut).ok();
            }
            return Err(e);
        }
    };
    app.emit("settings-changed", &saved).ok();
    app.state::<Recorder>().apply_live_controls();
    apply_microphone_in_background(&app);
    Ok(saved)
}

#[tauri::command]
async fn list_microphones(app: AppHandle) -> Result<Vec<MicDevice>, String> {
    // Enumerating devices talks to the OS audio service: keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || app.state::<Recorder>().list_microphones())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn microphone_status(recorder: tauri::State<'_, Recorder>) -> MicStatus {
    recorder.mic_status()
}

#[tauri::command]
fn retry_microphone(app: AppHandle) {
    apply_microphone_in_background(&app);
}

/// Whether the Settings screen is open: the level meter only streams then.
struct MeterActive(AtomicBool);

#[tauri::command]
fn set_meter_active(active: bool, meter: tauri::State<'_, MeterActive>) {
    meter.0.store(active, Ordering::Relaxed);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if *shortcut == save_clip_shortcut() {
                        run_command(app.app_handle(), Command::SaveClip);
                        return;
                    }
                    let mute = app
                        .state::<Arc<SettingsStore>>()
                        .get()
                        .microphone
                        .mute_shortcut;
                    if Shortcut::from_str(&mute).is_ok_and(|m| m == *shortcut) {
                        run_command(app.app_handle(), Command::ToggleMicMute);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            buffer_status,
            toggle_buffer,
            save_clip,
            open_clips_folder,
            list_clips,
            rename_clip,
            delete_clip,
            reveal_clip,
            get_settings,
            update_settings,
            list_microphones,
            microphone_status,
            retry_microphone,
            set_meter_active
        ])
        .setup(|app| {
            // Clips folder: ~/Movies/ClipForge (or the OS video dir).
            let clips_dir = app
                .path()
                .video_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("ClipForge");
            let settings = Arc::new(SettingsStore::load(
                app.path().app_config_dir()?.join("settings.json"),
            ));
            app.manage(Arc::clone(&settings));
            app.manage(Recorder::new(clips_dir, Arc::clone(&settings)));
            app.manage(MeterActive(AtomicBool::new(false)));

            // Tray menu (menu-bar icon on macOS).
            let toggle =
                MenuItemBuilder::with_id("toggle_buffer", "Iniciar/Detener buffer").build(app)?;
            let save = MenuItemBuilder::with_id("save_clip", "Guardar clip (últimos 30 s)")
                .accelerator("CmdOrCtrl+Shift+S")
                .build(app)?;
            let mute = MenuItemBuilder::with_id("toggle_mic_mute", "Silenciar/activar micrófono")
                .build(app)?;
            let open =
                MenuItemBuilder::with_id("open_folder", "Abrir carpeta de clips").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Salir de ClipForge").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&toggle)
                .item(&save)
                .item(&mute)
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

            // Global hotkeys. The save shortcut is essential; the mute one is
            // a convenience, so failing to claim it must not stop the app.
            app.global_shortcut().register(save_clip_shortcut())?;
            match Shortcut::from_str(&settings.get().microphone.mute_shortcut) {
                Ok(mute) => {
                    if let Err(e) = app.global_shortcut().register(mute) {
                        eprintln!("clipforge: mute shortcut unavailable: {e}");
                    }
                }
                Err(e) => eprintln!("clipforge: mute shortcut invalid: {e}"),
            }

            // Microphone watcher: streams the input level to the Settings
            // meter while it is open, and tells the UI whenever the
            // microphone's health changes (device unplugged, no signal...).
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut last: Option<MicStatus> = None;
                let mut tick = 0u32;
                loop {
                    std::thread::sleep(Duration::from_millis(50));
                    let recorder = handle.state::<Recorder>();
                    let peak = recorder.take_mic_peak();
                    if handle.state::<MeterActive>().0.load(Ordering::Relaxed) {
                        handle.emit("mic-level", peak).ok();
                    }
                    tick += 1;
                    if tick.is_multiple_of(20) {
                        let status = recorder.mic_status();
                        if last.as_ref() != Some(&status) {
                            handle.emit("mic-status", &status).ok();
                            last = Some(status);
                        }
                    }
                }
            });

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
