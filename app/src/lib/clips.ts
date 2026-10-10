/** Mirrors `library::ClipInfo` in `src-tauri/src/library.rs`. */
export interface ClipInfo {
  name: string;
  path: string;
  sizeBytes: number;
  modifiedMs: number;
}

export interface ClipGroup {
  label: string;
  clips: ClipInfo[];
}

/** File name without the `.mp4` extension, shown as the clip title. */
export function clipTitle(name: string): string {
  return name.replace(/\.mp4$/i, "");
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
