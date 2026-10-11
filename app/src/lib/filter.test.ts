import { describe, expect, it } from "vitest";
import type { ClipInfo } from "./clips";
import {
  NONE,
  applyFilter,
  emptyFilter,
  gameKey,
  gameOptions,
  isFiltering,
  layoutFor,
  normalizeText,
  rangeStart,
  validateGameName,
  type ClipFilter,
} from "./filter";
import type { FolderInfo } from "./folders";

const at = (y: number, m: number, d: number, h = 12) => new Date(y, m, d, h).getTime();
const NOW = at(2026, 9, 11, 15); // Sunday 11 October 2026, 15:00

const clip = (name: string, extra: Partial<ClipInfo> = {}): ClipInfo => ({
  name: `${name}.mp4`,
  path: `/clips/${name}.mp4`,
  sizeBytes: 1000,
  modifiedMs: NOW - 1000,
  folders: [],
  game: null,
  ...extra,
});

const folder = (id: string, name: string): FolderInfo => ({
  id,
  name,
  color: "blue",
  icon: "star",
  clipCount: 0,
  sizeBytes: 0,
  createdMs: 0,
});

const filter = (extra: Partial<ClipFilter>): ClipFilter => ({ ...emptyFilter(), ...extra });
const names = (clips: ClipInfo[]) => clips.map((c) => c.name.replace(/\.mp4$/, ""));

describe("isFiltering", () => {
  it("an empty filter hides nothing, and the order is not a filter", () => {
    expect(isFiltering(emptyFilter())).toBe(false);
    expect(isFiltering(filter({ sort: "size" }))).toBe(false);
    expect(isFiltering(filter({ query: "   " }))).toBe(false);
  });

  it("any narrowing counts", () => {
    expect(isFiltering(filter({ query: "ace" }))).toBe(true);
    expect(isFiltering(filter({ game: "valorant" }))).toBe(true);
    expect(isFiltering(filter({ game: NONE }))).toBe(true);
    expect(isFiltering(filter({ folder: "f1" }))).toBe(true);
    expect(isFiltering(filter({ range: "week" }))).toBe(true);
  });
});

describe("search", () => {
  const folders = [folder("f1", "Mejores jugadas"), folder("f2", "Águilas")];
  const clips = [
    clip("Ace en Mirage", { game: "Counter-Strike 2", folders: ["f1"] }),
    clip("clip-20261010-221500", { game: "Valorant" }),
    clip("Clutch 1v4", { game: "Valorant", folders: ["f1", "f2"] }),
    clip("sin-datos"),
  ];
  const find = (query: string) => names(applyFilter(clips, folders, filter({ query }), NOW));

  it("looks at the title", () => {
    expect(find("mirage")).toEqual(["Ace en Mirage"]);
  });

  it("looks at the game", () => {
    expect(find("valorant")).toEqual(["Clutch 1v4", "clip-20261010-221500"]);
  });

  it("looks at the names of the clip's folders, which are its tags", () => {
    expect(find("mejores")).toEqual(["Clutch 1v4", "Ace en Mirage"]);
  });

  it("ignores capitals and accents on both sides", () => {
    expect(find("AGUILAS")).toEqual(["Clutch 1v4"]);
    expect(find("águilas")).toEqual(["Clutch 1v4"]);
    expect(normalizeText("Ñandú Ácido")).toBe("nandu acido");
  });

  it("every word has to match, in any order and in different places", () => {
    expect(find("valorant clutch")).toEqual(["Clutch 1v4"]);
    expect(find("1v4 valorant")).toEqual(["Clutch 1v4"]);
    expect(find("valorant mirage")).toEqual([]);
  });

  it("extra spaces and an empty query change nothing", () => {
    expect(find("   mirage   ")).toEqual(["Ace en Mirage"]);
    expect(find("")).toHaveLength(4);
  });

  it("does not match the extension or a folder the clip isn't in", () => {
    expect(find("mp4")).toEqual([]);
    expect(find("aguilas mirage")).toEqual([]);
  });
});

describe("the game filter", () => {
  const clips = [
    clip("a", { game: "Valorant" }),
    clip("b", { game: "VALORANT" }),
    clip("c", { game: "Fortnite" }),
    clip("d"),
  ];

  it("matches a game whatever its capitals", () => {
    expect(names(applyFilter(clips, [], filter({ game: gameKey("Valorant") }), NOW)).sort()).toEqual(["a", "b"]);
  });

  it("can ask for the clips with no game", () => {
    expect(names(applyFilter(clips, [], filter({ game: NONE }), NOW))).toEqual(["d"]);
  });
});

describe("the folder filter", () => {
  const clips = [clip("a", { folders: ["f1"] }), clip("b", { folders: ["f1", "f2"] }), clip("c")];

  it("matches the clips in a folder, including those that are in others too", () => {
    expect(names(applyFilter(clips, [], filter({ folder: "f1" }), NOW)).sort()).toEqual(["a", "b"]);
    expect(names(applyFilter(clips, [], filter({ folder: "f2" }), NOW))).toEqual(["b"]);
  });

  it("can ask for the clips in no folder", () => {
    expect(names(applyFilter(clips, [], filter({ folder: NONE }), NOW))).toEqual(["c"]);
  });
});

