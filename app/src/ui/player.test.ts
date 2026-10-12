// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipInfo } from "../lib/clips";

const api = vi.hoisted(() => ({
  clipSrc: vi.fn((clip: { path: string }) => `asset://${clip.path}`),
  copyClip: vi.fn(),
}));
vi.mock("../api", () => api);
const editor = vi.hoisted(() => ({ openEditor: vi.fn() }));
vi.mock("./editor", () => editor);

const clip: ClipInfo = {
  name: "clip-1.mp4",
  kind: "clip",
  path: "/clips/clip-1.mp4",
  sizeBytes: 3 * 1024 * 1024,
  modifiedMs: 0,
  folders: [],
  game: null,
};

type PlayerModule = typeof import("./player");
let openPlayer: PlayerModule["openPlayer"];

beforeEach(async () => {
  document.body.innerHTML = '<div id="toasts"></div>';
  api.copyClip.mockReset();
  editor.openEditor.mockReset();
  vi.resetModules();
  ({ openPlayer } = (await import("./player")) as PlayerModule);
});

afterEach(() => {
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
});

const button = (text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>(".player-actions .btn")].find(
    (b) => b.textContent === text,
  )!;
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

describe("the player", () => {
  it("can copy the clip to paste it somewhere else", async () => {
    api.copyClip.mockResolvedValue(undefined);
    openPlayer(clip);
    button("Copiar").click();
    await settle();
    expect(api.copyClip).toHaveBeenCalledWith("clip-1.mp4");
    expect(document.getElementById("toasts")!.textContent).toContain("Copiado");
    expect(document.querySelector(".player"), "stays open to keep watching").not.toBeNull();
  });

  it("shows a screenshot as a picture, with nothing to trim", async () => {
    api.copyClip.mockResolvedValue(undefined);
    openPlayer({ ...clip, name: "captura-1.png", path: "/clips/captura-1.png", kind: "screenshot" });
    const image = document.querySelector<HTMLImageElement>(".player img.player-image")!;
    expect(image.getAttribute("src")).toBe("asset:///clips/captura-1.png");
    expect(document.querySelector(".player video")).toBeNull();
    expect(button("Recortar")).toBeUndefined();
    expect(document.querySelector(".player-title strong")!.textContent).toBe("captura-1");

    button("Copiar").click();
    await settle();
    expect(api.copyClip).toHaveBeenCalledWith("captura-1.png");
  });

  it("closes a screenshot without trying to release a video", () => {
    openPlayer({ ...clip, name: "captura-1.png", kind: "screenshot" });
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(document.querySelector(".player")).toBeNull();
  });

  it("still opens the editor from its own button", () => {
    openPlayer(clip);
    button("Recortar").click();
    expect(editor.openEditor).toHaveBeenCalledWith(clip);
  });

  it("a failed copy is reported", async () => {
    api.copyClip.mockRejectedValue("otra aplicación tiene ocupado el portapapeles");
    openPlayer(clip);
    button("Copiar").click();
    await settle();
    expect(document.querySelector(".toast-error")?.textContent).toContain("ocupado");
  });
});
