// Thin wrappers over the Tauri commands/events defined in src-tauri/src/lib.rs.
// No logic lives here, so there is nothing to unit test; the logic is in
// `lib/*` (pure, tested) and in `library.rs` (tested in Rust).
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { BufferStatus } from "./lib/buffer";
import type { ClipInfo } from "./lib/clips";

export const listClips = () => invoke<ClipInfo[]>("list_clips");
export const renameClip = (name: string, newName: string) =>
  invoke<string>("rename_clip", { name, newName });
export const deleteClip = (name: string) => invoke<void>("delete_clip", { name });
export const revealClip = (name: string) => invoke<void>("reveal_clip", { name });
export const openClipsFolder = () => invoke<void>("open_clips_folder");

export const bufferStatus = () => invoke<BufferStatus>("buffer_status");
export const toggleBuffer = () => invoke<void>("toggle_buffer");
export const saveClip = () => invoke<void>("save_clip");

export const clipSrc = (clip: ClipInfo) => convertFileSrc(clip.path);

export const onClipSaved = (cb: (name: string) => void): Promise<UnlistenFn> =>
  listen<string>("clip-saved", (e) => cb(e.payload));
export const onBufferChanged = (cb: (status: BufferStatus) => void): Promise<UnlistenFn> =>
  listen<BufferStatus>("buffer-changed", (e) => cb(e.payload));
