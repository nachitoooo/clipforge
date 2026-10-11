// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipInfo } from "../lib/clips";
import type { EditorClip } from "../lib/editor";

// ---- a fake backend ---------------------------------------------------------

const api = vi.hoisted(() => ({
  describeClip: vi.fn(),
  clipWaveform: vi.fn(),
  trimClip: vi.fn(),
  clipSrc: vi.fn((clip: { path: string }) => `asset://${clip.path}`),
}));
vi.mock("../api", () => api);
vi.mock("./filmstrip", () => ({ buildFilmstrip: vi.fn(async () => {}) }));
const toasts = vi.hoisted(() => ({ toast: vi.fn() }));
vi.mock("./toast", () => toasts);

const clip: ClipInfo = {
  name: "clip-20261009-101500.mp4",
  path: "/clips/clip-20261009-101500.mp4",
  sizeBytes: 5_000_000,
  modifiedMs: 0,
  folders: [],
};

const info = (audioTracks: number): EditorClip => ({
  name: clip.name,
  duration: 20,
  width: 1920,
  height: 1080,
  fps: 60,
  frames: 1200,
  audio: Array.from({ length: audioTracks }, (_, index) => ({
    index,
    channels: 2,
    sampleRate: 48000,
    duration: 20,
  })),
});

/** Peaks for a 20 s clip: loud in the first half, silent in the second. */
const wave = () => ({
  perSecond: 50,
  peaks: Array.from({ length: 1000 }, (_, i) => (i < 500 ? 200 : 0)),
});

const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

type EditorModule = typeof import("./editor");
let openEditor: EditorModule["openEditor"];

async function open(audioTracks: number, onClosed?: () => void) {
  api.describeClip.mockResolvedValue(info(audioTracks));
  const close = openEditor(clip, { onClosed });
  await settle();
  await settle();
  return close;
}

const q = <T extends Element>(selector: string) => document.querySelector(selector) as T;
const qa = <T extends Element>(selector: string) =>
  [...document.querySelectorAll(selector)] as T[];
const saveButton = () => q<HTMLButtonElement>(".editor-actions .btn-primary");
const startInput = () => q<HTMLInputElement>('input[aria-label^="Inicio"]');
const endInput = () => q<HTMLInputElement>('input[aria-label^="Final"]');
const video = () => q<HTMLVideoElement>(".editor-video");
const lane = (index: number) => q<HTMLElement>(`[data-track="${index}"]`);

/** Pretend the overlay is 1000 px wide, starting at x = 100. */
function sizeTimeline() {
  q<HTMLElement>(".tl-overlay").getBoundingClientRect = () =>
    ({ left: 100, top: 0, width: 1000, height: 200, right: 1100, bottom: 200, x: 100, y: 0 }) as DOMRect;
}

/** x of `seconds` on the 1000 px, 20 s timeline above. */
const xAt = (seconds: number) => 100 + (seconds / 20) * 1000;

function pointer(target: Element, type: string, clientX: number) {
  target.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX, button: 0 }));
}

function drag(handle: Element, toSeconds: number) {
  pointer(handle, "pointerdown", 0);
  pointer(handle, "pointermove", xAt(toSeconds));
  pointer(handle, "pointerup", xAt(toSeconds));
}

function press(key: string, init: KeyboardEventInit = {}, target: Element = q(".editor")) {
  target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }));
}

/** Give the <video> a playable state machine (happy-dom has none). */
function fakePlayback(el: HTMLVideoElement) {
  let paused = true;
  Object.defineProperty(el, "paused", { get: () => paused });
  Object.defineProperty(el, "ended", { get: () => false });
  el.play = vi.fn(async () => {
    paused = false;
    el.dispatchEvent(new Event("play"));
  });
  el.pause = vi.fn(() => {
    paused = true;
    el.dispatchEvent(new Event("pause"));
  });
  el.load = vi.fn();
}