describe("the date filter", () => {
  it("starts each range at the beginning of a local day", () => {
    expect(rangeStart("all", NOW)).toBeNull();
    expect(rangeStart("today", NOW)).toBe(at(2026, 9, 11, 0));
    expect(rangeStart("week", NOW)).toBe(at(2026, 9, 5, 0)); // today + the 6 days before
    expect(rangeStart("month", NOW)).toBe(at(2026, 8, 12, 0)); // 30 days including today
    expect(rangeStart("year", NOW)).toBe(at(2026, 0, 1, 0));
  });

  const clips = [
    clip("today", { modifiedMs: at(2026, 9, 11, 9) }),
    clip("yesterday", { modifiedMs: at(2026, 9, 10, 23) }),
    clip("six-days", { modifiedMs: at(2026, 9, 5, 0) }),
    clip("a-week-and-a-day", { modifiedMs: at(2026, 9, 4, 23) }),
    clip("september", { modifiedMs: at(2026, 8, 12, 0) }),
    clip("last-year", { modifiedMs: at(2025, 11, 31, 23) }),
  ];

  it("keeps what is inside the range, edges included", () => {
    const pick = (range: ClipFilter["range"]) => names(applyFilter(clips, [], filter({ range }), NOW));
    expect(pick("today")).toEqual(["today"]);
    expect(pick("week")).toEqual(["today", "yesterday", "six-days"]);
    expect(pick("month")).toEqual(["today", "yesterday", "six-days", "a-week-and-a-day", "september"]);
    expect(pick("year")).toHaveLength(5);
    expect(pick("all")).toHaveLength(6);
  });

  it("works across a year boundary", () => {
    const january = at(2027, 0, 3, 10);
    expect(rangeStart("week", january)).toBe(at(2026, 11, 28, 0));
  });
});

describe("filters combine", () => {
  it("a clip has to pass all of them", () => {
    const clips = [
      clip("a", { game: "Valorant", folders: ["f1"], modifiedMs: NOW - 1000 }),
      clip("b", { game: "Valorant", folders: [], modifiedMs: NOW - 1000 }),
      clip("c", { game: "Valorant", folders: ["f1"], modifiedMs: at(2026, 0, 2) }),
    ];
    const f = filter({ game: "valorant", folder: "f1", range: "week", query: "a" });
    expect(names(applyFilter(clips, [folder("f1", "X")], f, NOW))).toEqual(["a"]);
  });
});

describe("sorting", () => {
  const clips = [
    clip("clip-10", { modifiedMs: 300, sizeBytes: 50 }),
    clip("clip-2", { modifiedMs: 100, sizeBytes: 900 }),
    clip("Álamo", { modifiedMs: 200, sizeBytes: 500 }),
  ];
  const sorted = (sort: ClipFilter["sort"]) => names(applyFilter(clips, [], filter({ sort }), NOW));

  it("newest and oldest go by date", () => {
    expect(sorted("newest")).toEqual(["clip-10", "Álamo", "clip-2"]);
    expect(sorted("oldest")).toEqual(["clip-2", "Álamo", "clip-10"]);
  });

  it("by name is alphabetical for Spanish, with numbers in numeric order", () => {
    expect(sorted("name")).toEqual(["Álamo", "clip-2", "clip-10"]);
  });

  it("by size puts the heaviest first", () => {
    expect(sorted("size")).toEqual(["clip-2", "Álamo", "clip-10"]);
  });

  it("ties break by name so the order is stable", () => {
    const tied = [clip("b", { modifiedMs: 5 }), clip("a", { modifiedMs: 5 })];
    expect(names(applyFilter(tied, [], filter({ sort: "newest" }), NOW))).toEqual(["b", "a"]);
    expect(names(applyFilter(tied, [], filter({ sort: "oldest" }), NOW))).toEqual(["a", "b"]);
  });

  it("does not change the list it was given", () => {
    const input = [clip("b"), clip("a")];
    applyFilter(input, [], filter({ sort: "name" }), NOW);
    expect(names(input)).toEqual(["b", "a"]);
  });

  it("groups by day only for date orders", () => {
    expect(layoutFor("newest")).toBe("by-day");
    expect(layoutFor("oldest")).toBe("by-day");
    expect(layoutFor("name")).toBe("flat");
    expect(layoutFor("size")).toBe("flat");
  });
});

describe("gameOptions", () => {
  it("counts each game once whatever its capitals, most used first", () => {
    const { games, withoutGame } = gameOptions([
      clip("a", { game: "Fortnite" }),
      clip("b", { game: "Valorant" }),
      clip("c", { game: "VALORANT" }),
      clip("d"),
      clip("e"),
      clip("f"),
    ]);
    expect(games).toEqual([
      { key: "valorant", name: "Valorant", count: 2 },
      { key: "fortnite", name: "Fortnite", count: 1 },
    ]);
    expect(withoutGame).toBe(3);
  });

  it("ties go alphabetical for Spanish", () => {
    const { games } = gameOptions([clip("a", { game: "Zelda" }), clip("b", { game: "Ápex" })]);
    expect(games.map((g) => g.name)).toEqual(["Ápex", "Zelda"]);
  });

  it("is empty for no clips", () => {
    expect(gameOptions([])).toEqual({ games: [], withoutGame: 0 });
  });
});

describe("validateGameName", () => {
  it("trims and accepts a name", () => {
    expect(validateGameName("  Counter-Strike 2 ")).toEqual({ ok: true, name: "Counter-Strike 2" });
  });

  it("refuses empty, too long and control characters", () => {
    expect(validateGameName("   ")).toMatchObject({ ok: false });
    expect(validateGameName("a".repeat(61))).toMatchObject({ ok: false });
    expect(validateGameName("a".repeat(60))).toMatchObject({ ok: true });
    expect(validateGameName("tab\there")).toMatchObject({ ok: false });
  });
});
