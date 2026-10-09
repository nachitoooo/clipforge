/** `0:30`, `1:05`, `1:02:03`. */
export function formatDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) return "0:00";
  const whole = Math.round(totalSeconds);
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const seconds = whole % 60;
  const ss = String(seconds).padStart(2, "0");
  if (hours > 0) return `${hours}:${String(minutes).padStart(2, "0")}:${ss}`;
  return `${minutes}:${ss}`;
}

/** `512 B`, `1,5 KB`, `41,0 MB`, `2,3 GB` (decimal comma, as in es-AR). */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(1).replace(".", ",")} ${units[unit]}`;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** `ahora`, `hace 5 min`, `hace 3 h`, `hace 2 d`, or a short date after a week. */
export function formatRelativeTime(thenMs: number, nowMs: number): string {
  const diff = nowMs - thenMs;
  if (diff < MINUTE) return "ahora";
  if (diff < HOUR) return `hace ${Math.floor(diff / MINUTE)} min`;
  if (diff < DAY) return `hace ${Math.floor(diff / HOUR)} h`;
  if (diff < 7 * DAY) return `hace ${Math.floor(diff / DAY)} d`;
  return new Intl.DateTimeFormat("es-AR", {
    day: "numeric",
    month: "short",
  }).format(thenMs);
}
