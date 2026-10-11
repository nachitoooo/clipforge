// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipInfo } from "../lib/clips";
import type { FolderInfo } from "../lib/folders";

const api = vi.hoisted(() => ({
  listClips: vi.fn(),
  listFolders: vi.fn(),
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
const picker = vi.hoisted(() => ({ openFolderPicker: vi.fn() }));
vi.mock("./folderpicker", () => picker);

const clip = (n: number): ClipInfo => ({
  name: `clip-${n}.mp4`,
  path: `/clips/clip-${n}.mp4`,
  sizeBytes: 1000 * n,
  modifiedMs: Date.now() - n * 1000,
  folders: [],
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
  api.listFolders.mockResolvedValue([]);
  picker.openFolderPicker.mockReset();
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

describe("the folders of each clip", () => {
  const chips = (card: Element) => [...card.querySelectorAll(".chip")].map((c) => c.textContent);

  it("labels a clip with the folders it is in", async () => {
    api.listClips.mockResolvedValue([{ ...clip(1), folders: ["f2", "f1"] }, clip(2)]);
    api.listFolders.mockResolvedValue([folder("f1", "Zorros"), folder("f2", "Águilas")]);
    const gallery = createGallery(root());
    await gallery.refresh();
    const cards = [...root().querySelectorAll(".card")];
    expect(chips(cards[0])).toEqual(["Águilas", "Zorros"]);
    expect(chips(cards[1])).toEqual([]);
  });

  it("shows two labels and counts the rest", async () => {
    api.listClips.mockResolvedValue([{ ...clip(1), folders: ["f1", "f2", "f3", "f4"] }]);
    api.listFolders.mockResolvedValue([
      folder("f1", "A"),
      folder("f2", "B"),
      folder("f3", "C"),
      folder("f4", "D"),
    ]);
    const gallery = createGallery(root());
    await gallery.refresh();
    const card = root().querySelector(".card")!;
    expect(chips(card)).toEqual(["A", "B", "+2"]);
    expect(card.querySelector(".chip-more")!.getAttribute("title")).toBe("C, D");
  });

  it("ignores a folder that no longer exists", async () => {
    api.listClips.mockResolvedValue([{ ...clip(1), folders: ["f99"] }]);
    const gallery = createGallery(root());
    await gallery.refresh();
    expect(root().querySelector(".chip")).toBeNull();
  });

  it("has a 'Carpetas…' button that opens the picker for that clip", async () => {
    api.listClips.mockResolvedValue([clip(1), clip(2)]);
    const gallery = createGallery(root());
    await gallery.refresh();
    const button = root().querySelectorAll(".card")[1].querySelector('[aria-label="Carpetas…"]') as HTMLElement;
    button.click();
    expect(picker.openFolderPicker).toHaveBeenCalledTimes(1);
    expect(picker.openFolderPicker.mock.calls[0][0].name).toBe("clip-2.mp4");
  });

  it("refreshes after the picker saved", async () => {
    api.listClips.mockResolvedValue([clip(1)]);
    const gallery = createGallery(root());
    await gallery.refresh();
    (root().querySelector('[aria-label="Carpetas…"]') as HTMLElement).click();
    const onSaved = picker.openFolderPicker.mock.calls[0][1] as () => void;
    api.listClips.mockResolvedValue([{ ...clip(1), folders: ["f1"] }]);
    api.listFolders.mockResolvedValue([folder("f1", "Nueva")]);
    onSaved();
    await settle();
    expect(root().querySelector(".chip")!.textContent).toBe("Nueva");
  });

  it("the cards in Recientes are not draggable", async () => {
    api.listClips.mockResolvedValue([clip(1)]);
    const gallery = createGallery(root());
    await gallery.refresh();
    expect(root().querySelector(".card")!.getAttribute("draggable")).toBeNull();
  });
});
