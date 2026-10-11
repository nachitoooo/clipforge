// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipInfo } from "../lib/clips";

const api = vi.hoisted(() => ({ setClipGame: vi.fn() }));
vi.mock("../api", () => api);

const clip = (game: string | null): ClipInfo => ({
  name: "clip-1.mp4",
  path: "/clips/clip-1.mp4",
  sizeBytes: 1,
  modifiedMs: 0,
  folders: [],
  game,
});

const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

type Module = typeof import("./gamedialog");
let openGameDialog: Module["openGameDialog"];

beforeEach(async () => {
  document.body.innerHTML = '<div id="toasts"></div>';
  api.setClipGame.mockReset();
  api.setClipGame.mockResolvedValue(1);
  vi.resetModules();
  ({ openGameDialog } = (await import("./gamedialog")) as Module);
});

afterEach(() => {
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
});

const dialog = () => document.querySelector(".game-dialog") as HTMLElement | null;
const input = () => dialog()!.querySelector("input[aria-label='Juego']") as HTMLInputElement;
const allBox = () => dialog()!.querySelector("input[type=checkbox]") as HTMLInputElement | null;
const buttons = () => [...dialog()!.querySelectorAll<HTMLButtonElement>(".dialog-actions .btn")];
const button = (label: string) => buttons().find((b) => b.textContent === label)!;
const toastText = () => [...document.querySelectorAll(".toast")].map((t) => t.textContent).join();

function open(game: string | null, others = 1, games = ["Valorant", "Fortnite"]) {
  const onSaved = vi.fn();
  openGameDialog({ clip: clip(game), games, clipsOfGame: () => others, onSaved });
  return onSaved;
}

function type(text: string) {
  input().value = text;
  input().dispatchEvent(new Event("input", { bubbles: true }));
}

describe("the game dialog", () => {
  it("opens with the clip's game and suggests the known ones", () => {
    open("Valorant");
    expect(input().value).toBe("Valorant");
    const suggested = [...dialog()!.querySelectorAll("datalist option")].map((o) => o.getAttribute("value"));
    expect(suggested).toEqual(["Valorant", "Fortnite"]);
    expect(input().getAttribute("list")).toBe(dialog()!.querySelector("datalist")!.id);
  });

  it("sets the game of a clip that had none", async () => {
    const onSaved = open(null);
    expect(input().value).toBe("");
    expect(allBox()).toBeNull(); // nothing to apply to the others
    expect(button("Quitar el juego")).toBeUndefined();
    type("  Minecraft ");
    button("Guardar").click();
    await settle();
    expect(api.setClipGame).toHaveBeenCalledWith("clip-1.mp4", "Minecraft", false);
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(dialog()).toBeNull();
    expect(toastText()).toContain("Juego actualizado");
  });

  it("Enter saves too", async () => {
    open(null);
    type("Minecraft");
    input().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
    expect(api.setClipGame).toHaveBeenCalledTimes(1);
  });

  it("offers to change every clip of the game when there are others", async () => {
    api.setClipGame.mockResolvedValue(4);
    const onSaved = open("VALORANT-Win64-Shipping", 4);
    expect(dialog()!.textContent).toContain("Cambiar en todos los clips de «VALORANT-Win64-Shipping» (4)");
    expect(allBox()!.checked).toBe(false);
    type("Valorant");
    allBox()!.checked = true;
    button("Guardar").click();
    await settle();
    expect(api.setClipGame).toHaveBeenCalledWith("clip-1.mp4", "Valorant", true);
    expect(toastText()).toContain("cambiado en 4 clips");
    expect(onSaved).toHaveBeenCalled();
  });

  it("does not offer it when the clip is the only one of its game", () => {
    open("Solo", 1);
    expect(allBox()).toBeNull();
  });

  it("leaves the others alone unless asked", async () => {
    open("Game", 3);
    type("Otro");
    button("Guardar").click();
    await settle();
    expect(api.setClipGame).toHaveBeenCalledWith("clip-1.mp4", "Otro", false);
  });

  it("refuses an empty or silly name with a reason, before asking the backend", async () => {
    open("Game");
    type("   ");
    button("Guardar").click();
    await settle();
    expect(api.setClipGame).not.toHaveBeenCalled();
    expect(dialog()!.querySelector(".field-error")!.textContent).toContain("vacío");
    type("a".repeat(61));
    button("Guardar").click();
    expect(dialog()!.querySelector(".field-error")!.textContent).toContain("60");
    expect(dialog()).not.toBeNull();
  });

  it("typing clears the error", () => {
    open("Game");
    type("");
    button("Guardar").click();
    expect(dialog()!.querySelector(".field-error")!.textContent).not.toBe("");
    type("x");
    expect(dialog()!.querySelector(".field-error")!.textContent).toBe("");
  });

  it("closes without calling the backend when the name is the same", async () => {
    const onSaved = open("Game");
    button("Guardar").click();
    await settle();
    expect(api.setClipGame).not.toHaveBeenCalled();
    expect(onSaved).not.toHaveBeenCalled();
    expect(dialog()).toBeNull();
  });

  it("changing only the capitals does save (it fixes the spelling)", async () => {
    open("valorant");
    type("Valorant");
    button("Guardar").click();
    await settle();
    expect(api.setClipGame).toHaveBeenCalledWith("clip-1.mp4", "Valorant", false);
  });

  it("can take the game away from one clip", async () => {
    const onSaved = open("Game", 5);
    button("Quitar el juego").click();
    await settle();
    expect(api.setClipGame).toHaveBeenCalledWith("clip-1.mp4", null, false);
    expect(toastText()).toContain("sin juego");
    expect(onSaved).toHaveBeenCalled();
    expect(dialog()).toBeNull();
  });

  it("cancel and Escape change nothing", async () => {
    const onSaved = open("Game");
    button("Cancelar").click();
    await settle();
    expect(dialog()).toBeNull();
    open("Game");
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    expect(dialog()).toBeNull();
    expect(api.setClipGame).not.toHaveBeenCalled();
    expect(onSaved).not.toHaveBeenCalled();
  });

  it("keeps the dialog open and says why when the backend refuses", async () => {
    api.setClipGame.mockRejectedValue("el clip «clip-1.mp4» ya no existe");
    const onSaved = open(null);
    type("Game");
    button("Guardar").click();
    await settle();
    expect(toastText()).toContain("ya no existe");
    expect(dialog()).not.toBeNull();
    expect(onSaved).not.toHaveBeenCalled();
  });

  it("two dialogs never share a suggestion list", () => {
    open("A");
    const first = dialog()!.querySelector("datalist")!.id;
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    open("B");
    expect(dialog()!.querySelector("datalist")!.id).not.toBe(first);
  });
});