beforeEach(async () => {
  document.body.innerHTML = "";
  for (const fn of [api.describeClip, api.clipWaveform, api.trimClip, toasts.toast]) fn.mockReset();
  api.clipWaveform.mockResolvedValue(wave());
  vi.resetModules();
  ({ openEditor } = (await import("./editor")) as EditorModule);
});

afterEach(() => {
  document.body.innerHTML = "";
});

// ---- opening ------------------------------------------------------------------

describe("opening the editor", () => {
  it("shows a loading state until the clip is described", async () => {
    let resolve!: (value: EditorClip) => void;
    api.describeClip.mockReturnValue(new Promise<EditorClip>((r) => (resolve = r)));
    openEditor(clip);
    expect(q(".editor-loading").textContent).toContain("Abriendo");
    expect(q(".editor").getAttribute("aria-label")).toContain("clip-20261009-101500");
    resolve(info(1));
    await settle();
    expect(document.querySelector(".editor-loading")).toBeNull();
    expect(document.querySelector(".timeline")).not.toBeNull();
  });

  it("asks for the clip by name and plays it from the asset protocol", async () => {
    await open(1);
    expect(api.describeClip).toHaveBeenCalledWith(clip.name);
    expect(video().getAttribute("src")).toBe(`asset://${clip.path}`);
  });

  it("reports the clip's size and format", async () => {
    await open(1);
    expect(q(".editor-title").textContent).toContain("1920×1080");
    expect(q(".editor-title").textContent).toContain("60 fps");
  });

  it("explains why it could not open a clip", async () => {
    api.describeClip.mockRejectedValue("no se pudo leer el clip");
    openEditor(clip);
    await settle();
    expect(q(".editor-loading").textContent).toContain("no se pudo leer el clip");
    // ...and the way out still works.
    expect(q(".editor")).not.toBeNull();
  });

  it("selects the whole clip, so there is nothing to save yet", async () => {
    await open(2);
    expect(startInput().value).toBe("0:00.00");
    expect(endInput().value).toBe("0:20.00");
    expect(saveButton().disabled).toBe(true);
  });
});

// ---- audio lanes ----------------------------------------------------------------

describe("audio lanes", () => {
  it("shows only the video lane for a clip without sound", async () => {
    await open(0);
    expect(qa(".tl-row-video")).toHaveLength(1);
    expect(qa(".tl-row-audio")).toHaveLength(0);
    expect(api.clipWaveform).not.toHaveBeenCalled();
    expect(q(".editor-hint").textContent).not.toContain("pista de Juego");
  });

  it("puts one lane per audio track under the video lane", async () => {
    await open(2);
    const rows = qa(".tl-row").map((r) => r.className);
    expect(rows[1]).toContain("tl-row-video");
    expect(rows[2]).toContain("tl-row-audio");
    expect(rows[3]).toContain("tl-row-audio");
    expect(lane(0).textContent).toContain("Juego");
    expect(lane(1).textContent).toContain("Micrófono");
    expect(q(".editor-hint").textContent).toContain("pista de Juego");
  });

  it("calls a single track just 'Audio'", async () => {
    await open(1);
    expect(qa(".tl-row-audio")).toHaveLength(1);
    expect(lane(0).textContent).toContain("Audio");
  });

  it("asks for the waveform of every track and draws it", async () => {
    await open(2);
    expect(api.clipWaveform).toHaveBeenCalledWith(clip.name, 0);
    expect(api.clipWaveform).toHaveBeenCalledWith(clip.name, 1);
    for (const index of [0, 1]) {
      const path = lane(index).querySelector(".tl-wave path");
      expect(path?.getAttribute("d")).toMatch(/^M0,/);
      expect(lane(index).querySelector(".tl-note")?.textContent).toBe("");
    }
  });

  it("says the wave is being drawn, and what to do if it can't be", async () => {
    let resolve!: (value: unknown) => void;
    api.clipWaveform.mockImplementation((_n: string, track: number) =>
      track === 0 ? new Promise((r) => (resolve = r)) : Promise.reject("sin decodificador"),
    );
    await open(2);
    expect(lane(0).querySelector(".tl-note")?.textContent).toContain("Dibujando");
    expect(lane(1).querySelector(".tl-note")?.textContent).toContain("No se pudo dibujar");
    // A lane without a wave is still a lane you can switch off.
    expect(lane(1).querySelector(".switch-input")).not.toBeNull();
    resolve(wave());
    await settle();
    expect(lane(0).querySelector(".tl-wave")).not.toBeNull();
  });

  it("leaves a lane out of the copy when its switch is turned off", async () => {
    await open(2);
    const toggle = lane(1).querySelector<HTMLInputElement>(".switch-input")!;
    expect(toggle.checked).toBe(true);
    expect(toggle.getAttribute("aria-label")).toContain("Micrófono");

    toggle.checked = false;
    toggle.dispatchEvent(new Event("change"));
    expect(lane(1).classList.contains("tl-off")).toBe(true);
    expect(lane(1).querySelector(".tl-note")?.textContent).toContain("No se incluye");
    expect(saveButton().disabled).toBe(false);

    api.trimClip.mockResolvedValue({ name: "x-recorte.mp4", duration: 20, bytes: 1 });
    saveButton().click();
    await settle();
    expect(api.trimClip).toHaveBeenCalledWith({
      name: clip.name,
      start: 0,
      end: 20,
      keepAudio: [0],
    });
  });

  it("silences the preview when the first track is left out", async () => {
    await open(2);
    expect(video().muted).toBe(false);
    const toggle = lane(0).querySelector<HTMLInputElement>(".switch-input")!;
    toggle.checked = false;
    toggle.dispatchEvent(new Event("change"));
    expect(video().muted).toBe(true);
    toggle.checked = true;
    toggle.dispatchEvent(new Event("change"));
    expect(video().muted).toBe(false);
  });
});

