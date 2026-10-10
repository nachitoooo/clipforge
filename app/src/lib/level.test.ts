import { describe, expect, it } from "vitest";
import { FLOOR_DB, PeakMeter, peakToDb, peakToPercent } from "./level";

describe("peakToDb / peakToPercent", () => {
  it("maps full scale to 0 dB and 100 %", () => {
    expect(peakToDb(1)).toBeCloseTo(0, 6);
    expect(peakToPercent(1)).toBeCloseTo(100, 6);
  });

  it("maps half scale to about -6 dB", () => {
    expect(peakToDb(0.5)).toBeCloseTo(-6.02, 2);
    expect(peakToPercent(0.5)).toBeCloseTo(90, 0);
  });

  it("clamps silence and garbage to the floor", () => {
    for (const bad of [0, -1, NaN, 1e-9]) {
      expect(peakToDb(bad)).toBe(FLOOR_DB);
      expect(peakToPercent(bad)).toBe(0);
    }
  });

  it("clamps overs to 0 dB", () => {
    expect(peakToDb(3)).toBeCloseTo(0, 6);
  });

  it("is monotonic", () => {
    let last = -1;
    for (const p of [0.001, 0.01, 0.1, 0.3, 0.7, 1]) {
      const v = peakToPercent(p);
      expect(v).toBeGreaterThan(last);
      last = v;
    }
  });
});

describe("PeakMeter", () => {
  it("jumps up immediately", () => {
    const m = new PeakMeter();
    m.update(80, 1000);
    expect(m.level).toBe(80);
    expect(m.hold).toBe(80);
  });

  it("falls gradually, not instantly", () => {
    const m = new PeakMeter(700, 100);
    m.update(80, 1000);
    m.update(null, 1100); // 100 ms later: 10 percentage points lower
    expect(m.level).toBeCloseTo(70, 5);
    m.update(null, 1600);
    expect(m.level).toBeCloseTo(20, 5);
  });

  it("never goes below zero", () => {
    const m = new PeakMeter(700, 100);
    m.update(10, 1000);
    m.update(null, 10_000);
    expect(m.level).toBe(0);
  });

  it("holds the maximum for a moment, then lets it fall", () => {
    const m = new PeakMeter(500, 100);
    m.update(90, 1000);
    m.update(null, 1300); // within the hold time
    expect(m.hold).toBe(90);
    m.update(null, 1600); // hold expired at 1500
    expect(m.hold).toBeLessThan(90);
    expect(m.hold).toBeGreaterThanOrEqual(m.level);
  });

  it("a louder reading raises both bar and marker", () => {
    const m = new PeakMeter();
    m.update(40, 1000);
    m.update(60, 1010);
    expect(m.level).toBe(60);
    expect(m.hold).toBe(60);
  });

  it("reset clears everything", () => {
    const m = new PeakMeter();
    m.update(70, 1000);
    m.reset();
    expect([m.level, m.hold]).toEqual([0, 0]);
  });
});
