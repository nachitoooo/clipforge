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
const gameDialog = vi.hoisted(() => ({ openGameDialog: vi.fn() }));
vi.mock("./gamedialog", () => gameDialog);

const clip = (n: number): ClipInfo => ({
  name: `clip-${n}.mp4`,
  path: `/clips/clip-${n}.mp4`,
  sizeBytes: 1000 * n,
  modifiedMs: Date.now() - n * 1000,
  folders: [],
  game: null,
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
  gameDialog.openGameDialog.mockReset();
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

  const saveButton = () =>
    root().querySelector<HTMLButtonElement>(".view-header .btn-primary")!;

  it("the button says how much it saves, and follows the length in Settings", async () => {
    const gallery = createGallery(root());
    await gallery.refresh();
    expect(saveButton().title).toBe("Guardar los últimos 30 s");

    gallery.setSaveShortcut("CommandOrControl+Shift+S", 90);
    expect(saveButton().title).toBe("Guardar los últimos 1 min 30 s");
    expect(headerKey()).toBe("⇧⌘S");

    // A new shortcut alone keeps the length it already had.
    gallery.setSaveShortcut("Alt+F9");
    expect(saveButton().title).toBe("Guardar los últimos 1 min 30 s");
    expect(headerKey()).toBe("⌥F9");
  });

  it("a new length alone redraws the header", async () => {
    const gallery = createGallery(root());
    await gallery.refresh();
    const header = root().querySelector(".view-header");
    gallery.setSaveShortcut("CommandOrControl+Shift+S", 30);
    expect(root().querySelector(".view-header")).toBe(header);
    gallery.setSaveShortcut("CommandOrControl+Shift+S", 120);
    expect(saveButton().title).toBe("Guardar los últimos 2 min");
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

describe("searching and filtering Recientes", () => {
  const library = [
    { ...clip(1), game: "Valorant", name: "Ace en Mirage.mp4" },
    { ...clip(2), game: "Valorant" },
    { ...clip(3), game: "Fortnite" },
    { ...clip(4) },
  ];
  const bar = () => root().querySelector(".filterbar") as HTMLElement | null;
  const search = () => root().querySelector<HTMLInputElement>(".filterbar input")!;
  const titles = () => [...root().querySelectorAll(".card .card-title")].map((t) => t.textContent);
  const select = (label: string) => root().querySelector<HTMLSelectElement>(`select[aria-label="${label}"]`)!;

  function type(text: string) {
    search().value = text;
    search().dispatchEvent(new Event("input", { bubbles: true }));
  }

  function choose(el: HTMLSelectElement, value: string) {
    el.value = value;
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }

  async function open(clips: ClipInfo[] = library) {
    api.listClips.mockResolvedValue(clips);
    const gallery = createGallery(root());
    await gallery.refresh();
    return gallery;
  }

  it("shows the bar when there are clips and not when there are none", async () => {
    await open([]);
    expect(bar()).toBeNull();
    await open();
    expect(bar()).not.toBeNull();
  });

  it("typing narrows the list at once and counts what is left", async () => {
    await open();
    expect(titles()).toHaveLength(4);
    type("mirage");
    expect(titles()).toEqual(["Ace en Mirage"]);
    expect(root().querySelector(".filter-status")!.textContent).toBe("1 de 4 clips");
    expect(root().querySelector(".view-header p")!.textContent).toBe("4 clips"); // the header counts them all
  });

  it("searches by game too, and filters by it", async () => {
    await open();
    type("fortnite");
    expect(titles()).toEqual(["clip-3"]);
    type("");
    choose(select("Juego"), "valorant");
    expect(titles()).toHaveLength(2);
  });

  it("keeps the cards it already had instead of loading every thumbnail again", async () => {
    await open();
    const before = new Map([...root().querySelectorAll(".card")].map((c) => [c.querySelector(".card-title")!.textContent, c]));
    type("clip");
    const after = [...root().querySelectorAll(".card")];
    expect(after.length).toBe(3);
    for (const card of after) expect(before.get(card.querySelector(".card-title")!.textContent)).toBe(card);
    type("");
    for (const card of root().querySelectorAll(".card")) {
      expect(before.get(card.querySelector(".card-title")!.textContent)).toBe(card);
    }
  });

  it("an order by name is one list; by date it is grouped by day", async () => {
    await open();
    expect(root().querySelectorAll(".group-title").length).toBeGreaterThan(0);
    choose(select("Ordenar por"), "name");
    expect(root().querySelectorAll(".group-title")).toHaveLength(0);
    expect(titles()).toEqual(["Ace en Mirage", "clip-2", "clip-3", "clip-4"]);
    choose(select("Ordenar por"), "newest");
    expect(root().querySelectorAll(".group-title").length).toBeGreaterThan(0);
  });

  it("says nothing matches and offers to clear", async () => {
    await open();
    type("zzz");
    expect(root().querySelector(".empty h2")!.textContent).toBe("Ningún clip coincide");
    expect(bar()).not.toBeNull(); // the way back stays on screen
    (root().querySelector(".empty .btn") as HTMLButtonElement).click();
    expect(titles()).toHaveLength(4);
    expect(search().value).toBe("");
  });

  it("the filter survives a reload of the list, and so does the focus in the search box", async () => {
    const gallery = await open();
    search().focus();
    type("clip");
    api.listClips.mockResolvedValue([...library, { ...clip(5) }]);
    await gallery.refresh();
    expect(search().value).toBe("clip");
    expect(titles()).toHaveLength(4); // clip-2, 3, 4, 5
    expect(document.activeElement).toBe(search());
  });

  it("drops a game filter when its last clip goes away", async () => {
    const gallery = await open();
    choose(select("Juego"), "fortnite");
    expect(titles()).toEqual(["clip-3"]);
    api.listClips.mockResolvedValue(library.filter((c) => c.game !== "Fortnite"));
    await gallery.refresh();
    expect(titles()).toHaveLength(3);
  });

  it("offers the folders as a filter", async () => {
    api.listFolders.mockResolvedValue([folder("f1", "Mejores")]);
    await open([{ ...clip(1), folders: ["f1"] }, clip(2)]);
    choose(select("Carpeta"), "f1");
    expect(titles()).toEqual(["clip-1"]);
    choose(select("Carpeta"), "__none__");
    expect(titles()).toEqual(["clip-2"]);
  });

  it("each card names its game, or says it has none", async () => {
    await open();
    const links = [...root().querySelectorAll(".card .game-link")].map((l) => l.textContent);
    expect(links).toEqual(["Valorant", "Valorant", "Fortnite", "Sin juego"]);
    expect(root().querySelectorAll(".game-none")).toHaveLength(1);
  });

  it("clicking the game opens the game dialog with what it needs, without playing the clip", async () => {
    await open();
    (root().querySelector(".card .game-link") as HTMLElement).click();
    expect(gameDialog.openGameDialog).toHaveBeenCalledTimes(1);
    const options = gameDialog.openGameDialog.mock.calls[0][0];
    expect(options.clip.name).toBe("Ace en Mirage.mp4");
    expect(options.games).toEqual(["Valorant", "Fortnite"]);
    expect(options.clipsOfGame("valorant")).toBe(2);
    expect(options.clipsOfGame("Nada")).toBe(0);
  });

  it("reloads after the game was changed", async () => {
    await open();
    (root().querySelector(".card .game-link") as HTMLElement).click();
    api.listClips.mockResolvedValue([{ ...library[0], game: "Counter-Strike 2" }, library[1]]);
    gameDialog.openGameDialog.mock.calls[0][0].onSaved();
    await settle();
    expect(root().querySelector(".card .game-link")!.textContent).toBe("Counter-Strike 2");
  });
});

describe("renaming a clip does not leave a stale card behind", () => {
  it("a cancelled rename shows the title again, not the old text box", async () => {
    api.listClips.mockResolvedValue([clip(1)]);
    const gallery = createGallery(root());
    await gallery.refresh();
    (root().querySelector('[aria-label="Renombrar"]') as HTMLElement).click();
    const input = root().querySelector(".rename-input") as HTMLInputElement;
    expect(input).not.toBeNull();
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    expect(root().querySelector(".rename-input")).toBeNull();
    expect(root().querySelector(".card-title")!.textContent).toBe("clip-1");
  });

  it("a rename that went through shows the new name", async () => {
    api.listClips.mockResolvedValue([clip(1)]);
    api.renameClip.mockResolvedValue("nuevo.mp4");
    const gallery = createGallery(root());
    await gallery.refresh();
    (root().querySelector('[aria-label="Renombrar"]') as HTMLElement).click();
    const input = root().querySelector(".rename-input") as HTMLInputElement;
    input.value = "nuevo";
    api.listClips.mockResolvedValue([{ ...clip(1), name: "nuevo.mp4", path: "/clips/nuevo.mp4" }]);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
    expect(api.renameClip).toHaveBeenCalledWith("clip-1.mp4", "nuevo");
    expect(root().querySelector(".card-title")!.textContent).toBe("nuevo");
  });
});