// ---- cutting ----------------------------------------------------------------------

describe("cutting", () => {
  it("moves the start handle with the pointer, snapped to a frame", async () => {
    await open(1);
    sizeTimeline();
    drag(q(".tl-handle-start"), 5);
    expect(startInput().value).toBe("0:05.00");
    expect(q<HTMLElement>(".tl-handle-start").style.left).toBe("25%");
    expect(q<HTMLElement>(".tl-shade-start").style.width).toBe("25%");
    expect(q(".editor-length").textContent).toContain("0:15.00");
    expect(saveButton().disabled).toBe(false);
  });

  it("moves the end handle and keeps it after the start", async () => {
    await open(1);
    sizeTimeline();
    drag(q(".tl-handle-start"), 8);
    drag(q(".tl-handle-end"), 15);
    expect(endInput().value).toBe("0:15.00");
    expect(q<HTMLElement>(".tl-shade-end").style.width).toBe("25%");

    drag(q(".tl-handle-end"), 2); // dragged past the start
    expect(Number.parseFloat(endInput().value.split(":")[1])).toBeGreaterThan(8);
  });

  it("shows the frame the handle points at", async () => {
    await open(1);
    sizeTimeline();
    fakePlayback(video());
    drag(q(".tl-handle-start"), 6);
    expect(video().currentTime).toBeCloseTo(6, 2);
    drag(q(".tl-handle-end"), 12);
    // The last frame inside the cut, not the first one outside it.
    expect(video().currentTime).toBeCloseTo(12 - 1 / 60, 2);
  });

  it("moves a handle with the arrow keys: a frame, or a second with shift", async () => {
    await open(1);
    const start = q(".tl-handle-start");
    press("ArrowRight", {}, start);
    expect(Number.parseFloat(startInput().value.split(":")[1])).toBeCloseTo(1 / 60, 1);
    press("ArrowRight", { shiftKey: true }, start);
    expect(Number.parseFloat(startInput().value.split(":")[1])).toBeCloseTo(1.02, 1);
    press("ArrowLeft", { shiftKey: true }, start);
    press("ArrowLeft", {}, start);
    expect(startInput().value).toBe("0:00.00");
    // The key was the handle's: it must not also step the playhead.
    expect(q(".editor-time").textContent).toContain("0:00.00 /");
  });

  it("exposes the handles to assistive technology", async () => {
    await open(1);
    const start = q(".tl-handle-start");
    expect(start.getAttribute("role")).toBe("slider");
    expect(start.getAttribute("aria-valuemin")).toBe("0");
    expect(start.getAttribute("aria-valuemax")).toBe("20.00");
    expect(q(".tl-handle-end").getAttribute("aria-valuetext")).toBe("0:20.00");
  });

  it("marks in and out with the I and O keys at the playhead", async () => {
    await open(1);
    sizeTimeline();
    pointer(q(".tl-overlay"), "pointerdown", xAt(4));
    pointer(q(".tl-overlay"), "pointerup", xAt(4));
    press("i");
    pointer(q(".tl-overlay"), "pointerdown", xAt(9));
    pointer(q(".tl-overlay"), "pointerup", xAt(9));
    press("O");
    expect(startInput().value).toBe("0:04.00");
    expect(endInput().value).toBe("0:09.00");
  });

  it("clicking the timeline moves the playhead there", async () => {
    await open(1);
    sizeTimeline();
    fakePlayback(video());
    pointer(q(".tl-overlay"), "pointerdown", xAt(10));
    pointer(q(".tl-overlay"), "pointerup", xAt(10));
    expect(q(".editor-time").textContent).toContain("0:10.00 / 0:20.00");
    expect(q<HTMLElement>(".tl-playhead").style.left).toBe("50%");
    expect(video().currentTime).toBeCloseTo(10, 2);
  });

  it("accepts times typed into the boxes", async () => {
    await open(1);
    startInput().value = "0:03,5";
    startInput().dispatchEvent(new Event("change"));
    endInput().value = "12";
    endInput().dispatchEvent(new Event("change"));
    expect(startInput().value).toBe("0:03.50");
    expect(endInput().value).toBe("0:12.00");
    expect(toasts.toast).not.toHaveBeenCalled();
  });

  it("refuses text that is not a time and restores the box", async () => {
    await open(1);
    startInput().value = "mucho";
    startInput().dispatchEvent(new Event("change"));
    expect(toasts.toast).toHaveBeenCalledWith(expect.stringContaining("tiempo"), "error");
    expect(startInput().value).toBe("0:00.00");
  });

  it("'Restablecer' goes back to the whole clip and every track", async () => {
    await open(2);
    sizeTimeline();
    drag(q(".tl-handle-start"), 5);
    const toggle = lane(1).querySelector<HTMLInputElement>(".switch-input")!;
    toggle.checked = false;
    toggle.dispatchEvent(new Event("change"));
    expect(saveButton().disabled).toBe(false);

    [...qa<HTMLButtonElement>(".editor-footer .btn")].find((b) => b.textContent === "Restablecer")!.click();
    expect(startInput().value).toBe("0:00.00");
    expect(toggle.checked).toBe(true);
    expect(lane(1).classList.contains("tl-off")).toBe(false);
    expect(saveButton().disabled).toBe(true);
  });
});

