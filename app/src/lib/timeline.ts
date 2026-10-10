// Pure time math for the clip editor's timeline: nothing here touches the DOM.

/** A piece of the clip, in seconds. `end` is exclusive. */
export interface Range {
  start: number;
  end: number;
}

export interface Tick {
  time: number;
  label: string;
}

const clamp = (value: number, low: number, high: number) => Math.min(Math.max(value, low), high);

/** One frame, but never less than 20 ms: the shortest selection allowed. */
export function minSelection(fps: number): number {
  return Math.max(1 / Math.max(fps, 1), 0.02);
}

/** The closest frame boundary to `time`. */
export function snapToFrame(time: number, fps: number): number {
  if (!Number.isFinite(fps) || fps <= 0) return time;
  return Math.round(time * fps) / fps;
}

/** `range` forced inside `[0, duration]` with at least `minLength` between the ends. */
export function clampRange(range: Range, duration: number, minLength: number): Range {
  const length = Math.min(minLength, duration);
  const start = clamp(range.start, 0, Math.max(duration - length, 0));
  const end = clamp(range.end, start + length, duration);
  return { start, end };
}

/** Drag the start handle: the end stays where it is. */
export function withStart(range: Range, time: number, duration: number, minLength: number): Range {
  const length = Math.min(minLength, duration);
  return {
    start: clamp(time, 0, Math.max(Math.min(range.end, duration) - length, 0)),
    end: range.end,
  };
}

/** Drag the end handle: the start stays where it is. */
export function withEnd(range: Range, time: number, duration: number, minLength: number): Range {
  const length = Math.min(minLength, duration);
  return {
    start: range.start,
    end: clamp(time, Math.min(range.start + length, duration), duration),
  };
}

/** Slide the whole selection by `delta` seconds, keeping its length. */
export function shiftRange(range: Range, delta: number, duration: number): Range {
  const length = range.end - range.start;
  const start = clamp(range.start + delta, 0, Math.max(duration - length, 0));
  return { start, end: start + length };
}

/** True when the selection covers the whole clip (give or take rounding). */
export function isWhole(range: Range, duration: number, tolerance = 0.001): boolean {
  return range.start <= tolerance && range.end >= duration - tolerance;
}

/** Position of `time` along the clip as a percentage, clamped to 0–100. */
export function timeToPercent(time: number, duration: number): number {
  if (!(duration > 0)) return 0;
  return clamp((time / duration) * 100, 0, 100);
}

/** Inverse of [`timeToPercent`], for a pointer at `fraction` (0–1) of the track. */
export function fractionToTime(fraction: number, duration: number): number {
  return clamp(fraction, 0, 1) * Math.max(duration, 0);
}

const TICK_STEPS = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600];

/**
 * Labelled marks for the ruler: the smallest "round" step that keeps at
 * least `minSpacing` pixels between labels.
 */
export function rulerTicks(duration: number, widthPx: number, minSpacing = 80): Tick[] {
  if (!(duration > 0) || !(widthPx > 0)) return [];
  const pxPerSecond = widthPx / duration;
  const step = TICK_STEPS.find((s) => s * pxPerSecond >= minSpacing) ?? TICK_STEPS[TICK_STEPS.length - 1];
  const ticks: Tick[] = [];
  for (let n = 0; n * step <= duration + 1e-9; n++) {
    const time = n * step;
    ticks.push({ time, label: formatTimecode(time, step < 1 ? 1 : 0) });
  }
  return ticks;
}

/** `0:05`, `1:02.35`, `1:02:03.5`: minutes and seconds, `decimals` fraction digits. */
export function formatTimecode(totalSeconds: number, decimals = 2): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) totalSeconds = 0;
  const scale = 10 ** decimals;
  const units = Math.round(totalSeconds * scale);
  const whole = Math.floor(units / scale);
  const fraction = units - whole * scale;
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const seconds = whole % 60;
  const ss = String(seconds).padStart(2, "0") + (decimals > 0 ? `.${String(fraction).padStart(decimals, "0")}` : "");
  if (hours > 0) return `${hours}:${String(minutes).padStart(2, "0")}:${ss}`;
  return `${minutes}:${ss}`;
}

/**
 * What a person types into a time box: `12.5`, `12,5`, `0:12.5`, `1:02`,
 * `1:02:03.5`. Returns seconds, or `null` when it isn't a time.
 */
export function parseTimecode(text: string): number | null {
  const cleaned = text.trim().replace(",", ".");
  if (!/^\d+(\.\d+)?$|^(\d+:){1,2}\d+(\.\d+)?$/.test(cleaned)) return null;
  const parts = cleaned.split(":").map(Number);
  const seconds = parts.reduce((total, part) => total * 60 + part, 0);
  return Number.isFinite(seconds) ? seconds : null;
}
