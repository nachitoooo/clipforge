import type { ClipInfo } from "./clips";

/** Mirrors `folders::FolderInfo` in `src-tauri/src/folders.rs`. */
export interface FolderInfo {
  id: string;
  name: string;
  /** A palette id, see `FOLDER_COLORS`. */
  color: string;
  /** An icon id, see `FOLDER_ICON_IDS`. */
  icon: string;
  /** Clips that exist on disk right now. */
  clipCount: number;
  sizeBytes: number;
  createdMs: number;
}

/** What the folder dialog produces; mirrors `folders::FolderDraft`. */
export interface FolderDraft {
  name: string;
  color: string;
  icon: string;
}

export const MAX_FOLDER_NAME_LENGTH = 60;

export interface FolderColor {
  id: string;
  label: string;
  hex: string;
}

/** The palette. Rust only checks that a color is a short id; the colors live here. */
export const FOLDER_COLORS: readonly FolderColor[] = [
  { id: "orange", label: "Naranja", hex: "#ff7a1a" },
  { id: "red", label: "Rojo", hex: "#ff4d5e" },
  { id: "pink", label: "Rosa", hex: "#ff6fb5" },
  { id: "purple", label: "Violeta", hex: "#a77bff" },
  { id: "blue", label: "Azul", hex: "#4da3ff" },
  { id: "teal", label: "Turquesa", hex: "#2ec4b6" },
  { id: "green", label: "Verde", hex: "#3ddc84" },
  { id: "yellow", label: "Amarillo", hex: "#ffd166" },
  { id: "gray", label: "Gris", hex: "#8b90a3" },
];
export const DEFAULT_FOLDER_COLOR = "orange";

export interface FolderIconChoice {
  id: string;
  label: string;
}

export const FOLDER_ICONS: readonly FolderIconChoice[] = [
  { id: "folder", label: "Carpeta" },
  { id: "star", label: "Estrella" },
  { id: "trophy", label: "Trofeo" },
  { id: "gamepad", label: "Joystick" },
  { id: "bolt", label: "Rayo" },
  { id: "heart", label: "Corazón" },
  { id: "target", label: "Objetivo" },
  { id: "flame", label: "Llama" },
];
export const DEFAULT_FOLDER_ICON = "folder";

/** The color for a palette id; an id this version doesn't know gets the default. */
export function folderColor(id: string): string {
  const found = FOLDER_COLORS.find((c) => c.id === id);
  return (found ?? FOLDER_COLORS.find((c) => c.id === DEFAULT_FOLDER_COLOR)!).hex;
}

/** The icon id to draw; an unknown one falls back to the plain folder. */
export function folderIconId(id: string): string {
  return FOLDER_ICONS.some((i) => i.id === id) ? id : DEFAULT_FOLDER_ICON;
}

export type FolderNameCheck = { ok: true; name: string } | { ok: false; error: string };

// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

/**
 * Client-side mirror of the name rules in `folders.rs` (Rust stays
 * authoritative; this gives instant feedback). A folder name is a label, not a
 * file name, so `/` or `:` are fine. `exceptId` is the folder being renamed: it
 * may keep its own name.
 */
export function validateFolderName(
  input: string,
  folders: readonly FolderInfo[],
  exceptId?: string,
): FolderNameCheck {
  const name = input.trim();
  if (name === "") return { ok: false, error: "El nombre no puede estar vacío" };
  if ([...name].length > MAX_FOLDER_NAME_LENGTH) {
    return {
      ok: false,
      error: `El nombre no puede superar ${MAX_FOLDER_NAME_LENGTH} caracteres`,
    };
  }
  if (CONTROL.test(name)) {
    return { ok: false, error: "El nombre contiene caracteres no permitidos" };
  }
  const wanted = name.toLowerCase();
  if (folders.some((f) => f.id !== exceptId && f.name.toLowerCase() === wanted)) {
    return { ok: false, error: "Ya existe una carpeta con ese nombre" };
  }
  return { ok: true, name };
}

/** Alphabetical, the way people sort Spanish names. */
export function sortFolders(folders: readonly FolderInfo[]): FolderInfo[] {
  return [...folders].sort(
    (a, b) =>
      a.name.localeCompare(b.name, "es", { sensitivity: "base" }) || a.id.localeCompare(b.id),
  );
}

/** The clips filed in a folder, in the order given. */
export function clipsIn(clips: readonly ClipInfo[], folderId: string): ClipInfo[] {
  return clips.filter((c) => c.folders.includes(folderId));
}

/** The clips that are in no folder. */
export function looseClips(clips: readonly ClipInfo[]): ClipInfo[] {
  return clips.filter((c) => c.folders.length === 0);
}

/** The folders a clip is in (only ones that still exist), sorted by name. */
export function foldersOf(clip: ClipInfo, folders: readonly FolderInfo[]): FolderInfo[] {
  return sortFolders(folders.filter((f) => clip.folders.includes(f.id)));
}

export interface DeleteBreakdown {
  /** Clips in the folder. */
  total: number;
  /** In this folder and no other: the ones "delete everything" would remove. */
  exclusive: number;
  /** Also in another folder: kept either way. */
  shared: number;
}

export function deleteBreakdown(folder: FolderInfo, clips: readonly ClipInfo[]): DeleteBreakdown {
  const inside = clipsIn(clips, folder.id);
  const exclusive = inside.filter((c) => c.folders.length === 1).length;
  return { total: inside.length, exclusive, shared: inside.length - exclusive };
}
