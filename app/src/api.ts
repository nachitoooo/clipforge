// Thin wrappers over the Tauri commands/events defined in src-tauri/src/lib.rs.
// No logic lives here, so there is nothing to unit test; the logic is in
// `lib/*` (pure, tested) and in `library.rs` (tested in Rust).
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { BufferStatus } from "./lib/buffer";
import type { ClipInfo } from "./lib/clips";
import type { EditorClip, TrimRequest, TrimResult } from "./lib/editor";
import type { FolderDraft, FolderInfo } from "./lib/folders";
import type { MemoryPlans, MicDevice, MicStatus, Settings } from "./lib/settings";
import type { Waveform } from "./lib/waveform";

export const listClips = () => invoke<ClipInfo[]>("list_clips");
export const renameClip = (name: string, newName: string) =>
  invoke<string>("rename_clip", { name, newName });
export const deleteClip = (name: string) => invoke<void>("delete_clip", { name });
export const revealClip = (name: string) => invoke<void>("reveal_clip", { name });
/** Put the clip's file on the system clipboard, to paste it in a chat or a mail. */
export const copyClip = (name: string) => invoke<void>("copy_clip", { name });
/** Set (or clear, with null) a clip's game; with `all`, every clip of its current game. Resolves with how many changed. */
export const setClipGame = (name: string, game: string | null, all: boolean) =>
  invoke<number>("set_clip_game", { name, game, all });
export const openClipsFolder = () => invoke<void>("open_clips_folder");

// Library folders (src-tauri/src/folders.rs)
export const listFolders = () => invoke<FolderInfo[]>("list_folders");
export const createFolder = (draft: FolderDraft) => invoke<FolderInfo>("create_folder", { draft });
export const updateFolder = (id: string, draft: FolderDraft) =>
  invoke<FolderInfo>("update_folder", { id, draft });
/** Resolves with how many clip files were deleted. */
export const deleteFolder = (id: string, deleteClips: boolean) =>
  invoke<number>("delete_folder", { id, deleteClips });
export const setClipFolders = (name: string, folderIds: string[]) =>
  invoke<void>("set_clip_folders", { name, folderIds });
/** Resolves with how many of the clips were not already in the folder. */
export const addToFolder = (id: string, names: string[]) =>
  invoke<number>("add_to_folder", { id, names });
export const removeFromFolder = (id: string, names: string[]) =>
  invoke<number>("remove_from_folder", { id, names });

// Clip editor (src-tauri/src/editor.rs)
export const describeClip = (name: string) => invoke<EditorClip>("describe_clip", { name });
export const clipWaveform = (name: string, track: number) =>
  invoke<Waveform>("clip_waveform", { name, track });
export const trimClip = (request: TrimRequest) => invoke<TrimResult>("trim_clip", { request });

export const bufferStatus = () => invoke<BufferStatus>("buffer_status");
export const toggleBuffer = () => invoke<void>("toggle_buffer");
export const saveClip = () => invoke<void>("save_clip");
/** Save a picture of the screen next to the clips; the result arrives as a notification and a `clip-saved` event. */
export const takeScreenshot = () => invoke<void>("take_screenshot");

export const clipSrc = (clip: ClipInfo) => convertFileSrc(clip.path);

export const onClipSaved = (cb: (name: string) => void): Promise<UnlistenFn> =>
  listen<string>("clip-saved", (e) => cb(e.payload));
/** Clips or folders changed (from any view): reload what you show. */
export const onLibraryChanged = (cb: () => void): Promise<UnlistenFn> =>
  listen<null>("library-changed", () => cb());
export const onBufferChanged = (cb: (status: BufferStatus) => void): Promise<UnlistenFn> =>
  listen<BufferStatus>("buffer-changed", (e) => cb(e.payload));

export const getSettings = () => invoke<Settings>("get_settings");
/** What each memory mode (normal / low RAM) costs. */
export const memoryPlans = () => invoke<MemoryPlans>("memory_plans");
export const updateSettings = (settings: Settings) =>
  invoke<Settings>("update_settings", { settings });
export const listMicrophones = () => invoke<MicDevice[]>("list_microphones");
export const microphoneStatus = () => invoke<MicStatus>("microphone_status");
export const retryMicrophone = () => invoke<void>("retry_microphone");
/** The level meter only streams while this is true (Settings is open). */
export const setMeterActive = (active: boolean) => invoke<void>("set_meter_active", { active });

export const onSettingsChanged = (cb: (settings: Settings) => void): Promise<UnlistenFn> =>
  listen<Settings>("settings-changed", (e) => cb(e.payload));
export const onMicStatus = (cb: (status: MicStatus) => void): Promise<UnlistenFn> =>
  listen<MicStatus>("mic-status", (e) => cb(e.payload));
/** Peak input level, 0–1 linear, ~20 times a second. */
export const onMicLevel = (cb: (peak: number) => void): Promise<UnlistenFn> =>
  listen<number>("mic-level", (e) => cb(e.payload));
