mod dispatch;
mod editor;
mod library;
mod mic;
mod recorder;
mod settings;
mod shortcuts;

/// Helpers shared by the unit tests.
#[cfg(test)]
mod testutil {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A name that is unique per call, even for two tests started in the same
    /// clock tick (the clock alone is not fine-grained enough: tests sharing a
    /// scratch folder raced on its files).
    pub fn unique() -> String {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!(
            "{}-{nanos}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        )
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dispatch::{dispatch, Actions, Command, MuteOutcome};
use mic::{MicDevice, MicStatus};
use recorder::{BufferStatus, Recorder, DEFAULT_CLIP_SECONDS};
use settings::{Settings, SettingsStore};
use shortcuts::{Registry, ShortcutAction};
use tauri::menu::{MenuBuilder, MenuItem, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// The OS-level shortcut registrations, through the Tauri plugin.
struct AppRegistry<'a>(&'a AppHandle);

impl Registry for AppRegistry<'_> {
    fn register(&self, shortcut: Shortcut) -> Result<(), String> {
        self.0
            .global_shortcut()
            .register(shortcut)
            .map_err(|e| e.to_string())
    }

    fn unregister(&self, shortcut: Shortcut) {
        self.0.global_shortcut().unregister(shortcut).ok();
    }
}

/// The tray menu entries whose label depends on a setting.
struct TrayItems {
    save: MenuItem<Wry>,
}

/// Show the current save shortcut next to the tray's "save clip" entry.
fn show_save_shortcut_in_tray(app: &AppHandle, accelerator: &str) {
    if let Err(e) = app
        .state::<TrayItems>()
        .save
        .set_accelerator(Some(accelerator))
    {
        eprintln!("clipforge: tray shortcut hint «{accelerator}»: {e}");
    }
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
        let saved = store.replace(next.validated()?)?;
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

/// Run blocking clip work (reading and writing files) off the UI thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn describe_clip(app: AppHandle, name: String) -> Result<editor::EditorClip, String> {
    let dir = app.state::<Recorder>().clips_dir();
    blocking(move || editor::describe(&dir, &name)).await
}

#[tauri::command]
async fn clip_waveform(
    app: AppHandle,
    name: String,
    track: usize,
) -> Result<editor::Waveform, String> {
    let dir = app.state::<Recorder>().clips_dir();
    blocking(move || editor::waveform(&dir, &name, track)).await
}

#[tauri::command]
async fn trim_clip(
    app: AppHandle,
    request: editor::TrimRequest,
) -> Result<editor::TrimResult, String> {
    let dir = app.state::<Recorder>().clips_dir();
    let result = blocking(move || editor::trim_clip(&dir, &request)).await?;
    // The gallery refreshes itself and highlights the new clip.
    app.emit("clip-saved", &result.name).ok();
    Ok(result)
}

#[tauri::command]
fn get_settings(store: tauri::State<'_, Arc<SettingsStore>>) -> Settings {
    store.get()
}

#[tauri::command]
fn update_settings(
    app: AppHandle,
    store: tauri::State<'_, Arc<SettingsStore>>,
    settings: Settings,
) -> Result<Settings, String> {
    let validated = settings.validated()?;
    let previous = store.get();
    // Move the global shortcuts first: if the OS or another app refuses one,
    // nothing has changed yet.
    shortcuts::switch(&AppRegistry(&app), &previous, &validated)?;
    let saved = match store.replace(validated.clone()) {
        Ok(saved) => saved,
        Err(e) => {
            shortcuts::switch(&AppRegistry(&app), &validated, &previous).ok();
            return Err(e);
        }
    };
    if saved.shortcuts.save_clip != previous.shortcuts.save_clip {
        show_save_shortcut_in_tray(&app, &saved.shortcuts.save_clip);
    }
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
                    let settings = app.state::<Arc<SettingsStore>>().get();
                    match shortcuts::action_for(&settings, shortcut) {
                        Some(ShortcutAction::SaveClip) => {
                            run_command(app.app_handle(), Command::SaveClip)
                        }
                        Some(ShortcutAction::ToggleMicMute) => {
                            run_command(app.app_handle(), Command::ToggleMicMute)
                        }
                        None => {}
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
            describe_clip,
            clip_waveform,
            trim_clip,
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
            let save =
                MenuItemBuilder::with_id("save_clip", "Guardar clip (últimos 30 s)").build(app)?;
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
            app.manage(TrayItems { save: save.clone() });
            show_save_shortcut_in_tray(app.handle(), &settings.get().shortcuts.save_clip);
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

            // Global hotkeys. One that is taken by another app must not stop
            // ClipForge from starting: tell the user (below) and carry on.
            let shortcut_problems =
                shortcuts::register_all(&AppRegistry(app.handle()), &settings.get());

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
                for problem in shortcut_problems {
                    actions.notify("ClipForge — atajo no disponible", &problem);
                }
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
