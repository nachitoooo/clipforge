// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipInfo } from "../lib/clips";
import type { FolderInfo } from "../lib/folders";

const api = vi.hoisted(() => ({
  listClips: vi.fn(),
  listFolders: vi.fn(),
  createFolder: vi.fn(),
  updateFolder: vi.fn(),
  deleteFolder: vi.fn(),
  setClipFolders: vi.fn(),
  addToFolder: vi.fn(),
  removeFromFolder: vi.fn(),
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

const clip = (n: number, folders: string[] = []): ClipInfo => ({
  name: `clip-${n}.mp4`,
  path: `/clips/clip-${n}.mp4`,
  sizeBytes: 1_000_000 * n,
  modifiedMs: Date.now() - n * 1000,
  folders,
});

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

const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

type LibraryModule = typeof import("./library");
let createLibrary: LibraryModule["createLibrary"];

beforeEach(async () => {
  document.body.innerHTML = '<div id="toasts"></div><section id="root"></section>';
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  for (const fn of Object.values(api)) if ("mockReset" in fn && fn !== api.clipSrc) fn.mockReset();
  picker.openFolderPicker.mockReset();
  api.listClips.mockResolvedValue([]);
  api.listFolders.mockResolvedValue([]);
  api.createFolder.mockResolvedValue(folder("f9", "Nueva"));
  api.updateFolder.mockResolvedValue(folder("f1", "x"));
  api.deleteFolder.mockResolvedValue(0);
  api.addToFolder.mockResolvedValue(1);
  api.removeFromFolder.mockResolvedValue(1);
  vi.resetModules();
  ({ createLibrary } = (await import("./library")) as LibraryModule);
});

// A dialog a test left open keeps its Escape listener on `document`, and it
// would swallow the Escape of a later test: close whatever is still open.
afterEach(() => {
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
});

const root = () => document.getElementById("root")!;
const q = <T extends Element = HTMLElement>(selector: string, from: ParentNode = document) =>
  from.querySelector(selector) as T | null;
const all = (selector: string, from: ParentNode = document) => [...from.querySelectorAll(selector)] as HTMLElement[];
const click = (el: Element | null) => (el as HTMLElement).click();
const folderCards = () => all(".folder-card", root());
const folderCard = (id: string) => q(`.folder-card[data-folder="${id}"]`, root())!;
const action = (card: Element, label: string) => q(`[aria-label="${label}"]`, card);
const looseCards = () => all('[data-section="loose"] .card', root());
const toastText = () => all(".toast").map((t) => t.textContent);

async function open(clips: ClipInfo[], folders: FolderInfo[]) {
  api.listClips.mockResolvedValue(clips);
  api.listFolders.mockResolvedValue(folders);
  const library = createLibrary(root());
  await library.refresh();
  return library;
}

/** Type into a text input the way a person does. */
function type(input: HTMLInputElement, text: string) {
  input.value = text;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("the Biblioteca root", () => {
  it("invites you to make the first folder when there is nothing yet", async () => {
    await open([], []);
    expect(q("h1", root())!.textContent).toBe("Biblioteca");
    expect(root().textContent).toContain("0 carpetas");
    expect(root().textContent).toContain("Crear la primera carpeta");
    expect(root().textContent).toContain("Todavía no hay clips");
    expect(folderCards()).toHaveLength(0);
  });

  it("shows the folders sorted by name and only the clips that are in none", async () => {
    await open(
      [clip(1, ["f1"]), clip(2), clip(3, ["f1", "f2"]), clip(4)],
      [
        folder("f2", "Zorros", { clipCount: 1, sizeBytes: 3_000_000 }),
        folder("f1", "Ánimo", { clipCount: 2, sizeBytes: 4_000_000 }),
      ],
    );
    expect(folderCards().map((c) => q(".card-title", c)!.textContent)).toEqual(["Ánimo", "Zorros"]);
    expect(q(".card-meta", folderCard("f1"))!.textContent).toContain("2 clips");
    expect(looseCards().map((c) => q(".card-title", c)!.textContent)).toEqual(["clip-2", "clip-4"]);
    expect(root().textContent).toContain("Sin carpeta · 2");
    expect(root().textContent).toContain("2 carpetas");
    expect(root().textContent).toContain("Arrastrá un clip");
  });

  it("says so when every clip is already in a folder", async () => {
    await open([clip(1, ["f1"])], [folder("f1", "Todo", { clipCount: 1 })]);
    expect(looseCards()).toHaveLength(0);
    expect(root().textContent).toContain("Todos tus clips están en alguna carpeta");
  });

  it("an empty folder says it holds no clips", async () => {
    await open([], [folder("f1", "Vacía")]);
    expect(q(".card-meta", folderCard("f1"))!.textContent).toBe("Sin clips");
  });

  it("shows the folder's color and icon", async () => {
    await open([], [folder("f1", "Azul", { color: "blue", icon: "trophy" })]);
    expect(folderCard("f1").getAttribute("style")).toContain("--folder-color:#4da3ff");
    expect(q("svg", folderCard("f1"))).not.toBeNull();
  });

  it("copes with a color or icon it has never heard of", async () => {
    await open([], [folder("f1", "Futuro", { color: "ultraviolet", icon: "rocket" })]);
    expect(folderCard("f1").getAttribute("style")).toContain("--folder-color:#ff7a1a");
    expect(q("svg", folderCard("f1"))).not.toBeNull();
  });

  it("loose clips can be dragged, and show no 'take out' button", async () => {
    await open([clip(1)], [folder("f1", "A")]);
    expect(looseCards()[0].getAttribute("draggable")).toBe("true");
    expect(action(looseCards()[0], "Sacar de esta carpeta")).toBeNull();
    expect(action(looseCards()[0], "Carpetas…")).not.toBeNull();
  });

  it("surfaces a failed load as an error toast and still renders", async () => {
    api.listClips.mockRejectedValue("la carpeta de clips no se puede leer");
    const library = createLibrary(root());
    await library.refresh();
    expect(toastText().join()).toContain("no se puede leer");
    expect(q("h1", root())!.textContent).toBe("Biblioteca");
  });

  it("onShow reloads", async () => {
    const library = await open([], []);
    api.listFolders.mockResolvedValue([folder("f1", "Nueva")]);
    library.onShow();
    await settle();
    expect(folderCards()).toHaveLength(1);
  });

  it("opening the 'Carpetas…' button of a clip hands that clip to the picker", async () => {
    await open([clip(1)], [folder("f1", "A")]);
    click(action(looseCards()[0], "Carpetas…"));
    expect(picker.openFolderPicker).toHaveBeenCalledTimes(1);
    expect(picker.openFolderPicker.mock.calls[0][0].name).toBe("clip-1.mp4");
  });
});

describe("making a folder", () => {
  const dialog = () => q(".folder-dialog")!;
  const nameInput = () => q<HTMLInputElement>("input", dialog())!;
  const submit = () => q<HTMLButtonElement>(".btn-primary", dialog())!;

  it("creates it with the name, color and icon you picked", async () => {
    await open([], [folder("f1", "Existente")]);
    click(q(".view-header .btn-primary", root()));
    expect(dialog()).not.toBeNull();
    expect(submit().disabled).toBe(true); // no name yet

    type(nameInput(), "  Valorant  ");
    click(q('[data-color="green"]', dialog()));
    click(q('[data-icon="trophy"]', dialog()));
    expect(q('[data-color="green"]', dialog())!.getAttribute("aria-checked")).toBe("true");
    expect(q('[data-color="orange"]', dialog())!.getAttribute("aria-checked")).toBe("false");
    expect(q(".folder-preview-name", dialog())!.textContent).toBe("Valorant");

    click(submit());
    await settle();
    expect(api.createFolder).toHaveBeenCalledWith({ name: "Valorant", color: "green", icon: "trophy" });
    expect(q(".folder-dialog")).toBeNull();
    expect(toastText().join()).toContain("«Valorant» creada");
    expect(api.listFolders).toHaveBeenCalledTimes(2); // reloaded
  });

  it("Enter in the name box also creates it, with the default look", async () => {
    await open([], []);
    click(q(".view-header .btn-primary", root()));
    type(nameInput(), "Rápida");
    nameInput().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
    expect(api.createFolder).toHaveBeenCalledWith({ name: "Rápida", color: "orange", icon: "folder" });
  });

  it("refuses a name that is taken, with a reason, before asking the backend", async () => {
    await open([], [folder("f1", "Valorant")]);
    click(q(".view-header .btn-primary", root()));
    type(nameInput(), "valorant");
    expect(q(".field-error", dialog())!.textContent).toContain("Ya existe");
    expect(submit().disabled).toBe(true);
    nameInput().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
    expect(api.createFolder).not.toHaveBeenCalled();
    expect(q(".folder-dialog")).not.toBeNull();
  });

  it("cancel and Escape make nothing", async () => {
    await open([], []);
    click(q(".view-header .btn-primary", root()));
    type(nameInput(), "Nada");
    click(all(".dialog-actions .btn", dialog())[0]);
    await settle();
    expect(q(".folder-dialog")).toBeNull();

    click(q(".view-header .btn-primary", root()));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    expect(q(".folder-dialog")).toBeNull();
    expect(api.createFolder).not.toHaveBeenCalled();
  });

  it("shows the backend's reason when it refuses", async () => {
    api.createFolder.mockRejectedValue("no se pueden tener más de 200 carpetas");
    await open([], []);
    click(q(".view-header .btn-primary", root()));
    type(nameInput(), "Una más");
    click(submit());
    await settle();
    expect(toastText().join()).toContain("200 carpetas");
  });

  it("the empty state has its own button", async () => {
    await open([], []);
    click(q(".empty-inline .btn-primary", root()));
    expect(dialog()).not.toBeNull();
  });
});

describe("editing a folder", () => {
  const dialog = () => q(".folder-dialog")!;

  it("opens with its current values and saves what you change", async () => {
    await open([], [folder("f1", "Original", { color: "blue", icon: "star" })]);
    click(action(folderCard("f1"), "Editar carpeta"));
    expect(q<HTMLInputElement>("input", dialog())!.value).toBe("Original");
    expect(q('[data-color="blue"]', dialog())!.getAttribute("aria-checked")).toBe("true");
    expect(q('[data-icon="star"]', dialog())!.getAttribute("aria-checked")).toBe("true");

    type(q<HTMLInputElement>("input", dialog())!, "Cambiada");
    click(q('[data-color="red"]', dialog()));
    click(q('[data-icon="flame"]', dialog()));
    click(q(".btn-primary", dialog()));
    await settle();
    expect(api.updateFolder).toHaveBeenCalledWith("f1", { name: "Cambiada", color: "red", icon: "flame" });
    expect(toastText().join()).toContain("Carpeta actualizada");
  });

  it("lets a folder keep its own name", async () => {
    await open([], [folder("f1", "Original"), folder("f2", "Otra")]);
    click(action(folderCard("f1"), "Editar carpeta"));
    expect(q<HTMLButtonElement>(".btn-primary", dialog())!.disabled).toBe(false);
    type(q<HTMLInputElement>("input", dialog())!, "otra");
    expect(q<HTMLButtonElement>(".btn-primary", dialog())!.disabled).toBe(true);
  });

  it("an unknown color or icon opens on the defaults", async () => {
    await open([], [folder("f1", "Raro", { color: "ultraviolet", icon: "rocket" })]);
    click(action(folderCard("f1"), "Editar carpeta"));
    expect(q('[data-color="orange"]', dialog())!.getAttribute("aria-checked")).toBe("true");
    expect(q('[data-icon="folder"]', dialog())!.getAttribute("aria-checked")).toBe("true");
  });
});

describe("renaming a folder in place", () => {
  const rename = async () => {
    click(action(folderCard("f1"), "Renombrar"));
    return q<HTMLInputElement>(".rename-input", folderCard("f1"))!;
  };
  const enter = (input: HTMLInputElement) =>
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

  it("Enter saves the new name and keeps color and icon", async () => {
    await open([], [folder("f1", "Vieja", { color: "teal", icon: "bolt" })]);
    const input = await rename();
    expect(input.value).toBe("Vieja");
    input.value = "  Nueva  ";
    enter(input);
    await settle();
    expect(api.updateFolder).toHaveBeenCalledWith("f1", { name: "Nueva", color: "teal", icon: "bolt" });
  });

  it("Escape and an unchanged name do nothing", async () => {
    await open([], [folder("f1", "Vieja")]);
    let input = await rename();
    input.value = "Otra";
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    input = await rename();
    enter(input);
    await settle();
    expect(api.updateFolder).not.toHaveBeenCalled();
  });

  it("a taken name is refused with a message and nothing is sent", async () => {
    await open([], [folder("f1", "Vieja"), folder("f2", "Ocupada")]);
    const input = await rename();
    input.value = "ocupada";
    enter(input);
    await settle();
    expect(api.updateFolder).not.toHaveBeenCalled();
    expect(toastText().join()).toContain("Ya existe");
  });

  it("a refresh arriving mid-edit does not tear the box down", async () => {
    const library = await open([], [folder("f1", "Vieja")]);
    const input = await rename();
    await library.refresh();
    expect(q(".rename-input", root())).toBe(input);
  });

  it("clicking inside the box does not open the folder", async () => {
    await open([], [folder("f1", "Vieja")]);
    const input = await rename();
    click(input);
    expect(q(".breadcrumb", root())).toBeNull();
  });
});

describe("deleting a folder", () => {
  const dialog = () => q(".dialog")!;

  it("an empty folder only needs a confirmation", async () => {
    await open([], [folder("f1", "Vacía")]);
    click(action(folderCard("f1"), "Borrar carpeta"));
    expect(dialog().textContent).toContain("está vacía");
    click(q(".btn-danger", dialog()));
    await settle();
    expect(api.deleteFolder).toHaveBeenCalledWith("f1", false);
    expect(toastText().join()).toContain("Carpeta borrada");
  });

  const withClips = () =>
    open(
      [clip(1, ["f1"]), clip(2, ["f1"]), clip(3, ["f1", "f2"])],
      [folder("f1", "Con clips", { clipCount: 3 }), folder("f2", "Otra", { clipCount: 1 })],
    );

  it("offers to keep the clips or delete the ones only it holds", async () => {
    await withClips();
    click(action(folderCard("f1"), "Borrar carpeta"));
    const text = dialog().textContent!;
    expect(text).toContain("Tiene 3 clips");
    expect(text).toContain("2 clips que no están en ninguna otra carpeta");
    expect(text).toContain("1 clip está también en otras carpetas");
    const labels = all("[data-choice]", dialog()).map((b) => b.textContent);
    expect(labels).toEqual(["Conservar los clips", "Borrar carpeta y 2 clips"]);
  });

  it("'Conservar los clips' deletes the folder only", async () => {
    await withClips();
    click(action(folderCard("f1"), "Borrar carpeta"));
    click(q('[data-choice="keep"]', dialog()));
    await settle();
    expect(api.deleteFolder).toHaveBeenCalledWith("f1", false);
  });

  it("'Borrar todo' deletes the clips too, and says how many went", async () => {
    api.deleteFolder.mockResolvedValue(2);
    await withClips();
    click(action(folderCard("f1"), "Borrar carpeta"));
    click(q('[data-choice="all"]', dialog()));
    await settle();
    expect(api.deleteFolder).toHaveBeenCalledWith("f1", true);
    expect(toastText().join()).toContain("junto con 2 clips");
  });

  it("the safe choice is focused and cancelling deletes nothing", async () => {
    await withClips();
    click(action(folderCard("f1"), "Borrar carpeta"));
    expect(document.activeElement).toBe(q('[data-choice="keep"]', dialog()));
    click(all(".dialog-actions .btn", dialog())[0]); // Cancelar
    await settle();
    expect(api.deleteFolder).not.toHaveBeenCalled();
    expect(q(".dialog")).toBeNull();
  });

  it("when every clip is also elsewhere there is nothing to choose", async () => {
    await open(
      [clip(1, ["f1", "f2"])],
      [folder("f1", "Compartida", { clipCount: 1 }), folder("f2", "Otra", { clipCount: 1 })],
    );
    click(action(folderCard("f1"), "Borrar carpeta"));
    expect(q("[data-choice]", dialog())).toBeNull();
    expect(dialog().textContent).toContain("Los clips no se borran");
    click(q(".btn-danger", dialog()));
    await settle();
    expect(api.deleteFolder).toHaveBeenCalledWith("f1", false);
  });

  it("shows the reason when it fails and reloads anyway", async () => {
    api.deleteFolder.mockRejectedValue("no se pudieron borrar 1 clips; la carpeta se conservó");
    await open([], [folder("f1", "Vacía")]);
    click(action(folderCard("f1"), "Borrar carpeta"));
    click(q(".btn-danger", dialog()));
    await settle();
    expect(toastText().join()).toContain("se conservó");
    expect(api.listFolders).toHaveBeenCalledTimes(2);
  });
});

describe("inside a folder", () => {
  const folders = [folder("f1", "Mía", { clipCount: 2, sizeBytes: 3_000_000 }), folder("f2", "Otra")];
  const clips = [clip(1, ["f1"]), clip(2), clip(3, ["f1", "f2"])];

  it("opens on click and shows only its clips", async () => {
    await open(clips, folders);
    click(folderCard("f1"));
    expect(q("h1", root())!.textContent).toContain("Mía");
    expect(q(".breadcrumb", root())!.textContent).toContain("Biblioteca");
    expect(all(".card", root()).map((c) => q(".card-title", c)!.textContent)).toEqual(["clip-1", "clip-3"]);
    expect(root().textContent).toContain("2 clips");
  });

  it("opens with the keyboard too", async () => {
    await open(clips, folders);
    folderCard("f1").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(q(".breadcrumb", root())).not.toBeNull();
  });

  it("labels each clip with its other folders", async () => {
    await open(clips, folders);
    click(folderCard("f1"));
    const third = all(".card", root())[1];
    expect(all(".chip", third).map((c) => c.textContent)).toEqual(["Mía", "Otra"]);
  });

  it("the back button returns to the root", async () => {
    await open(clips, folders);
    click(folderCard("f1"));
    click(q(".crumb", root()));
    expect(q(".breadcrumb", root())).toBeNull();
    expect(folderCards()).toHaveLength(2);
  });

  it("can take a clip out of the folder without deleting it", async () => {
    await open(clips, folders);
    click(folderCard("f1"));
    click(action(all(".card", root())[0], "Sacar de esta carpeta"));
    await settle();
    expect(api.removeFromFolder).toHaveBeenCalledWith("f1", ["clip-1.mp4"]);
    expect(api.deleteClip).not.toHaveBeenCalled();
    expect(toastText().join()).toContain("salió de «Mía»");
  });

  it("its clips are not draggable (there is no folder to drop on)", async () => {
    await open(clips, folders);
    click(folderCard("f1"));
    expect(all(".card", root())[0].getAttribute("draggable")).toBeNull();
  });

  it("edits and deletes the folder from its own header", async () => {
    await open(clips, folders);
    click(folderCard("f1"));
    const [edit, remove] = all(".header-actions .btn", root());
    click(edit);
    expect(q(".folder-dialog")).not.toBeNull();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();

    click(remove);
    click(q('[data-choice="keep"]'));
    await settle();
    expect(api.deleteFolder).toHaveBeenCalledWith("f1", false);
  });

  it("goes back to the root when the folder it shows is gone", async () => {
    const library = await open(clips, folders);
    click(folderCard("f1"));
    api.listFolders.mockResolvedValue([folders[1]]);
    await library.refresh();
    expect(q(".breadcrumb", root())).toBeNull();
    expect(folderCards()).toHaveLength(1);
  });

  it("an empty folder explains how to fill it", async () => {
    await open([clip(2)], [folder("f1", "Vacía")]);
    click(folderCard("f1"));
    expect(q(".empty", root())!.textContent).toContain("Esta carpeta está vacía");
    expect(q(".empty", root())!.textContent).toContain("arrastrándolos");
  });
});

describe("dragging a clip onto a folder", () => {
  const TYPE = "application/x-clipforge-clip";

  function drag(target: Element, type: string, carries: string[] = [TYPE], payload = "clip-2.mp4") {
    const event = new Event(type, { bubbles: true, cancelable: true });
    Object.defineProperty(event, "dataTransfer", {
      value: { types: carries, getData: () => payload, dropEffect: "none", effectAllowed: "all", setData: vi.fn() },
    });
    target.dispatchEvent(event);
    return event;
  }

  const setup = () => open([clip(2)], [folder("f1", "Destino")]);

  it("a dragged clip carries its file name", async () => {
    await setup();
    const event = drag(looseCards()[0], "dragstart");
    const data = (event as unknown as { dataTransfer: { setData: ReturnType<typeof vi.fn> } }).dataTransfer;
    expect(data.setData).toHaveBeenCalledWith(TYPE, "clip-2.mp4");
    expect(looseCards()[0].classList.contains("dragging")).toBe(true);
    drag(looseCards()[0], "dragend");
    expect(looseCards()[0].classList.contains("dragging")).toBe(false);
  });

  it("lights up the folder under a clip and files the clip on drop", async () => {
    await setup();
    const target = folderCard("f1");
    const over = drag(target, "dragover");
    expect(over.defaultPrevented).toBe(true);
    expect(target.classList.contains("drop-target")).toBe(true);

    const dropped = drag(target, "drop");
    expect(dropped.defaultPrevented).toBe(true);
    expect(target.classList.contains("drop-target")).toBe(false);
    await settle();
    expect(api.addToFolder).toHaveBeenCalledWith("f1", ["clip-2.mp4"]);
    expect(toastText().join()).toContain("«clip-2» agregado a «Destino»");
  });

  it("says when the clip was already there", async () => {
    api.addToFolder.mockResolvedValue(0);
    await setup();
    drag(folderCard("f1"), "drop");
    await settle();
    expect(toastText().join()).toContain("ya estaba en «Destino»");
  });

  it("ignores drags that are not clips (text, files)", async () => {
    await setup();
    const target = folderCard("f1");
    const over = drag(target, "dragover", ["text/plain"]);
    expect(over.defaultPrevented).toBe(false);
    expect(target.classList.contains("drop-target")).toBe(false);
    drag(target, "drop", ["text/plain"]);
    await settle();
    expect(api.addToFolder).not.toHaveBeenCalled();
  });

  it("stops lighting up when the clip leaves", async () => {
    await setup();
    const target = folderCard("f1");
    drag(target, "dragover");
    drag(target, "dragleave");
    expect(target.classList.contains("drop-target")).toBe(false);
  });

  it("shows the backend's reason when filing fails", async () => {
    api.addToFolder.mockRejectedValue("el clip «clip-2.mp4» ya no existe");
    await setup();
    drag(folderCard("f1"), "drop");
    await settle();
    expect(toastText().join()).toContain("ya no existe");
  });
});
