// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipInfo } from "../lib/clips";
import type { FolderInfo } from "../lib/folders";
import { chooseOption, dropdownOf } from "./dropdown.testing";

const api = vi.hoisted(() => ({
  listClips: vi.fn(),
  listFolders: vi.fn(),
  toggleBuffer: vi.fn(),
  saveClip: vi.fn(),
  openClipsFolder: vi.fn(),
  renameClip: vi.fn(),
  deleteClip: vi.fn(),
  revealClip: vi.fn(),
  copyClip: vi.fn(),
  takeScreenshot: vi.fn(),
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
  kind: "clip",
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
  vi.mocked((await import("./player")).openPlayer).mockClear();
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

describe("screenshots", () => {
  const shot = (n: number, extra: Partial<ClipInfo> = {}): ClipInfo => ({
    ...clip(n),
    name: `captura-${n}.png`,
    kind: "screenshot",
    path: `/clips/captura-${n}.png`,
    ...extra,
  });
  const labelsOf = (card: Element) =>
    [...card.querySelectorAll<HTMLElement>(".card-actions button")].map((b) => b.getAttribute("aria-label"));

  async function show(...items: ClipInfo[]) {
    api.listClips.mockResolvedValue(items);
    const gallery = createGallery(root());
    await gallery.refresh();
    return [...root().querySelectorAll<HTMLElement>(".card")];
  }

  it("take a picture from the button in the header", async () => {
    await show();
    const button = [...root().querySelectorAll<HTMLButtonElement>(".view-header .btn")].find(
      (b) => b.textContent?.includes("Captura"),
    )!;
    button.click();
    expect(api.takeScreenshot).toHaveBeenCalledTimes(1);
  });

  it("are shown as the picture itself, not as a video", async () => {
    const [card] = await show(shot(1));
    const image = card.querySelector<HTMLImageElement>("img.thumb-image")!;
    expect(image.getAttribute("src")).toBe("asset:///clips/captura-1.png");
    expect(card.querySelector("video")).toBeNull();
    expect(card.querySelector(".thumb-play")).toBeNull();
    expect(card.querySelector(".card-title")!.textContent).toBe("captura-1");
  });

  it("say their size once the picture has loaded", async () => {
    const [card] = await show(shot(1));
    expect(card.querySelector(".badge")!.textContent).toBe("PNG");
    const image = card.querySelector<HTMLImageElement>("img")!;
    Object.defineProperty(image, "naturalWidth", { value: 2940 });
    Object.defineProperty(image, "naturalHeight", { value: 1912 });
    image.dispatchEvent(new Event("load"));
    expect(card.querySelector(".badge")!.textContent).toBe("2940×1912");
  });

  it("can't be trimmed, but can be renamed, filed, copied, shown and deleted", async () => {
    const [picture, video] = await show(shot(1), clip(2));
    expect(labelsOf(picture)).toEqual([
      "Renombrar",
      "Carpetas…",
      "Copiar la captura para pegarla en un chat o un mail",
      "Mostrar el archivo en el disco",
      "Borrar",
    ]);
    expect(labelsOf(video)).toContain("Recortar y editar");
  });

  it("open in the viewer, not the editor", async () => {
    const player = await import("./player");
    const [picture] = await show(shot(1));
    picture.click();
    expect(player.openPlayer).toHaveBeenCalledWith(expect.objectContaining({ name: "captura-1.png" }));
  });

  it("copy the picture and say so", async () => {
    api.copyClip.mockResolvedValue(undefined);
    const [picture] = await show(shot(1));
    picture
      .querySelector<HTMLButtonElement>('[aria-label^="Copiar la captura"]')!
      .click();
    await settle();
    expect(api.copyClip).toHaveBeenCalledWith("captura-1.png");
    expect(document.getElementById("toasts")!.textContent).toContain("Copiado");
  });

  it("are counted next to the clips in the header", async () => {
    await show(clip(1), clip(2), shot(3));
    expect(root().querySelector(".view-header p")!.textContent).toBe("2 clips y 1 captura");
  });

  it("can be told apart with the type filter, which appears with the first one", async () => {
    await show(clip(1));
    expect(dropdownOf(root(), "Tipo")).toSatisfy((el: HTMLElement | null) => el === null || el.hidden);
    document.body.innerHTML = '<div id="toasts"></div><section id="root"></section>';
    const cards = await show(clip(1), shot(2), shot(3));
    expect(cards).toHaveLength(3);
    const select = dropdownOf(root(), "Tipo")!;
    expect(select.hidden).toBe(false);
    chooseOption(select, "screenshot");
    const names = [...root().querySelectorAll(".card-title")].map((t) => t.textContent);
    expect(names.sort()).toEqual(["captura-2", "captura-3"]);
    expect(root().querySelector(".filter-status")!.textContent).toBe("2 de 3 elementos");
  });
});

describe("copying a clip to paste it elsewhere", () => {
  const MB = 1024 * 1024;
  const copyButton = (card: Element) =>
    card.querySelector<HTMLButtonElement>(
      'button[aria-label="Copiar el clip para pegarlo en un chat o un mail"]',
    )!;
  const toasts = () => document.getElementById("toasts")!.textContent;

  async function withClips(...clips: ClipInfo[]) {
    api.listClips.mockResolvedValue(clips);
    const gallery = createGallery(root());
    await gallery.refresh();
    return [...root().querySelectorAll(".card")];
  }

  it("every card can be copied, and it asks for that clip", async () => {
    api.copyClip.mockResolvedValue(undefined);
    const cards = await withClips(clip(1), clip(2));
    expect(cards).toHaveLength(2);
    copyButton(cards[1]).click();
    await settle();
    expect(api.copyClip).toHaveBeenCalledTimes(1);
    expect(api.copyClip).toHaveBeenCalledWith(cards[1].querySelector(".card-title")!.getAttribute("title"));
  });

  it("confirms, and does not open the player", async () => {
    api.copyClip.mockResolvedValue(undefined);
    const player = await import("./player");
    const [card] = await withClips(clip(1));
    copyButton(card).click();
    await settle();
    expect(toasts()).toContain("Copiado: pegalo en el chat o en un mail");
    expect(player.openPlayer).not.toHaveBeenCalled();
  });

  it("warns when the clip is too big for a chat, but it was copied", async () => {
    api.copyClip.mockResolvedValue(undefined);
    const [card] = await withClips({ ...clip(1), sizeBytes: 30 * MB });
    copyButton(card).click();
    await settle();
    expect(toasts()).toContain("Copiado, pero pesa 30,0 MB");
    expect(toasts()).toContain("Discord sin Nitro (20 MB)");
    expect(document.querySelector(".toast-error")).toBeNull();
  });

  it("a clip that can't be copied says why", async () => {
    api.copyClip.mockRejectedValue("el clip ya no existe en el disco");
    const [card] = await withClips(clip(1));
    copyButton(card).click();
    await settle();
    expect(document.querySelector(".toast-error")?.textContent).toContain("ya no existe");
    expect(toasts()).not.toContain("Copiado");
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
  const select = (label: string) => dropdownOf(root(), label)!;

  function type(text: string) {
    search().value = text;
    search().dispatchEvent(new Event("input", { bubbles: true }));
  }

  const choose = chooseOption;

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
