import { describe, expect, it } from "vitest";
import { formatBytes, formatDuration, formatRelativeTime } from "./format";

describe("formatDuration", () => {
  it("formats seconds, minutes and hours", () => {
    expect(formatDuration(0)).toBe("0:00");
    expect(formatDuration(5)).toBe("0:05");
    expect(formatDuration(30)).toBe("0:30");
    expect(formatDuration(65)).toBe("1:05");
    expect(formatDuration(3723)).toBe("1:02:03");
  });

  it("rounds to the nearest second", () => {
    expect(formatDuration(29.6)).toBe("0:30");
    expect(formatDuration(59.6)).toBe("1:00");
  });

  it("is safe on bad input", () => {
    expect(formatDuration(-1)).toBe("0:00");
    expect(formatDuration(NaN)).toBe("0:00");
    expect(formatDuration(Infinity)).toBe("0:00");
  });
});

describe("formatBytes", () => {
  it("scales units with a decimal comma", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1,5 KB");
    expect(formatBytes(41 * 1024 * 1024)).toBe("41,0 MB");
    expect(formatBytes(2.3 * 1024 ** 3)).toBe("2,3 GB");
  });

  it("is safe on bad input", () => {
    expect(formatBytes(-5)).toBe("0 B");
    expect(formatBytes(NaN)).toBe("0 B");
  });
});

describe("formatRelativeTime", () => {
  const now = Date.UTC(2026, 9, 9, 12, 0, 0);
  const ago = (ms: number) => formatRelativeTime(now - ms, now);

  it("uses coarse relative units", () => {
    expect(ago(0)).toBe("ahora");
    expect(ago(59_000)).toBe("ahora");
    expect(ago(60_000)).toBe("hace 1 min");
    expect(ago(59 * 60_000)).toBe("hace 59 min");
    expect(ago(60 * 60_000)).toBe("hace 1 h");
    expect(ago(23 * 3_600_000)).toBe("hace 23 h");
    expect(ago(24 * 3_600_000)).toBe("hace 1 d");
    expect(ago(6 * 86_400_000)).toBe("hace 6 d");
  });

  it("falls back to a short date after a week", () => {
    expect(ago(7 * 86_400_000)).toMatch(/\d/);
    expect(ago(7 * 86_400_000)).not.toMatch(/hace/);
  });

  it("treats future timestamps (clock skew) as now", () => {
    expect(formatRelativeTime(now + 5_000, now)).toBe("ahora");
  });
});
