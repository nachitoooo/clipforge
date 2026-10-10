import { describe, expect, it } from "vitest";
import {
  clampRange,
  formatTimecode,
  fractionToTime,
  isWhole,
  minSelection,
  parseTimecode,
  rulerTicks,
  shiftRange,
  snapToFrame,
  timeToPercent,
  withEnd,
  withStart,
} from "./timeline";

describe("minSelection", () => {
  it("is one frame, but never below 20 ms", () => {
    expect(minSelection(60)).toBeCloseTo(1 / 60);
    expect(minSelection(30)).toBeCloseTo(1 / 30);
    expect(minSelection(240)).toBe(0.02);
    expect(minSelection(0)).toBe(1); // nonsense fps falls back to 1 fps
  });
});

describe("snapToFrame", () => {
  it("lands on the nearest frame boundary", () => {
    expect(snapToFrame(0.01, 60)).toBeCloseTo(1 / 60);
    expect(snapToFrame(0.0083, 60)).toBe(0);
    expect(snapToFrame(12.3, 30)).toBeCloseTo(12.3, 1);
    expect(snapToFrame(1.5, 60)).toBe(1.5);
  });

  it("leaves time alone when the frame rate is unknown", () => {
    expect(snapToFrame(1.234, 0)).toBe(1.234);
    expect(snapToFrame(1.234, Number.NaN)).toBe(1.234);
  });
});

describe("handles", () => {
  const range = { start: 5, end: 15 };

  it("the start handle stays inside the clip and before the end", () => {
    expect(withStart(range, 8, 30, 0.1)).toEqual({ start: 8, end: 15 });
    expect(withStart(range, -3, 30, 0.1)).toEqual({ start: 0, end: 15 });
    expect(withStart(range, 99, 30, 0.1).start).toBeCloseTo(14.9);
    expect(withStart(range, 99, 30, 0.1).end).toBe(15);
  });

  it("the end handle stays inside the clip and after the start", () => {
    expect(withEnd(range, 20, 30, 0.1)).toEqual({ start: 5, end: 20 });
    expect(withEnd(range, 99, 30, 0.1)).toEqual({ start: 5, end: 30 });
    expect(withEnd(range, 0, 30, 0.1).end).toBeCloseTo(5.1);
  });

  it("a clip shorter than the minimum selection can still be dragged", () => {
    expect(withStart({ start: 0, end: 0.05 }, 0.04, 0.05, 1)).toEqual({ start: 0, end: 0.05 });
    expect(withEnd({ start: 0, end: 0.05 }, 0, 0.05, 1)).toEqual({ start: 0, end: 0.05 });
  });

  it("an empty clip does not produce NaN", () => {
    expect(withStart({ start: 0, end: 0 }, 1, 0, 0.1)).toEqual({ start: 0, end: 0 });
    expect(clampRange({ start: 3, end: 1 }, 0, 0.1)).toEqual({ start: 0, end: 0 });
  });
});

describe("clampRange", () => {
  it("repairs ranges that are out of order or outside the clip", () => {
    expect(clampRange({ start: -2, end: 50 }, 30, 0.1)).toEqual({ start: 0, end: 30 });
    expect(clampRange({ start: 10, end: 4 }, 30, 0.5)).toEqual({ start: 10, end: 10.5 });
    expect(clampRange({ start: 29.99, end: 30 }, 30, 0.5)).toEqual({ start: 29.5, end: 30 });
  });
});

describe("shiftRange", () => {
  it("slides while keeping the length, stopping at the edges", () => {
    expect(shiftRange({ start: 5, end: 10 }, 2, 30)).toEqual({ start: 7, end: 12 });
    expect(shiftRange({ start: 5, end: 10 }, -50, 30)).toEqual({ start: 0, end: 5 });
    expect(shiftRange({ start: 5, end: 10 }, 50, 30)).toEqual({ start: 25, end: 30 });
  });
});

