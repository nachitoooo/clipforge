import type { ClipInfo } from "./clips";
import { clipTitle } from "./clips";
import type { FolderInfo } from "./folders";
import { foldersOf } from "./folders";

export type DateRange = "all" | "today" | "week" | "month" | "year";
export type SortKey = "newest" | "oldest" | "name" | "size";

/** The "no game" / "no folder" choice of the game and folder filters. */
export const NONE = "__none__";

export interface ClipFilter {
  query: string;
  /** A game key (`gameKey`), `NONE` for clips without one, or null for any. */
  game: string | null;
  /** A folder id, `NONE` for clips in no folder, or null for any. */
  folder: string | null;
  range: DateRange;
  sort: SortKey;
}

export const RANGE_OPTIONS: readonly { id: DateRange; label: string }[] = [
  { id: "all", label: "Todo el tiempo" },
  { id: "today", label: "Hoy" },
  { id: "week", label: "Últimos 7 días" },
  { id: "month", label: "Últimos 30 días" },
  { id: "year", label: "Este año" },
];

export const SORT_OPTIONS: readonly { id: SortKey; label: string }[] = [
  { id: "newest", label: "Más nuevos" },
  { id: "oldest", label: "Más viejos" },
  { id: "name", label: "Nombre (A–Z)" },
  { id: "size", label: "Más pesados" },
];

export function emptyFilter(): ClipFilter {
  return { query: "", game: null, folder: null, range: "all", sort: "newest" };
}

/** True when the filter hides something. The sort order doesn't hide anything. */
export function isFiltering(filter: ClipFilter): boolean {
  return (
    filter.query.trim() !== "" ||
    filter.game !== null ||
    filter.folder !== null ||
    filter.range !== "all"
  );
}

/** Lowercase, without accents: "Águila" and "aguila" are the same word. */
export function normalizeText(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;
export const MAX_GAME_NAME_LENGTH = 60;

export type GameNameCheck = { ok: true; name: string } | { ok: false; error: string };

/** Client-side mirror of `clipmeta::validate_game` (Rust stays authoritative). */
export function validateGameName(input: string): GameNameCheck {
  const name = input.trim();
  if (name === "") return { ok: false, error: "El nombre del juego no puede estar vacío" };
  if ([...name].length > MAX_GAME_NAME_LENGTH) {
    return {
      ok: false,
      error: `El nombre del juego no puede superar ${MAX_GAME_NAME_LENGTH} caracteres`,
    };
  }
  if (CONTROL.test(name)) {
    return { ok: false, error: "El nombre del juego contiene caracteres no permitidos" };
  }
  return { ok: true, name };
}

/** What two spellings of the same game share: "Valorant" and "VALORANT". */
export function gameKey(name: string): string {
  return name.trim().toLowerCase();
}

export interface GameOption {
  key: string;
  name: string;
  count: number;
}

export interface GameOptions {
  /** Most used first, then by name. */
  games: GameOption[];
  withoutGame: number;
}

export function gameOptions(clips: readonly ClipInfo[]): GameOptions {
  const byKey = new Map<string, GameOption>();
  let withoutGame = 0;
  for (const clip of clips) {
    if (clip.game === null) {
      withoutGame++;
      continue;
    }
    const key = gameKey(clip.game);
    const found = byKey.get(key);
    if (found) found.count++;
    else byKey.set(key, { key, name: clip.game, count: 1 });
  }
  const games = [...byKey.values()].sort(
    (a, b) => b.count - a.count || a.name.localeCompare(b.name, "es", { sensitivity: "base" }),
  );
  return { games, withoutGame };
}

function startOfLocalDay(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** The first instant a range includes (local time), or null for "all". */
export function rangeStart(range: DateRange, nowMs: number): number | null {
  const today = startOfLocalDay(nowMs);
  const d = new Date(today);
  switch (range) {
    case "all":
      return null;
    case "today":
      return today;
    case "week":
      return new Date(d.getFullYear(), d.getMonth(), d.getDate() - 6).getTime();
    case "month":
      return new Date(d.getFullYear(), d.getMonth(), d.getDate() - 29).getTime();
    case "year":
      return new Date(d.getFullYear(), 0, 1).getTime();
  }
}

const byName = (a: ClipInfo, b: ClipInfo) =>
  clipTitle(a.name).localeCompare(clipTitle(b.name), "es", { numeric: true, sensitivity: "base" });

const SORTERS: Record<SortKey, (a: ClipInfo, b: ClipInfo) => number> = {
  newest: (a, b) => b.modifiedMs - a.modifiedMs || b.name.localeCompare(a.name),
  oldest: (a, b) => a.modifiedMs - b.modifiedMs || a.name.localeCompare(b.name),
  name: (a, b) => byName(a, b) || a.name.localeCompare(b.name),
  size: (a, b) => b.sizeBytes - a.sizeBytes || b.name.localeCompare(a.name),
};

/**
 * The clips that pass `filter`, in its order. The search looks at the title,
 * the game and the names of the clip's folders (they are its tags); every word
 * typed has to match somewhere.
 */
export function applyFilter(
  clips: readonly ClipInfo[],
  folders: readonly FolderInfo[],
  filter: ClipFilter,
  nowMs: number,
): ClipInfo[] {
  const words = normalizeText(filter.query).split(/\s+/).filter(Boolean);
  const from = rangeStart(filter.range, nowMs);

  const passes = (clip: ClipInfo): boolean => {
    if (from !== null && clip.modifiedMs < from) return false;
    if (filter.game !== null) {
      const wanted = filter.game === NONE ? clip.game === null : clip.game !== null && gameKey(clip.game) === filter.game;
      if (!wanted) return false;
    }
    if (filter.folder !== null) {
      const wanted = filter.folder === NONE ? clip.folders.length === 0 : clip.folders.includes(filter.folder);
      if (!wanted) return false;
    }
    if (words.length > 0) {
      const haystack = normalizeText(
        [clipTitle(clip.name), clip.game ?? "", ...foldersOf(clip, folders).map((f) => f.name)].join(" "),
      );
      if (!words.every((word) => haystack.includes(word))) return false;
    }
    return true;
  };

  return clips.filter(passes).sort(SORTERS[filter.sort]);
}

/** Group by day only when the order is by date; any other order is one flat list. */
export function layoutFor(sort: SortKey): "by-day" | "flat" {
  return sort === "newest" || sort === "oldest" ? "by-day" : "flat";
}