// ---- playback ---------------------------------------------------------------------

describe("playback", () => {
  it("space plays from the start of the selection and pauses on the next press", async () => {
    await open(1);
    sizeTimeline();
    fakePlayback(video());
    drag(q(".tl-handle-start"), 6);
    press(" ");
    expect(video().play).toHaveBeenCalled();
    expect(video().currentTime).toBeCloseTo(6, 2);
    expect(q(".editor-transport .btn-round").getAttribute("aria-label")).toBe("Pausar");
    press(" ");
    expect(video().pause).toHaveBeenCalled();
    expect(q(".editor-transport .btn-round").getAttribute("aria-label")).toBe("Reproducir");
  });

  it("the play button does the same", async () => {
    await open(1);
    fakePlayback(video());
    q<HTMLButtonElement>(".editor-transport .btn-round").click();
    await settle();
    expect(video().play).toHaveBeenCalled();
  });

  it("steps a frame at a time with the arrow keys", async () => {
    await open(1);
    fakePlayback(video());
    press("ArrowRight");
    press("ArrowRight");
    expect(video().currentTime).toBeCloseTo(2 / 60, 3);
    press("ArrowLeft");
    expect(video().currentTime).toBeCloseTo(1 / 60, 3);
    press("ArrowRight", { shiftKey: true });
    expect(video().currentTime).toBeCloseTo(1 + 1 / 60, 3);
  });

  it("jumps to the ends of the selection with Home and End", async () => {
    await open(1);
    sizeTimeline();
    fakePlayback(video());
    drag(q(".tl-handle-start"), 4);
    drag(q(".tl-handle-end"), 9);
    press("End");
    expect(video().currentTime).toBeCloseTo(9, 2);
    press("Home");
    expect(video().currentTime).toBeCloseTo(4, 2);
  });

  it("toggles looping", async () => {
    await open(1);
    const button = q<HTMLButtonElement>(".btn-toggle");
    expect(button.getAttribute("aria-pressed")).toBe("true");
    press("l");
    expect(button.getAttribute("aria-pressed")).toBe("false");
    button.click();
    expect(button.getAttribute("aria-pressed")).toBe("true");
  });

  it("wraps around when a looping selection runs to the very end of the clip", async () => {
    await open(1);
    sizeTimeline();
    fakePlayback(video());
    drag(q(".tl-handle-start"), 8);
    video().dispatchEvent(new Event("ended"));
    expect(video().play).toHaveBeenCalledTimes(1);
    expect(video().currentTime).toBeCloseTo(8, 2);
  });

  it("stays at the end when not looping", async () => {
    await open(1);
    sizeTimeline();
    fakePlayback(video());
    press("l");
    video().dispatchEvent(new Event("ended"));
    expect(video().play).not.toHaveBeenCalled();
  });

  it("does not wrap when the selection ends before the clip does", async () => {
    await open(1);
    sizeTimeline();
    fakePlayback(video());
    drag(q(".tl-handle-end"), 15);
    video().dispatchEvent(new Event("ended"));
    expect(video().play).not.toHaveBeenCalled();
  });

  it("ignores shortcut keys while typing in a time box", async () => {
    await open(1);
    fakePlayback(video());
    press(" ", {}, startInput());
    press("i", {}, startInput());
    expect(video().play).not.toHaveBeenCalled();
    expect(startInput().value).toBe("0:00.00");
  });
});

