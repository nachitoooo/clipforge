// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipInfo } from "../lib/clips";

const api = vi.hoisted(() => ({
  listClips: vi.fn(),
  toggleBuffer: vi.fn(),
  saveClip: vi.fn(),
  openClipsFolder: vi.fn(),
  renameClip: vi.fn(),
  deleteClip: vi.fn(),
  revealClip: vi.fn(),
  clipSrc: vi.fn((clip: { path: string }) => `asset://${clip.path}`),
}));
vi.mock("../api", () => api);
vi.mock("./editor", () => ({ openEditor: vi.fn() }));
vi.mock("./player", () => ({ openPlayer: vi.fn() }));

const clip = (n: number): ClipInfo => ({
  name: `clip-${n}.mp4`,
  path: `/clips/clip-${n}.mp4`,
  sizeBytes: 1000 * n,
  modifiedMs: Date.now() - n * 1000,
});

const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

type GalleryModule = typeof import("./gallery");
let createGallery: GalleryModule["createGallery"];

beforeEach(async () => {
  document.body.innerHTML = '<div id="toasts"></div><section id="root"></section>';
  vi.stubGlobal("IntersectionObserver", class {
    observe() {}
    unobserve() {}
    disconnect() {}
  });
  vi.stubGlobal("navigator", { userAgent: "Macintosh" });
  for (const fn of Object.values(api)) if ("mockReset" in fn && fn !== api.clipSrc) fn.mockReset();
  api.listClips.mockResolvedValue([]);
  vi.resetModules();
  ({ createGallery } = (await import("./gallery")) as GalleryModule);
});

const root = () => document.getElementById("root")!;
const headerKey = () => root().querySelector(".btn-primary kbd")?.textContent;
const emptyKey = () => root().querySelector(".empty kbd")?.textContent;

describe("the save-clip shortcut shown in the gallery", () => {
  it("starts with the usual shortcut", async () => {
    const gallery = createGallery(root());
    await gallery.refresh();
    expect(headerKey()).toBe("⇧⌘S");
    expect(emptyKey()).toBe("⇧⌘S");
  });

  it("shows a changed shortcut in the button and in the empty state", async () => {
    const gallery = createGallery(root());
    await gallery.refresh();
    gallery.setSaveShortcut("Alt+F9");
    expect(headerKey()).toBe("⌥F9");
    expect(emptyKey()).toBe("⌥F9");
    gallery.setSaveShortcut("CommandOrControl+Shift+S");
    expect(headerKey()).toBe("⇧⌘S");
  });

  it("uses Ctrl/Alt names outside macOS", async () => {
    vi.stubGlobal("navigator", { userAgent: "Windows NT 10.0" });
    const gallery = createGallery(root());
    await gallery.refresh();
    expect(headerKey()).toBe("Ctrl+Shift+S");
    gallery.setSaveShortcut("Control+Alt+K");
    expect(headerKey()).toBe("Ctrl+Alt+K");
  });

  it("updates the header without rebuilding the clip cards", async () => {
    api.listClips.mockResolvedValue([clip(1), clip(2)]);
    const gallery = createGallery(root());
    await gallery.refresh();
    const cards = [...root().querySelectorAll(".card")];
    expect(cards).toHaveLength(2);
    expect(root().querySelector(".empty")).toBeNull();

    gallery.setSaveShortcut("Alt+F9");
    expect(headerKey()).toBe("⌥F9");
    const after = [...root().querySelectorAll(".card")];
    expect(after.every((card, i) => card === cards[i])).toBe(true); // same elements: no thumbnails reloaded
  });

  it("asking for the same shortcut again changes nothing", async () => {
    const gallery = createGallery(root());
    await gallery.refresh();
    const header = root().querySelector(".view-header");
    gallery.setSaveShortcut("CommandOrControl+Shift+S");
    expect(root().querySelector(".view-header")).toBe(header);
  });

  it("works before the clips have loaded", async () => {
    const gallery = createGallery(root());
    gallery.setSaveShortcut("Alt+F9");
    await gallery.refresh();
    expect(headerKey()).toBe("⌥F9");
    await settle();
  });
});
