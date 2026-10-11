import { describe, expect, it } from "vitest";
import type { ClipInfo } from "./clips";
import {
  DEFAULT_FOLDER_COLOR,
  FOLDER_COLORS,
  FOLDER_ICONS,
  MAX_FOLDER_NAME_LENGTH,
  clipsIn,
  deleteBreakdown,
  folderColor,
  folderIconId,
  foldersOf,
  looseClips,
  sortFolders,
  validateFolderName,
  type FolderInfo,
} from "./folders";

const folder = (id: string, name: string, extra: Partial<FolderInfo> = {}): FolderInfo => ({
  id,
  name,
  color: "blue",
  icon: "star",
  clipCount: 0,
  sizeBytes: 0,
  createdMs: 0,
  ...extra,
});

const clip = (name: string, folders: string[] = []): ClipInfo => ({
  name,
  path: `/clips/${name}`,
  sizeBytes: 1,
  modifiedMs: 0,
  folders,
  game: null,
});

describe("validateFolderName", () => {
  const existing = [folder("f1", "Valorant"), folder("f2", "Ranked")];

  it("trims and accepts a good name", () => {
    expect(validateFolderName("  Mis jugadas  ", existing)).toEqual({ ok: true, name: "Mis jugadas" });
  });

  it("is a label, not a file name: slashes and colons are fine", () => {
    expect(validateFolderName("Ranked / noche: 2", existing)).toMatchObject({ ok: true });
  });

  it("rejects empty, too long and control characters", () => {
    expect(validateFolderName("   ", existing)).toMatchObject({ ok: false });
    expect(validateFolderName("a".repeat(MAX_FOLDER_NAME_LENGTH + 1), existing)).toMatchObject({ ok: false });
    expect(validateFolderName("a".repeat(MAX_FOLDER_NAME_LENGTH), existing)).toMatchObject({ ok: true });
    expect(validateFolderName("tab\there", existing)).toMatchObject({ ok: false });
  });

  it("counts characters, not UTF-16 units", () => {
    expect(validateFolderName("🎮".repeat(MAX_FOLDER_NAME_LENGTH), existing)).toMatchObject({ ok: true });
  });

  it("refuses a name that is taken, whatever its capitals", () => {
    const result = validateFolderName("valorant", existing);
    expect(result).toMatchObject({ ok: false });
    expect(!result.ok && result.error).toContain("Ya existe");
  });

  it("lets a folder keep its own name, even changing the capitals", () => {
    expect(validateFolderName("VALORANT", existing, "f1")).toMatchObject({ ok: true });
    expect(validateFolderName("Ranked", existing, "f1")).toMatchObject({ ok: false });
  });
});

describe("sortFolders", () => {
  it("sorts by name the Spanish way without mutating the input", () => {
    const input = [folder("f1", "zorro"), folder("f2", "Ñandú"), folder("f3", "árbol"), folder("f4", "Beta")];
    const sorted = sortFolders(input).map((f) => f.name);
    expect(sorted).toEqual(["árbol", "Beta", "Ñandú", "zorro"]);
    expect(input[0].name).toBe("zorro");
  });

  it("breaks ties by id so the order is stable", () => {
    const sorted = sortFolders([folder("f2", "Igual"), folder("f1", "Igual")]);
    expect(sorted.map((f) => f.id)).toEqual(["f1", "f2"]);
  });
});

describe("what is in which folder", () => {
  const clips = [clip("a.mp4", ["f1"]), clip("b.mp4", ["f1", "f2"]), clip("c.mp4"), clip("d.mp4", ["f2"])];

  it("lists a folder's clips in the given order", () => {
    expect(clipsIn(clips, "f1").map((c) => c.name)).toEqual(["a.mp4", "b.mp4"]);
    expect(clipsIn(clips, "f9")).toEqual([]);
  });

  it("finds the clips in no folder", () => {
    expect(looseClips(clips).map((c) => c.name)).toEqual(["c.mp4"]);
  });

  it("lists a clip's folders by name, ignoring ids that no longer exist", () => {
    const folders = [folder("f1", "Zeta"), folder("f2", "Alfa")];
    expect(foldersOf(clips[1], folders).map((f) => f.name)).toEqual(["Alfa", "Zeta"]);
    expect(foldersOf(clip("x.mp4", ["f1", "f77"]), folders).map((f) => f.id)).toEqual(["f1"]);
    expect(foldersOf(clips[2], folders)).toEqual([]);
  });
});

describe("deleteBreakdown", () => {
  it("separates the clips only this folder holds from the shared ones", () => {
    const clips = [clip("a.mp4", ["f1"]), clip("b.mp4", ["f1", "f2"]), clip("c.mp4", ["f2"])];
    expect(deleteBreakdown(folder("f1", "A"), clips)).toEqual({ total: 2, exclusive: 1, shared: 1 });
    expect(deleteBreakdown(folder("f2", "B"), clips)).toEqual({ total: 2, exclusive: 1, shared: 1 });
    expect(deleteBreakdown(folder("f3", "C"), clips)).toEqual({ total: 0, exclusive: 0, shared: 0 });
  });
});

describe("colors and icons", () => {
  it("has a palette with unique ids that includes the default", () => {
    const ids = FOLDER_COLORS.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toContain(DEFAULT_FOLDER_COLOR);
    for (const c of FOLDER_COLORS) expect(c.hex).toMatch(/^#[0-9a-f]{6}$/);
  });

  it("falls back to the default for an id this version doesn't know", () => {
    expect(folderColor("blue")).toBe("#4da3ff");
    expect(folderColor("from-the-future")).toBe(folderColor(DEFAULT_FOLDER_COLOR));
    expect(folderIconId("trophy")).toBe("trophy");
    expect(folderIconId("from-the-future")).toBe("folder");
  });

  it("uses ids Rust accepts (lowercase letters and dashes, 16 at most)", () => {
    for (const id of [...FOLDER_COLORS.map((c) => c.id), ...FOLDER_ICONS.map((i) => i.id)]) {
      expect(id).toMatch(/^[a-z-]{1,16}$/);
    }
  });
});