describe("isWhole", () => {
  it("is true for the full clip, give or take rounding", () => {
    expect(isWhole({ start: 0, end: 30 }, 30)).toBe(true);
    expect(isWhole({ start: 0.0004, end: 29.9996 }, 30)).toBe(true);
    expect(isWhole({ start: 0.5, end: 30 }, 30)).toBe(false);
    expect(isWhole({ start: 0, end: 29 }, 30)).toBe(false);
  });
});

describe("positions", () => {
  it("maps time to a percentage and back", () => {
    expect(timeToPercent(15, 30)).toBe(50);
    expect(timeToPercent(-1, 30)).toBe(0);
    expect(timeToPercent(99, 30)).toBe(100);
    expect(timeToPercent(5, 0)).toBe(0);
    expect(fractionToTime(0.25, 40)).toBe(10);
    expect(fractionToTime(-1, 40)).toBe(0);
    expect(fractionToTime(2, 40)).toBe(40);
  });
});

describe("rulerTicks", () => {
  it("picks a round step that keeps the labels apart", () => {
    // 30 s over 1200 px = 40 px/s: 2 s is the first step with >= 80 px.
    const ticks = rulerTicks(30, 1200);
    expect(ticks.map((t) => t.time).slice(0, 4)).toEqual([0, 2, 4, 6]);
    expect(ticks[ticks.length - 1].time).toBe(30);
    const gaps = ticks.slice(1).map((t, i) => ((t.time - ticks[i].time) * 1200) / 30);
    expect(Math.min(...gaps)).toBeGreaterThanOrEqual(80);
  });

  it("uses bigger steps for long clips and in narrow windows", () => {
    expect(rulerTicks(120, 1200)[1].time).toBe(10);
    expect(rulerTicks(30, 300)[1].time).toBe(10);
  });

  it("shows tenths when the step is under a second", () => {
    const ticks = rulerTicks(4, 2000);
    expect(ticks[1]).toEqual({ time: 0.5, label: "0:00.5" });
  });

  it("is empty without a duration or a width", () => {
    expect(rulerTicks(0, 1000)).toEqual([]);
    expect(rulerTicks(30, 0)).toEqual([]);
  });
});

describe("formatTimecode", () => {
  it("formats minutes, seconds and hundredths", () => {
    expect(formatTimecode(0)).toBe("0:00.00");
    expect(formatTimecode(12.345)).toBe("0:12.35");
    expect(formatTimecode(65.5)).toBe("1:05.50");
    expect(formatTimecode(3723.4, 1)).toBe("1:02:03.4");
  });

  it("can drop the fraction", () => {
    expect(formatTimecode(59.6, 0)).toBe("1:00");
    expect(formatTimecode(5, 0)).toBe("0:05");
  });

  it("carries rounding into the next second", () => {
    expect(formatTimecode(9.999)).toBe("0:10.00");
    expect(formatTimecode(59.999)).toBe("1:00.00");
  });

  it("never prints negative or non-finite times", () => {
    expect(formatTimecode(-3)).toBe("0:00.00");
    expect(formatTimecode(Number.NaN)).toBe("0:00.00");
    expect(formatTimecode(Number.POSITIVE_INFINITY)).toBe("0:00.00");
  });
});

describe("parseTimecode", () => {
  it("reads what a person types", () => {
    expect(parseTimecode("12.5")).toBe(12.5);
    expect(parseTimecode("12,5")).toBe(12.5);
    expect(parseTimecode("0:12.5")).toBe(12.5);
    expect(parseTimecode("1:02")).toBe(62);
    expect(parseTimecode("1:02:03.5")).toBe(3723.5);
    expect(parseTimecode("  7 ")).toBe(7);
  });

  it("rejects anything else", () => {
    for (const bad of ["", "abc", "-1", "1:", ":30", "1:2:3:4", "1..2", "1e3", "12 s"]) {
      expect(parseTimecode(bad), JSON.stringify(bad)).toBeNull();
    }
  });

  it("round-trips with formatTimecode", () => {
    for (const t of [0, 0.5, 12.34, 61.01, 599.99]) {
      expect(parseTimecode(formatTimecode(t))).toBeCloseTo(t, 2);
    }
  });
});
