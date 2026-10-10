// Turning the backend's peak data into something drawable. Pure functions.

/** What `clip_waveform` returns: loudness per `1 / perSecond` of the clip. */
export interface Waveform {
  perSecond: number;
  /** 0–255 per bucket. */
  peaks: number[];
}

/** Anything quieter than this (dBFS) is drawn as silence. */
const FLOOR_DB = -54;
/** A line of this height is always drawn, so a silent lane doesn't look broken. */
const MIN_HEIGHT = 0.02;

/**
 * Height (0–1) a peak (0–255) is drawn at. Loudness is logarithmic, so a
 * quiet voice must not vanish next to an explosion: -6 dB is tall, -20 dB is
 * still clearly visible.
 */
export function levelHeight(peak: number): number {
  const amplitude = Math.min(Math.max(peak, 0), 255) / 255;
  if (amplitude <= 0) return MIN_HEIGHT;
  const db = 20 * Math.log10(amplitude);
  return Math.max((db - FLOOR_DB) / -FLOOR_DB, MIN_HEIGHT);
}

/**
 * `count` heights (0–1) covering `duration` seconds: each column takes the
 * loudest bucket that falls inside it, so short bursts never disappear when
 * the lane is narrower than the data.
 */
export function columns(wave: Waveform, duration: number, count: number): number[] {
  const n = Math.max(Math.floor(count), 0);
  if (n === 0) return [];
  const heights = new Array<number>(n).fill(MIN_HEIGHT);
  if (!(duration > 0) || wave.peaks.length === 0 || !(wave.perSecond > 0)) return heights;
  const bucketsPerColumn = (wave.perSecond * duration) / n;
  for (let i = 0; i < n; i++) {
    const from = Math.floor(i * bucketsPerColumn);
    const to = Math.max(Math.ceil((i + 1) * bucketsPerColumn), from + 1);
    let peak = 0;
    for (let b = from; b < to && b < wave.peaks.length; b++) peak = Math.max(peak, wave.peaks[b]);
    heights[i] = levelHeight(peak);
  }
  return heights;
}

/**
 * SVG path of a mirrored waveform for a `heights.length` × `viewHeight` box:
 * one closed outline, drawn from the left along the top and back along the
 * bottom.
 */
export function waveformPath(heights: number[], viewHeight = 100): string {
  if (heights.length === 0) return "";
  const mid = viewHeight / 2;
  const y = (h: number, sign: 1 | -1) => (mid - sign * h * mid).toFixed(1);
  const top = heights.map((h, i) => `${i === 0 ? "M" : "L"}${i},${y(h, 1)} L${i + 1},${y(h, 1)}`);
  const bottom = heights
    .map((h, i) => `L${i + 1},${y(h, -1)} L${i},${y(h, -1)}`)
    .reverse();
  return `${top.join(" ")} ${bottom.join(" ")} Z`;
}
