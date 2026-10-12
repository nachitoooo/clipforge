mod clipmeta;
mod dispatch;
mod editor;
mod folders;
mod library;
mod memory;
mod mic;
mod persist;
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

    /// A scratch directory that is removed on drop.
    pub struct TempDir(pub std::path::PathBuf);

    impl TempDir {
        pub fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("clipforge-{label}-{}", unique()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clipmeta::ClipMetaStore;
use dispatch::{dispatch, Actions, Command, MuteOutcome};
use folders::{FolderDraft, FolderInfo, FolderStore};
use mic::{MicDevice, MicStatus};
use recorder::{BufferStatus, Recorder};
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
    screenshot: MenuItem<Wry>,
}

/// Show the length and the shortcut of the main save in the tray's "save clip"
/// entry (the extra shortcuts have no entry of their own), and the screenshot
/// shortcut, if there is one, next to "take a screenshot".
fn show_shortcuts_in_tray(app: &AppHandle, shortcuts: &settings::ShortcutSettings) {
    let tray = app.state::<TrayItems>();
    let label = format!(
        "Guardar clip (últimos {})",
        settings::clip_length_label(shortcuts.clip_seconds)
    );
    if let Err(e) = tray.save.set_text(label) {
        eprintln!("clipforge: tray label: {e}");
    }
    if let Err(e) = tray.save.set_accelerator(Some(&shortcuts.save_clip)) {
        eprintln!(
            "clipforge: tray shortcut hint «{}»: {e}",
            shortcuts.save_clip
        );
    }
    let screenshot = (!shortcuts.screenshot.is_empty()).then_some(shortcuts.screenshot.as_str());
    if let Err(e) = tray.screenshot.set_accelerator(screenshot) {
        eprintln!("clipforge: tray screenshot hint: {e}");
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

    /// A clip or a screenshot was just saved: note its game (what was in front
    /// during the last `lookback_seconds`; ClipForge itself is ignored, so saving
    /// from the tray or the window still names the game), file it in the folder
    /// of that game when automatic folders are on, and tell the views.
    fn register_new_file(&self, name: &str, lookback_seconds: u64) {
        let game = self.recorder().detected_game(lookback_seconds);
        let meta = self.app.state::<ClipMetaStore>();
        keep_index(meta.set_detected_game(name, game.as_deref()));
        // With automatic folders on, the file goes into the folder of its game
        // (the spelling the index kept, so "VALORANT" joins "Valorant").
        if self.settings().get().library.auto_folders {
            if let Some(game) = meta.game_of(name) {
                keep_index(self.app.state::<FolderStore>().file_under_game(&game, name));
            }
        }
        // The gallery refreshes itself on this event (F-3.1 "live update").
        self.app.emit("clip-saved", name).ok();
    }
}

/// The name of a file inside the clips folder.
fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A screenshot is tagged with the application in front over this many seconds.
const SCREENSHOT_GAME_LOOKBACK_SECONDS: u64 = 5;

/// Start the video capture over, off the calling thread, so a change that is
/// fixed at start (the memory mode) takes effect. The buffer is emptied. Does
/// nothing when the buffer was stopped on purpose: it will start with the new
/// setting next time.
fn restart_capture_in_background(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let actions = AppActions { app };
        if !actions.buffer_running() {
            return;
        }
        actions.stop_buffer();
        if let Err(e) = actions.start_buffer() {
            actions.notify("ClipForge — no se pudo reiniciar la captura", &e);
        }
    });
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

    fn save_clip(&self, seconds: Option<u64>) -> Result<String, String> {
        let seconds = seconds.unwrap_or_else(|| self.settings().get().shortcuts.clip_seconds);
        let saved = self.recorder().save_clip(seconds)?;
        let name = file_name_of(&saved.path);
        // The game is whatever held the screen for most of the clip.
        self.register_new_file(&name, seconds);
        Ok(format!("{:.0} s → {name}", saved.duration.as_secs_f64()))
    }

    fn take_screenshot(&self) -> Result<String, String> {
        let path = clipforge_core::screenshot::next_path(&self.recorder().clips_dir());
        let shot = clipforge_core::screenshot::capture_png(&path).map_err(|e| e.to_string())?;
        let name = file_name_of(&path);
        // A picture is of what is in front *now*: the last few seconds.
        self.register_new_file(&name, SCREENSHOT_GAME_LOOKBACK_SECONDS);
        Ok(format!("{}×{} → {name}", shot.width, shot.height))
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
    run_command(&app, Command::SaveClip(None));
}

#[tauri::command]
fn take_screenshot(app: AppHandle) {
    run_command(&app, Command::Screenshot);
}

#[tauri::command]
fn open_clips_folder(app: AppHandle) {
    run_command(&app, Command::OpenClipsFolder);
}

#[tauri::command]
fn list_clips(
    recorder: tauri::State<'_, Recorder>,
    folders: tauri::State<'_, FolderStore>,
    meta: tauri::State<'_, ClipMetaStore>,
) -> Result<Vec<library::ClipInfo>, String> {
    let mut clips = library::list_clips(&recorder.clips_dir())?;
    folders.annotate(&mut clips);
    meta.annotate(&mut clips);
    Ok(clips)
}

/// Tell every view that clips or folders changed, so each one reloads.
fn library_changed(app: &AppHandle) {
    app.emit("library-changed", ()).ok();
}

/// The indexes follow files the app renames, deletes or copies. If one can't be
/// written the file operation has already happened, so say so and carry on.
fn keep_index(result: Result<(), String>) {
    if let Err(e) = result {
        eprintln!("clipforge: library index: {e}");
    }
}

/// `old` is now `new`: its folders and what is known about it follow.
fn clip_renamed(app: &AppHandle, old: &str, new: &str) {
    keep_index(app.state::<FolderStore>().rename_clip(old, new));
    keep_index(app.state::<ClipMetaStore>().rename_clip(old, new));
    library_changed(app);
}

/// These clips are gone from the disk.
fn clips_deleted(app: &AppHandle, names: &[String]) {
    keep_index(app.state::<FolderStore>().forget_clips(names));
    keep_index(app.state::<ClipMetaStore>().forget_clips(names));
    library_changed(app);
}

/// `new` is a copy of `source` (a trimmed clip): same folders, same game.
fn clip_copied(app: &AppHandle, source: &str, new: &str) {
    keep_index(app.state::<FolderStore>().inherit(source, new));
    keep_index(app.state::<ClipMetaStore>().inherit(source, new));
}

#[tauri::command]
fn rename_clip(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    name: String,
    new_name: String,
) -> Result<String, String> {
    let new = library::rename_clip(&recorder.clips_dir(), &name, &new_name)?;
    if new != name {
        clip_renamed(&app, &name, &new);
    }
    Ok(new)
}

#[tauri::command]
fn delete_clip(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    name: String,
) -> Result<(), String> {
    library::delete_clip(&recorder.clips_dir(), &name)?;
    clips_deleted(&app, &[name]);
    Ok(())
}

// ---- Library folders (folders.rs) ----

/// `id`'s folder as the UI shows it (with the counts of the clips on disk).
fn folder_info(recorder: &Recorder, folders: &FolderStore, id: &str) -> Result<FolderInfo, String> {
    let clips = library::list_clips(&recorder.clips_dir())?;
    folders
        .list(&clips)
        .into_iter()
        .find(|f| f.id == id)
        .ok_or_else(|| "la carpeta ya no existe".to_string())
}

/// Names the webview sent, checked to be clips that exist in the clips folder.
fn existing_clips(recorder: &Recorder, names: &[String]) -> Result<Vec<String>, String> {
    let dir = recorder.clips_dir();
    for name in names {
        if !library::media_path(&dir, name)?.is_file() {
            return Err(format!("el clip «{name}» ya no existe"));
        }
    }
    Ok(names.to_vec())
}

#[tauri::command]
fn list_folders(
    recorder: tauri::State<'_, Recorder>,
    folders: tauri::State<'_, FolderStore>,
) -> Result<Vec<FolderInfo>, String> {
    let clips = library::list_clips(&recorder.clips_dir())?;
    Ok(folders.list(&clips))
}

#[tauri::command]
fn create_folder(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    folders: tauri::State<'_, FolderStore>,
    draft: FolderDraft,
) -> Result<FolderInfo, String> {
    let id = folders.create(&draft)?;
    library_changed(&app);
    folder_info(&recorder, &folders, &id)
}

#[tauri::command]
fn update_folder(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    folders: tauri::State<'_, FolderStore>,
    id: String,
    draft: FolderDraft,
) -> Result<FolderInfo, String> {
    folders.update(&id, &draft)?;
    library_changed(&app);
    folder_info(&recorder, &folders, &id)
}

/// Delete a folder; with `delete_clips` also the clips only it holds. Returns
/// how many clip files were deleted.
#[tauri::command]
fn delete_folder(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    folders: tauri::State<'_, FolderStore>,
    id: String,
    delete_clips: bool,
) -> Result<usize, String> {
    let result = library::delete_folder(&recorder.clips_dir(), &folders, &id, delete_clips);
    match result {
        Ok(deleted) => {
            clips_deleted(&app, &deleted);
            Ok(deleted.len())
        }
        Err(e) => {
            // Even a failed attempt may have deleted some clips.
            library_changed(&app);
            Err(e)
        }
    }
}

/// Correct the game of a clip (`None` clears it). With `all`, every clip of the
/// game it had so far changes too. Returns how many clips changed.
#[tauri::command]
fn set_clip_game(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    meta: tauri::State<'_, ClipMetaStore>,
    name: String,
    game: Option<String>,
    all: bool,
) -> Result<usize, String> {
    let names = existing_clips(&recorder, &[name])?;
    let changed = match game.as_deref() {
        Some(game) if all => meta.set_game_everywhere(&names[0], game)?,
        game => {
            meta.set_game(&names[0], game)?;
            1
        }
    };
    library_changed(&app);
    Ok(changed)
}

#[tauri::command]
fn set_clip_folders(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    folders: tauri::State<'_, FolderStore>,
    name: String,
    folder_ids: Vec<String>,
) -> Result<(), String> {
    let names = existing_clips(&recorder, &[name])?;
    folders.set_clip_folders(&names[0], &folder_ids)?;
    library_changed(&app);
    Ok(())
}

/// File clips in a folder; returns how many were not already in it.
#[tauri::command]
fn add_to_folder(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    folders: tauri::State<'_, FolderStore>,
    id: String,
    names: Vec<String>,
) -> Result<usize, String> {
    let names = existing_clips(&recorder, &names)?;
    let added = folders.add_to_folder(&id, &names)?;
    library_changed(&app);
    Ok(added)
}

#[tauri::command]
fn remove_from_folder(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    folders: tauri::State<'_, FolderStore>,
    id: String,
    names: Vec<String>,
) -> Result<usize, String> {
    // The files may be gone already; only the names have to be plain.
    let dir = recorder.clips_dir();
    for name in &names {
        library::media_path(&dir, name)?;
    }
    let removed = folders.remove_from_folder(&id, &names)?;
    library_changed(&app);
    Ok(removed)
}

#[tauri::command]
fn reveal_clip(
    app: AppHandle,
    recorder: tauri::State<'_, Recorder>,
    name: String,
) -> Result<(), String> {
    let path = library::media_path(&recorder.clips_dir(), &name)?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| format!("no se pudo abrir la carpeta: {e}"))
}

