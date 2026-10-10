import { describe, expect, it } from "vitest";
import { columns, levelHeight, waveformPath } from "./waveform";

describe("levelHeight", () => {
  it("grows with loudness", () => {
    const heights = [0, 2, 8, 32, 128, 255].map(levelHeight);
    for (let i = 1; i < heights.length; i++) expect(heights[i]).toBeGreaterThan(heights[i - 1]);
  });

  it("is full height at full scale and a hairline for silence", () => {
    expect(levelHeight(255)).toBeCloseTo(1);
    expect(levelHeight(0)).toBe(0.02);
  });

  it("keeps quiet sounds visible", () => {
    // About -20 dBFS (a normal voice) must be over half the lane.
    expect(levelHeight(255 * 0.1)).toBeGreaterThan(0.5);
    // Half scale is tall.
    expect(levelHeight(128)).toBeGreaterThan(0.85);
  });

  it("clamps what is out of range", () => {
    expect(levelHeight(-5)).toBe(0.02);
    expect(levelHeight(900)).toBeCloseTo(1);
  });
});

describe("columns", () => {
  it("lays one bucket per column when they match", () => {
    const wave = { perSecond: 2, peaks: [0, 255, 0, 255] };
    const heights = columns(wave, 2, 4);
    expect(heights).toHaveLength(4);
    expect(heights[1]).toBeCloseTo(1);
    expect(heights[0]).toBe(0.02);
  });

  it("keeps short bursts when the lane is narrower than the data", () => {
    const peaks = new Array(100).fill(0);
    peaks[57] = 255;
    const heights = columns({ perSecond: 10, peaks }, 10, 10);
    expect(heights.filter((h) => h > 0.5)).toHaveLength(1);
    expect(heights[5]).toBeCloseTo(1);
  });

  it("stretches the data over wider lanes", () => {
    const heights = columns({ perSecond: 1, peaks: [255, 0] }, 2, 8);
    expect(heights.slice(0, 4).every((h) => h > 0.9)).toBe(true);
    expect(heights.slice(4).every((h) => h === 0.02)).toBe(true);
  });

  it("copes with missing data", () => {
    expect(columns({ perSecond: 50, peaks: [] }, 10, 3)).toEqual([0.02, 0.02, 0.02]);
    expect(columns({ perSecond: 50, peaks: [1] }, 0, 3)).toEqual([0.02, 0.02, 0.02]);
    expect(columns({ perSecond: 50, peaks: [1] }, 10, 0)).toEqual([]);
    expect(columns({ perSecond: 0, peaks: [255] }, 10, 2)).toEqual([0.02, 0.02]);
  });

  it("ignores data past the end of the clip", () => {
    const heights = columns({ perSecond: 1, peaks: [0, 0, 255, 255] }, 2, 2);
    expect(heights).toEqual([0.02, 0.02]);
  });
});

describe("waveformPath", () => {
  it("is a closed outline that mirrors around the middle", () => {
    const path = waveformPath([1, 0.5], 100);
    expect(path.startsWith("M0,0.0")).toBe(true);
    expect(path.endsWith("Z")).toBe(true);
    expect(path).toContain("L1,100.0"); // bottom of the first, loudest column
    expect(path).toContain("L2,25.0"); // top of the second at half height
  });

  it("is empty for no columns", () => {
    expect(waveformPath([])).toBe("");
  });

  it("stays inside the view box", () => {
    const numbers = waveformPath([0, 0.3, 1], 100)
      .replace(/[MLZ]/g, " ")
      .split(/[\s,]+/)
      .filter(Boolean)
      .map(Number);
    expect(numbers.every((n) => n >= 0 && n <= 100)).toBe(true);
  });
});
