/** What a library file is: a recorded video or a picture of the screen. */
export type ClipKind = "clip" | "screenshot";

/** Mirrors `library::ClipInfo` in `src-tauri/src/library.rs`. */
export interface ClipInfo {
  name: string;
  kind: ClipKind;
  path: string;
  sizeBytes: number;
  modifiedMs: number;
  /** Ids of the library folders the clip is filed in. */
  folders: string[];
  /** The game (application) it was recorded in; null when unknown. */
  game: string | null;
}

export interface ClipGroup {
  label: string;
  clips: ClipInfo[];
}

/** `Sin clips`, `1 clip`, `12 clips`. */
export function clipCountLabel(n: number): string {
  if (n === 0) return "Sin clips";
  return n === 1 ? "1 clip" : `${n} clips`;
}

/** `1 captura`, `3 capturas`. */
export function screenshotCountLabel(n: number): string {
  return n === 1 ? "1 captura" : `${n} capturas`;
}

/**
 * What a list holds, in words: `Sin clips`, `12 clips`, `4 capturas`,
 * `12 clips y 4 capturas`.
 */
export function mediaCountLabel(items: readonly ClipInfo[]): string {
  const pictures = items.filter((item) => item.kind === "screenshot").length;
  const videos = items.length - pictures;
  if (pictures === 0) return clipCountLabel(videos);
  if (videos === 0) return screenshotCountLabel(pictures);
  return `${clipCountLabel(videos)} y ${screenshotCountLabel(pictures)}`;
}

/** File name without its `.mp4` / `.png` extension, shown as the title. */
export function clipTitle(name: string): string {
  return name.replace(/\.(mp4|png)$/i, "");
}

function startOfLocalDay(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** `Hoy`, `Ayer`, `3 de octubre` (with the year when it isn't the current one). */
export function dayLabel(ms: number, nowMs: number): string {
  const today = startOfLocalDay(nowMs);
  const day = startOfLocalDay(ms);
  const daysAgo = Math.round((today - day) / 86_400_000);
  if (daysAgo <= 0) return "Hoy";
  if (daysAgo === 1) return "Ayer";
  const sameYear = new Date(ms).getFullYear() === new Date(nowMs).getFullYear();
  return new Intl.DateTimeFormat("es-AR", {
    day: "numeric",
    month: "long",
    ...(sameYear ? {} : { year: "numeric" }),
  }).format(ms);
}

/**
 * Group clips by local calendar day. Keeps the incoming order, so a
 * newest-first list yields newest-first groups.
 */
export function groupByDay(clips: ClipInfo[], nowMs: number): ClipGroup[] {
  const groups: ClipGroup[] = [];
  let currentDay: number | null = null;
  for (const clip of clips) {
    const day = startOfLocalDay(clip.modifiedMs);
    if (day !== currentDay) {
      groups.push({ label: dayLabel(clip.modifiedMs, nowMs), clips: [] });
      currentDay = day;
    }
    groups[groups.length - 1].clips.push(clip);
  }
  return groups;
}
