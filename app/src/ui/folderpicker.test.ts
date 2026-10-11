// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipInfo } from "../lib/clips";
import type { FolderInfo } from "../lib/folders";

const api = vi.hoisted(() => ({
  listFolders: vi.fn(),
  createFolder: vi.fn(),
  setClipFolders: vi.fn(),
}));
vi.mock("../api", () => api);

const folder = (id: string, name: string, clipCount = 0): FolderInfo => ({
  id,
  name,
  color: "blue",
  icon: "star",
  clipCount,
  sizeBytes: 0,
  createdMs: 0,
});

const clip = (folders: string[]): ClipInfo => ({
  name: "clip-1.mp4",
  path: "/clips/clip-1.mp4",
  sizeBytes: 1,
  modifiedMs: 0,
  folders,
});

const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

type PickerModule = typeof import("./folderpicker");
let openFolderPicker: PickerModule["openFolderPicker"];

beforeEach(async () => {
  document.body.innerHTML = '<div id="toasts"></div>';
  for (const fn of Object.values(api)) fn.mockReset();
  api.listFolders.mockResolvedValue([folder("f1", "Zorros", 2), folder("f2", "Águilas", 1)]);
  api.setClipFolders.mockResolvedValue(undefined);
  vi.resetModules();
  ({ openFolderPicker } = (await import("./folderpicker")) as PickerModule);
});

afterEach(() => {
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
});

const dialog = () => document.querySelector(".folder-picker") as HTMLElement | null;
const boxes = () => [...document.querySelectorAll<HTMLInputElement>(".picker-row input")];
const box = (id: string) => document.querySelector<HTMLInputElement>(`input[data-folder="${id}"]`)!;
const saveButton = () => document.querySelector<HTMLButtonElement>(".dialog-actions .btn-primary")!;
const newName = () => document.querySelector<HTMLInputElement>(".picker-new input")!;
const toastText = () => [...document.querySelectorAll(".toast")].map((t) => t.textContent).join();

function tick(input: HTMLInputElement, checked: boolean) {
  input.checked = checked;
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("the folder picker", () => {
  it("lists the folders by name with the clip's own ticked", async () => {
    openFolderPicker(clip(["f1"]), vi.fn());
    expect(dialog()!.textContent).toContain("Cargando");
    await settle();
    expect(boxes().map((b) => b.getAttribute("aria-label"))).toEqual(["Águilas", "Zorros"]);
    expect(box("f1").checked).toBe(true);
    expect(box("f2").checked).toBe(false);
    expect(dialog()!.textContent).toContain("clip-1"); // the clip it is about
  });

  it("saves the new set of folders and reports back", async () => {
    const onSaved = vi.fn();
    openFolderPicker(clip(["f1"]), onSaved);
    await settle();
    tick(box("f2"), true);
    saveButton().click();
    await settle();
    expect(api.setClipFolders).toHaveBeenCalledWith("clip-1.mp4", ["f1", "f2"]);
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(dialog()).toBeNull();
    expect(toastText()).toContain("Carpetas actualizadas");
  });

  it("can take the clip out of all of its folders", async () => {
    openFolderPicker(clip(["f1", "f2"]), vi.fn());
    await settle();
    tick(box("f1"), false);
    tick(box("f2"), false);
    saveButton().click();
    await settle();
    expect(api.setClipFolders).toHaveBeenCalledWith("clip-1.mp4", []);
  });

  it("only offers Save once something changed", async () => {
    openFolderPicker(clip(["f1"]), vi.fn());
    await settle();
    expect(saveButton().disabled).toBe(true);
    tick(box("f2"), true);
    expect(saveButton().disabled).toBe(false);
    tick(box("f2"), false);
    expect(saveButton().disabled).toBe(true);
  });

  it("cancelling saves nothing", async () => {
    const onSaved = vi.fn();
    openFolderPicker(clip([]), onSaved);
    await settle();
    tick(box("f1"), true);
    document.querySelectorAll<HTMLButtonElement>(".dialog-actions .btn")[0].click();
    await settle();
    expect(api.setClipFolders).not.toHaveBeenCalled();
    expect(onSaved).not.toHaveBeenCalled();
    expect(dialog()).toBeNull();
  });

  it("ignores a folder id that no longer exists", async () => {
    openFolderPicker(clip(["f1", "f77"]), vi.fn());
    await settle();
    expect(saveButton().disabled).toBe(true);
    tick(box("f2"), true);
    saveButton().click();
    await settle();
    expect(api.setClipFolders).toHaveBeenCalledWith("clip-1.mp4", ["f1", "f2"]);
  });

  it("makes a new folder on the spot and ticks it", async () => {
    api.createFolder.mockResolvedValue(folder("f3", "Recién hecha"));
    openFolderPicker(clip([]), vi.fn());
    await settle();
    newName().value = "Recién hecha";
    newName().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
    expect(api.createFolder).toHaveBeenCalledWith({ name: "Recién hecha", color: "orange", icon: "folder" });
    expect(box("f3").checked).toBe(true);
    expect(newName().value).toBe("");
    saveButton().click();
    await settle();
    expect(api.setClipFolders).toHaveBeenCalledWith("clip-1.mp4", ["f3"]);
  });

  it("refuses a name that is taken, saying why", async () => {
    openFolderPicker(clip([]), vi.fn());
    await settle();
    newName().value = "zorros";
    document.querySelector<HTMLButtonElement>(".picker-new .btn")!.click();
    await settle();
    expect(api.createFolder).not.toHaveBeenCalled();
    expect(dialog()!.querySelector(".field-error")!.textContent).toContain("Ya existe");
  });

  it("explains itself when there are no folders yet", async () => {
    api.listFolders.mockResolvedValue([]);
    openFolderPicker(clip([]), vi.fn());
    await settle();
    expect(dialog()!.textContent).toContain("Todavía no tenés carpetas");
    expect(boxes()).toHaveLength(0);
  });

  it("gives up with a message when the folders can't be read", async () => {
    api.listFolders.mockRejectedValue("no se pudo leer la biblioteca");
    openFolderPicker(clip([]), vi.fn());
    await settle();
    expect(toastText()).toContain("no se pudo leer la biblioteca");
    expect(dialog()).toBeNull();
  });

  it("keeps the dialog open and says why when saving fails", async () => {
    api.setClipFolders.mockRejectedValue("el clip «clip-1.mp4» ya no existe");
    const onSaved = vi.fn();
    openFolderPicker(clip([]), onSaved);
    await settle();
    tick(box("f1"), true);
    saveButton().click();
    await settle();
    expect(toastText()).toContain("ya no existe");
    expect(dialog()).not.toBeNull();
    expect(onSaved).not.toHaveBeenCalled();
  });
});