/// Put the clip's file on the system clipboard, so it can be pasted in a chat,
/// a mail or a folder as a file.
#[tauri::command]
async fn copy_clip(app: AppHandle, name: String) -> Result<(), String> {
    let path = library::media_path(&app.state::<Recorder>().clips_dir(), &name)?;
    blocking(move || clipforge_core::clipboard::copy_file(&path)).await
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
    let source = request.name.clone();
    let result = blocking(move || editor::trim_clip(&dir, &request)).await?;
    // The copy sits in the same folders, and is of the same game, as the clip
    // it came from.
    clip_copied(&app, &source, &result.name);
    // The views refresh themselves and highlight the new clip.
    app.emit("clip-saved", &result.name).ok();
    Ok(result)
}

/// What each memory mode costs, for the settings screen.
#[tauri::command]
fn memory_plans() -> memory::Plans {
    memory::plans()
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
    if saved.shortcuts != previous.shortcuts {
        show_shortcuts_in_tray(&app, &saved.shortcuts);
    }
    app.emit("settings-changed", &saved).ok();
    app.state::<Recorder>().apply_live_controls();
    if Settings::needs_capture_restart(&previous, &saved) {
        restart_capture_in_background(&app);
    } else {
        apply_microphone_in_background(&app);
    }
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
                        Some(ShortcutAction::SaveClip(seconds)) => {
                            run_command(app.app_handle(), Command::SaveClip(Some(seconds)))
                        }
                        Some(ShortcutAction::ToggleMicMute) => {
                            run_command(app.app_handle(), Command::ToggleMicMute)
                        }
                        Some(ShortcutAction::Screenshot) => {
                            run_command(app.app_handle(), Command::Screenshot)
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
            take_screenshot,
            open_clips_folder,
            list_clips,
            rename_clip,
            delete_clip,
            reveal_clip,
            copy_clip,
            list_folders,
            create_folder,
            update_folder,
            delete_folder,
            set_clip_game,
            set_clip_folders,
            add_to_folder,
            remove_from_folder,
            describe_clip,
            clip_waveform,
            trim_clip,
            get_settings,
            memory_plans,
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
            app.manage(FolderStore::load(
                app.path().app_config_dir()?.join("library.json"),
            ));
            app.manage(ClipMetaStore::load(
                app.path().app_config_dir()?.join("clipmeta.json"),
            ));
            app.manage(Recorder::new(clips_dir, Arc::clone(&settings)));
            app.manage(MeterActive(AtomicBool::new(false)));

            // Tray menu (menu-bar icon on macOS).
            let toggle =
                MenuItemBuilder::with_id("toggle_buffer", "Iniciar/Detener buffer").build(app)?;
            let save = MenuItemBuilder::with_id("save_clip", "Guardar clip").build(app)?; // label set below
            let screenshot =
                MenuItemBuilder::with_id("screenshot", "Tomar captura de pantalla").build(app)?;
            let mute = MenuItemBuilder::with_id("toggle_mic_mute", "Silenciar/activar micrófono")
                .build(app)?;
            let open =
                MenuItemBuilder::with_id("open_folder", "Abrir carpeta de clips").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Salir de ClipForge").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&toggle)
                .item(&save)
                .item(&screenshot)
                .item(&mute)
                .separator()
                .item(&open)
                .separator()
                .item(&quit)
                .build()?;
            app.manage(TrayItems {
                save: save.clone(),
                screenshot: screenshot.clone(),
            });
            show_shortcuts_in_tray(app.handle(), &settings.get().shortcuts);
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