// ---- saving -------------------------------------------------------------------------

describe("saving", () => {
  async function cut() {
    sizeTimeline();
    drag(q(".tl-handle-start"), 5);
    drag(q(".tl-handle-end"), 15);
  }

  it("sends the cut and closes when it worked", async () => {
    const closed = vi.fn();
    await open(2, closed);
    await cut();
    api.trimClip.mockResolvedValue({ name: "clip-recorte.mp4", duration: 10, bytes: 99 });

    saveButton().click();
    await settle();

    expect(api.trimClip).toHaveBeenCalledTimes(1);
    expect(api.trimClip).toHaveBeenCalledWith({
      name: clip.name,
      start: 5,
      end: 15,
      keepAudio: [0, 1],
    });
    expect(toasts.toast).toHaveBeenCalledWith(expect.stringContaining("clip-recorte"));
    expect(document.querySelector(".editor")).toBeNull();
    expect(closed).toHaveBeenCalledTimes(1);
  });

  it("disables the button while saving, so a double click saves once", async () => {
    await open(1);
    await cut();
    let finish!: (value: unknown) => void;
    api.trimClip.mockReturnValue(new Promise((r) => (finish = r)));
    saveButton().click();
    expect(saveButton().disabled).toBe(true);
    expect(saveButton().textContent).toContain("Guardando");
    saveButton().click();
    press("s", { metaKey: true });
    expect(api.trimClip).toHaveBeenCalledTimes(1);
    finish({ name: "a-recorte.mp4", duration: 10, bytes: 1 });
    await settle();
  });

  it("keeps the editor open and says why when saving fails", async () => {
    await open(1);
    await cut();
    api.trimClip.mockRejectedValue("no hay espacio en el disco");
    saveButton().click();
    await settle();
    expect(toasts.toast).toHaveBeenCalledWith("no hay espacio en el disco", "error");
    expect(document.querySelector(".editor")).not.toBeNull();
    expect(saveButton().disabled).toBe(false); // can try again
    expect(startInput().value).toBe("0:05.00"); // and the cut is still there
  });

  it("saves with the keyboard shortcut", async () => {
    await open(1);
    await cut();
    api.trimClip.mockResolvedValue({ name: "a-recorte.mp4", duration: 10, bytes: 1 });
    press("s", { ctrlKey: true });
    await settle();
    expect(api.trimClip).toHaveBeenCalledTimes(1);
  });

  it("does nothing when there is nothing to save", async () => {
    await open(1);
    saveButton().click();
    press("s", { metaKey: true });
    await settle();
    expect(api.trimClip).not.toHaveBeenCalled();
  });
});

