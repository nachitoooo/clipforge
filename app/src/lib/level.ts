/** Bottom of the meter: anything quieter reads as empty. */
export const FLOOR_DB = -60;

/** Peak (0–1, linear) to decibels; silence is clamped to the floor. */
export function peakToDb(peak: number): number {
  if (!(peak > 0)) return FLOOR_DB;
  return Math.max(FLOOR_DB, 20 * Math.log10(Math.min(peak, 1)));
}

/** Meter fill, 0–100: decibels mapped linearly from the floor up to 0 dB. */
export function peakToPercent(peak: number): number {
  return ((peakToDb(peak) - FLOOR_DB) / -FLOOR_DB) * 100;
}

/**
 * Meter ballistics: the bar jumps up at once and falls back gradually, and a
 * thin marker holds the recent maximum for a moment before dropping.
 */
export class PeakMeter {
  level = 0;
  hold = 0;
  private holdUntil = 0;
  private lastTick = 0;

  constructor(
    /** How long the marker stays at its maximum. */
    private readonly holdMs = 700,
    /** How fast the bar falls, in percent per second. */
    private readonly fallPerSecond = 90,
  ) {}

  /** Feed a new reading (percent, or `null` for none) at time `nowMs`. */
  update(percent: number | null, nowMs: number): void {
    const dt = this.lastTick === 0 ? 0 : Math.max(0, nowMs - this.lastTick) / 1000;
    this.lastTick = nowMs;

    this.level = Math.max(0, this.level - this.fallPerSecond * dt);
    if (percent !== null && percent > this.level) this.level = percent;

    if (this.level >= this.hold) {
      this.hold = this.level;
      this.holdUntil = nowMs + this.holdMs;
    } else if (nowMs > this.holdUntil) {
      this.hold = Math.max(this.level, this.hold - this.fallPerSecond * dt);
    }
  }

  reset(): void {
    this.level = 0;
    this.hold = 0;
    this.holdUntil = 0;
    this.lastTick = 0;
  }
}
