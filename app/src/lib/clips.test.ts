import { describe, expect, it } from "vitest";
import { clipTitle, dayLabel, groupByDay, type ClipInfo } from "./clips";

// Local-time constructors: the grouping is by *local* calendar day.
const at = (y: number, m: number, d: number, h = 12, min = 0) =>
  new Date(y, m, d, h, min).getTime();

const clip = (name: string, modifiedMs: number): ClipInfo => ({
  name,
  path: `/clips/${name}`,
  sizeBytes: 1,
  modifiedMs,
  folders: [],
});

describe("clipTitle", () => {
  it("strips the mp4 extension only", () => {
    expect(clipTitle("clip-20260707-123456.mp4")).toBe("clip-20260707-123456");
    expect(clipTitle("UPPER.MP4")).toBe("UPPER");
    expect(clipTitle("my.mp4.backup.mp4")).toBe("my.mp4.backup");
    expect(clipTitle("no-extension")).toBe("no-extension");
  });
});

describe("dayLabel", () => {
  const now = at(2026, 9, 9, 15);

  it("labels today and yesterday", () => {
    expect(dayLabel(at(2026, 9, 9, 0, 1), now)).toBe("Hoy");
    expect(dayLabel(at(2026, 9, 9, 14), now)).toBe("Hoy");
    expect(dayLabel(at(2026, 9, 8, 23, 59), now)).toBe("Ayer");
  });

  it("uses a long date for older days", () => {
    expect(dayLabel(at(2026, 9, 3), now)).toMatch(/3 de octubre/);
    expect(dayLabel(at(2026, 9, 3), now)).not.toMatch(/2026/);
  });

  it("adds the year when it differs from the current one", () => {
    expect(dayLabel(at(2025, 11, 31), now)).toMatch(/2025/);
  });

  it("treats a future timestamp as today", () => {
    expect(dayLabel(at(2026, 9, 10), now)).toBe("Hoy");
  });
});

describe("groupByDay", () => {
  const now = at(2026, 9, 9, 15);

  it("returns no groups for an empty library", () => {
    expect(groupByDay([], now)).toEqual([]);
  });

  it("groups consecutive clips of the same day, newest first", () => {
    const clips = [
      clip("c.mp4", at(2026, 9, 9, 14)),
      clip("b.mp4", at(2026, 9, 9, 9)),
      clip("a.mp4", at(2026, 9, 8, 22)),
      clip("old.mp4", at(2026, 9, 1, 10)),
    ];
    const groups = groupByDay(clips, now);
    expect(groups.map((g) => g.label)).toEqual([
      "Hoy",
      "Ayer",
      expect.stringMatching(/1 de octubre/),
    ]);
    expect(groups.map((g) => g.clips.map((c) => c.name))).toEqual([
      ["c.mp4", "b.mp4"],
      ["a.mp4"],
      ["old.mp4"],
    ]);
  });

  it("splits at local midnight", () => {
    const groups = groupByDay(
      [clip("late.mp4", at(2026, 9, 9, 0, 0)), clip("early.mp4", at(2026, 9, 8, 23, 59))],
      now,
    );
    expect(groups).toHaveLength(2);
  });
});