// ---- closing --------------------------------------------------------------------------

describe("closing", () => {
  const dialog = () => document.querySelector(".dialog");
  const dialogButton = (label: string) =>
    qa<HTMLButtonElement>(".dialog-actions button").find((b) => b.textContent === label)!;

  it("closes right away when nothing changed", async () => {
    const closed = vi.fn();
    await open(1, closed);
    q<HTMLButtonElement>(".editor-bar .btn").click();
    await settle();
    expect(document.querySelector(".editor")).toBeNull();
    expect(closed).toHaveBeenCalledTimes(1);
  });

  it("Escape closes a clean editor", async () => {
    await open(1);
    press("Escape");
    await settle();
    expect(document.querySelector(".editor")).toBeNull();
  });

  it("asks before throwing away an unsaved cut", async () => {
    await open(1);
    sizeTimeline();
    drag(q(".tl-handle-start"), 5);
    q<HTMLButtonElement>(".editor-bar .btn").click();
    await settle();
    expect(dialog()?.textContent).toContain("Descartar");
    expect(document.querySelector(".editor")).not.toBeNull();

    dialogButton("Cancelar").click();
    await settle();
    expect(document.querySelector(".editor")).not.toBeNull();
    expect(startInput().value).toBe("0:05.00");

    q<HTMLButtonElement>(".editor-bar .btn").click();
    await settle();
    dialogButton("Descartar").click();
    await settle();
    expect(document.querySelector(".editor")).toBeNull();
  });

  it("asks before throwing away a removed audio track too", async () => {
    await open(2);
    const toggle = lane(1).querySelector<HTMLInputElement>(".switch-input")!;
    toggle.checked = false;
    toggle.dispatchEvent(new Event("change"));
    press("Escape");
    await settle();
    expect(dialog()).not.toBeNull();
  });

  it("releases the video file when it closes", async () => {
    const close = await open(1);
    fakePlayback(video());
    const el = video();
    close();
    expect(el.hasAttribute("src")).toBe(false);
    expect(el.load).toHaveBeenCalled();
    expect(document.querySelector(".editor")).toBeNull();
  });

  it("closing twice is harmless", async () => {
    const closed = vi.fn();
    const close = await open(1, closed);
    close();
    close();
    expect(closed).toHaveBeenCalledTimes(1);
  });

  it("a late answer from the backend after closing does nothing", async () => {
    let resolve!: (value: EditorClip) => void;
    api.describeClip.mockReturnValue(new Promise<EditorClip>((r) => (resolve = r)));
    const close = openEditor(clip);
    close();
    resolve(info(2));
    await settle();
    expect(document.querySelector(".editor")).toBeNull();
    expect(api.clipWaveform).not.toHaveBeenCalled();
  });
});
