// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MicStatus, Settings } from "../lib/settings";
import { defaultSettings } from "../lib/settings";

// ---- a fake backend ---------------------------------------------------------

type Listener<T> = (payload: T) => void;
const listeners = {
  level: [] as Listener<number>[],
  status: [] as Listener<MicStatus>[],
  settings: [] as Listener<Settings>[],
};

const healthy: MicStatus = {
  enabled: false,
  active: false,
  muted: false,
  error: null,
  noSignal: false,
  supported: true,
};

const api = vi.hoisted(() => ({
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
  listMicrophones: vi.fn(),
  microphoneStatus: vi.fn(),
  retryMicrophone: vi.fn(),
  setMeterActive: vi.fn(),
  onMicLevel: vi.fn(),
  onMicStatus: vi.fn(),
  onSettingsChanged: vi.fn(),
}));
vi.mock("../api", () => api);

function subscribe<T>(list: Listener<T>[]) {
  return (cb: Listener<T>) => {
    list.push(cb);
    return Promise.resolve(() => {});
  };
}

/** Let promise chains and 0 ms timers settle. */
const settle = async () => {
  for (let i = 0; i < 6; i++) await vi.advanceTimersByTimeAsync(1);
};

type SettingsModule = typeof import("./settings");
let createSettings: SettingsModule["createSettings"];

/** Views mounted by the current test: hidden afterwards so a recorder left
 * listening can't answer the next test's key presses. */
const mounted: { onHide(): void }[] = [];

async function mount(initial: Settings = defaultSettings(), status: MicStatus = healthy) {
  document.body.innerHTML = '<div id="toasts"></div><section id="root"></section>';
  api.getSettings.mockResolvedValue(initial);
  api.microphoneStatus.mockResolvedValue(status);
  const view = createSettings(document.getElementById("root")!);
  mounted.push(view);
  await settle();
  return view;
}

const q = <T extends Element>(selector: string) => document.querySelector(selector) as T;
const qa = <T extends Element>(selector: string) =>
  [...document.querySelectorAll(selector)] as T[];
const enabledSwitch = () => q<HTMLInputElement>('input[aria-label="Grabar el micrófono"]');
const muteSwitch = () => q<HTMLInputElement>('input[aria-label="Silenciar el micrófono"]');
const gainSlider = () => q<HTMLInputElement>('input[type="range"]');
const deviceSelect = () => q<HTMLSelectElement>("select");
const micBody = () => q<HTMLElement>('[aria-labelledby="mic-title"] .panel-body');
const banner = () => q<HTMLElement>(".banner-slot");
const lastSaved = () =>
  api.updateSettings.mock.calls[api.updateSettings.mock.calls.length - 1][0] as Settings;

beforeEach(async () => {
  vi.useFakeTimers();
  vi.stubGlobal("navigator", { userAgent: "Macintosh" });
  for (const list of Object.values(listeners)) list.length = 0;
  for (const fn of Object.values(api)) fn.mockReset();
  api.updateSettings.mockImplementation((s: Settings) => Promise.resolve(s));
  api.listMicrophones.mockResolvedValue([
    { id: "builtin", name: "Micrófono integrado", isDefault: true },
    { id: "usb", name: "Micrófono USB", isDefault: false },
  ]);
  api.setMeterActive.mockResolvedValue(undefined);
  api.retryMicrophone.mockResolvedValue(undefined);
  api.onMicLevel.mockImplementation(subscribe(listeners.level));
  api.onMicStatus.mockImplementation(subscribe(listeners.status));
  api.onSettingsChanged.mockImplementation(subscribe(listeners.settings));
  vi.resetModules();
  ({ createSettings } = (await import("./settings")) as SettingsModule);
});

afterEach(() => {
  for (const view of mounted.splice(0)) view.onHide();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function micSettings(patch: Partial<Settings["microphone"]>): Settings {
  const s = defaultSettings();
  return { ...s, microphone: { ...s.microphone, ...patch } };
}

describe("the microphone section", () => {
  it("starts switched off with the options hidden", async () => {
    await mount();
    expect(enabledSwitch().checked).toBe(false);
    expect(micBody().hidden).toBe(true);
    expect(q(".chip").textContent).toBe("Desactivado");
  });

  it("shows the saved settings", async () => {
    await mount(micSettings({ enabled: true, gainPercent: 150, mode: "separate", muted: true }), {
      ...healthy,
      enabled: true,
      active: true,
      muted: true,
    });
    expect(enabledSwitch().checked).toBe(true);
    expect(micBody().hidden).toBe(false);
    expect(gainSlider().value).toBe("150");
    expect(q(".range-value").textContent).toBe("150 % · +3,5 dB");
    expect(muteSwitch().checked).toBe(true);
    expect(qa<HTMLInputElement>('input[name="mic-mode"]').find((i) => i.checked)?.value).toBe(
      "separate",
    );
    expect(q(".mode-note").textContent).toContain("pista aparte");
  });

  it("turning it on saves at once and reveals the options", async () => {
    await mount();
    enabledSwitch().checked = true;
    enabledSwitch().dispatchEvent(new Event("change"));
    await settle();
    expect(lastSaved().microphone.enabled).toBe(true);
    expect(micBody().hidden).toBe(false);
  });

  it("lists the microphones when the screen opens and saves the choice", async () => {
    const view = await mount(micSettings({ enabled: true }));
    view.onShow();
    await settle();
    const options = qa<HTMLOptionElement>("select option").map((o) => o.textContent);
    expect(options).toEqual([
      "Predeterminado del sistema",
      "Micrófono integrado (predeterminado)",
      "Micrófono USB",
    ]);
    deviceSelect().value = "usb";
    deviceSelect().dispatchEvent(new Event("change"));
    await settle();
    expect(lastSaved().microphone.deviceId).toBe("usb");

    deviceSelect().value = "";
    deviceSelect().dispatchEvent(new Event("change"));
    await settle();
    expect(lastSaved().microphone.deviceId).toBeNull();
  });

  it("flags a saved microphone that is no longer connected", async () => {
    const view = await mount(micSettings({ enabled: true, deviceId: "gone" }));
    view.onShow();
    await settle();
    const labels = qa<HTMLOptionElement>("select option").map((o) => o.textContent);
    expect(labels).toContain("Micrófono guardado (no conectado)");
    expect(deviceSelect().value).toBe("gone");
  });

  it("switches between mixed and separate and explains each", async () => {
    await mount(micSettings({ enabled: true }));
    expect(q(".mode-note").textContent).toContain("una sola pista");
    const separate = qa<HTMLInputElement>('input[name="mic-mode"]').find((i) => i.value === "separate")!;
    separate.checked = true;
    separate.dispatchEvent(new Event("change"));
    await settle();
    expect(lastSaved().microphone.mode).toBe("separate");
    expect(q(".mode-note").textContent).toContain("solo tocan la primera pista");
  });

  it("the volume slider updates its label at once but saves once, after a pause", async () => {
    await mount(micSettings({ enabled: true }));
    for (const value of ["110", "120", "135", "140"]) {
      gainSlider().value = value;
      gainSlider().dispatchEvent(new Event("input"));
    }
    expect(q(".range-value").textContent).toBe("140 % · +2,9 dB");
    expect(api.updateSettings).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    expect(api.updateSettings).toHaveBeenCalledTimes(1);
    expect(lastSaved().microphone.gainPercent).toBe(140);
  });

  it("the mute switch saves", async () => {
    await mount(micSettings({ enabled: true }));
    muteSwitch().checked = true;
    muteSwitch().dispatchEvent(new Event("change"));
    await settle();
    expect(lastSaved().microphone.muted).toBe(true);
  });

  it("a rejected save reverts the screen and says why", async () => {
    await mount(micSettings({ enabled: true }));
    api.updateSettings.mockRejectedValueOnce("atajo no válido");
    muteSwitch().checked = true;
    muteSwitch().dispatchEvent(new Event("change"));
    await settle();
    expect(muteSwitch().checked).toBe(false);
    expect(q("#toasts").textContent).toContain("atajo no válido");
  });

  it("follows changes made elsewhere, like the mute hotkey", async () => {
    await mount(micSettings({ enabled: true }));
    listeners.settings.forEach((cb) => cb(micSettings({ enabled: true, muted: true })));
    await settle();
    expect(muteSwitch().checked).toBe(true);
  });
});

describe("health messages", () => {
  const enabled = micSettings({ enabled: true });

  it("shows why the microphone fails, with a retry", async () => {
    await mount(enabled);
    listeners.status.forEach((cb) =>
      cb({ ...healthy, enabled: true, error: "permiso de micrófono denegado" }),
    );
    await settle();
    expect(banner().textContent).toContain("No se pudo usar el micrófono");
    expect(banner().textContent).toContain("permiso de micrófono denegado");
    expect(q(".chip").className).toContain("chip-error");

    const retry = qa<HTMLButtonElement>(".banner button").find((b) => b.textContent === "Reintentar")!;
    retry.click();
    expect(api.retryMicrophone).toHaveBeenCalledTimes(1);
  });

  it("offers the default microphone when a chosen one fails", async () => {
    await mount(micSettings({ enabled: true, deviceId: "usb" }));
    listeners.status.forEach((cb) =>
      cb({ ...healthy, enabled: true, error: "el micrófono elegido no está conectado" }),
    );
    await settle();
    const useDefault = qa<HTMLButtonElement>(".banner button").find(
      (b) => b.textContent === "Usar el predeterminado",
    )!;
    useDefault.click();
    await settle();
    expect(lastSaved().microphone.deviceId).toBeNull();
  });

  it("warns when nothing is heard", async () => {
    await mount(enabled);
    listeners.status.forEach((cb) =>
      cb({ ...healthy, enabled: true, active: true, noSignal: true }),
    );
    await settle();
    expect(banner().textContent).toContain("No se oye nada");
    expect(banner().textContent).toContain("permiso");
  });

  it("clears the banner when the problem goes away", async () => {
    await mount(enabled);
    listeners.status.forEach((cb) => cb({ ...healthy, enabled: true, error: "x" }));
    await settle();
    listeners.status.forEach((cb) => cb({ ...healthy, enabled: true, active: true }));
    await settle();
    expect(banner().textContent).toBe("");
    expect(q(".chip").textContent).toBe("Grabando tu voz");
  });

  it("says so on a platform without microphone support", async () => {
    await mount(enabled, { ...healthy, supported: false });
    expect(banner().textContent).toContain("todavía no soporta el micrófono");
  });
});

describe("the mute shortcut", () => {
  const keydown = (init: KeyboardEventInit & { code: string }) =>
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  const shortcutButton = () => q<HTMLButtonElement>('[data-shortcut="mute-mic"]');
  const note = () => q('[data-shortcut-note="mute-mic"]');

  it("shows the current shortcut in the platform's notation", async () => {
    await mount(micSettings({ enabled: true }));
    expect(shortcutButton().textContent).toBe("⇧⌘M");
  });

  it("records a new combination and saves it", async () => {
    await mount(micSettings({ enabled: true }));
    shortcutButton().click();
    expect(shortcutButton().textContent).toBe("Presioná la combinación…");

    keydown({ code: "MetaLeft", key: "Meta", metaKey: true }); // only a modifier: keep waiting
    expect(shortcutButton().textContent).toBe("Presioná la combinación…");

    keydown({ code: "KeyK", key: "k", metaKey: true, shiftKey: true });
    await settle();
    expect(lastSaved().microphone.muteShortcut).toBe("CommandOrControl+Shift+K");
    expect(shortcutButton().textContent).toBe("⇧⌘K");
  });

  it("refuses a shortcut that another feature owns", async () => {
    await mount(micSettings({ enabled: true }));
    shortcutButton().click();
    keydown({ code: "KeyS", key: "s", metaKey: true, shiftKey: true });
    await settle();
    expect(api.updateSettings).not.toHaveBeenCalled();
    expect(note().textContent).toContain("Guardar clip");
    expect(shortcutButton().textContent).toBe("Presioná la combinación…"); // still recording
  });

  it("explains an unusable combination and keeps listening", async () => {
    await mount(micSettings({ enabled: true }));
    shortcutButton().click();
    keydown({ code: "KeyK", key: "k" }); // no modifier
    expect(note().textContent).toContain("Combiná");
    expect(shortcutButton().classList.contains("recording")).toBe(true);
  });

  it("Escape cancels without changing anything", async () => {
    await mount(micSettings({ enabled: true }));
    shortcutButton().click();
    keydown({ code: "Escape", key: "Escape" });
    await settle();
    expect(api.updateSettings).not.toHaveBeenCalled();
    expect(shortcutButton().textContent).toBe("⇧⌘M");
  });

  it("leaving the screen stops recording", async () => {
    const view = await mount(micSettings({ enabled: true }));
    view.onShow();
    shortcutButton().click();
    view.onHide();
    keydown({ code: "KeyK", key: "k", metaKey: true });
    await settle();
    expect(api.updateSettings).not.toHaveBeenCalled();
  });
});

describe("the save-clip shortcut", () => {
  const keydown = (init: KeyboardEventInit & { code: string }) =>
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  const saveButton = () => q<HTMLButtonElement>('[data-shortcut="save-clip"]');
  const muteButton = () => q<HTMLButtonElement>('[data-shortcut="mute-mic"]');
  const note = () => q('[data-shortcut-note="save-clip"]');
  const resetButton = () =>
    qa<HTMLButtonElement>(".panel .btn").find((b) => b.textContent === "Restablecer")!;
  const withSave = (saveClip: string, micro: Partial<Settings["microphone"]> = {}): Settings => {
    const s = micSettings(micro);
    return { ...s, shortcuts: { ...s.shortcuts, saveClip } };
  };

  it("is there even with the microphone off, in the platform's notation", async () => {
    await mount();
    expect(enabledSwitch().checked).toBe(false);
    expect(saveButton().textContent).toBe("⇧⌘S");
    expect(q('[aria-labelledby="shortcuts-title"]').textContent).toContain("Guardar clip");
  });

  it("records a new combination and saves it without touching the microphone", async () => {
    await mount(micSettings({ enabled: true, gainPercent: 130 }));
    saveButton().click();
    expect(saveButton().textContent).toBe("Presioná la combinación…");

    keydown({ code: "F9", key: "F9", altKey: true });
    await settle();
    expect(lastSaved().shortcuts.saveClip).toBe("Alt+F9");
    expect(lastSaved().microphone.gainPercent).toBe(130);
    expect(lastSaved().microphone.muteShortcut).toBe("CommandOrControl+Shift+M");
    expect(saveButton().textContent).toBe("⌥F9");
  });

  it("refuses the keys the microphone's mute shortcut uses", async () => {
    await mount();
    saveButton().click();
    keydown({ code: "KeyM", key: "m", metaKey: true, shiftKey: true });
    await settle();
    expect(api.updateSettings).not.toHaveBeenCalled();
    expect(note().textContent).toContain("Silenciar el micrófono");
    expect(saveButton().textContent).toBe("Presioná la combinación…"); // still recording
  });

  it("accepts a single key that types nothing, like F9", async () => {
    await mount();
    saveButton().click();
    keydown({ code: "F9", key: "F9" });
    await settle();
    expect(lastSaved().shortcuts.saveClip).toBe("F9");
    expect(saveButton().textContent).toBe("F9");
    expect(resetButton().hidden).toBe(false);
  });

  it("accepts the other keys that don't write, with their Spanish names", async () => {
    await mount();
    for (const [code, saved, shown] of [
      ["Insert", "Insert", "Insert"],
      ["Home", "Home", "Inicio"],
      ["PageUp", "PageUp", "RePág"],
      ["Pause", "Pause", "Pausa"],
    ]) {
      saveButton().click();
      keydown({ code, key: code });
      await settle();
      expect(lastSaved().shortcuts.saveClip).toBe(saved);
      expect(saveButton().textContent).toBe(shown);
    }
  });

  it("refuses a letter on its own and says what to do", async () => {
    await mount();
    saveButton().click();
    keydown({ code: "KeyK", key: "k" });
    await settle();
    expect(api.updateSettings).not.toHaveBeenCalled();
    expect(note().textContent).toContain("Sola, esa tecla dejaría de funcionar");
    expect(note().textContent).toContain("F1–F24");
    expect(saveButton().textContent).toBe("Presioná la combinación…"); // keeps listening
    // ...and the same letter with a modifier is fine.
    keydown({ code: "KeyK", key: "k", metaKey: true });
    await settle();
    expect(lastSaved().shortcuts.saveClip).toBe("CommandOrControl+K");
  });

  it("explains the rule in the panel", async () => {
    await mount();
    const text = q('[aria-labelledby="shortcuts-title"] .panel-intro').textContent!;
    expect(text).toContain("tecla sola");
    expect(text).toContain("⌘, ⌃ o ⌥");
  });

  it("the mute shortcut can be a single key too, but not the save shortcut's", async () => {
    await mount(withSave("F9", { enabled: true }));
    muteButton().click();
    keydown({ code: "F9", key: "F9" });
    await settle();
    expect(api.updateSettings).not.toHaveBeenCalled();
    expect(q('[data-shortcut-note="mute-mic"]').textContent).toContain("Guardar clip");
    keydown({ code: "F10", key: "F10" });
    await settle();
    expect(lastSaved().microphone.muteShortcut).toBe("F10");
    expect(muteButton().textContent).toBe("F10");
  });

  it("the mute shortcut refuses whatever the save shortcut was changed to", async () => {
    await mount(withSave("Alt+F9", { enabled: true }));
    muteButton().click();
    keydown({ code: "F9", key: "F9", altKey: true });
    await settle();
    expect(api.updateSettings).not.toHaveBeenCalled();
    expect(q('[data-shortcut-note="mute-mic"]').textContent).toContain("Guardar clip");
    // ...and the old save keys are free for the mute shortcut now.
    keydown({ code: "KeyS", key: "s", metaKey: true, shiftKey: true });
    await settle();
    expect(lastSaved().microphone.muteShortcut).toBe("CommandOrControl+Shift+S");
  });

  it("explains an unusable combination and keeps listening", async () => {
    await mount();
    saveButton().click();
    keydown({ code: "KeyK", key: "k" });
    expect(note().textContent).toContain("Combiná");
    expect(saveButton().classList.contains("recording")).toBe(true);
  });

  it("Escape cancels without changing anything", async () => {
    await mount();
    saveButton().click();
    keydown({ code: "Escape", key: "Escape" });
    await settle();
    expect(api.updateSettings).not.toHaveBeenCalled();
    expect(saveButton().textContent).toBe("⇧⌘S");
    expect(note().textContent).toBe("");
  });

  it("recording one shortcut stops recording the other", async () => {
    await mount(micSettings({ enabled: true }));
    muteButton().click();
    saveButton().click();
    expect(muteButton().textContent).toBe("⇧⌘M");
    expect(saveButton().textContent).toBe("Presioná la combinación…");

    keydown({ code: "KeyK", key: "k", metaKey: true, shiftKey: true });
    await settle();
    expect(lastSaved().shortcuts.saveClip).toBe("CommandOrControl+Shift+K");
    expect(lastSaved().microphone.muteShortcut).toBe("CommandOrControl+Shift+M");
  });

  it("offers to go back to the usual shortcut only when it was changed", async () => {
    await mount();
    expect(resetButton().hidden).toBe(true);

    saveButton().click();
    keydown({ code: "F9", key: "F9", altKey: true });
    await settle();
    expect(resetButton().hidden).toBe(false);

    resetButton().click();
    await settle();
    expect(lastSaved().shortcuts.saveClip).toBe("CommandOrControl+Shift+S");
    expect(saveButton().textContent).toBe("⇧⌘S");
    expect(resetButton().hidden).toBe(true);
  });

  it("a shortcut the system refuses is reverted and the reason is shown", async () => {
    await mount();
    api.updateSettings.mockRejectedValueOnce("no se pudo registrar el atajo «Alt+F9»");
    saveButton().click();
    keydown({ code: "F9", key: "F9", altKey: true });
    await settle();
    expect(q("#toasts").textContent).toContain("no se pudo registrar");
    expect(saveButton().textContent).toBe("⇧⌘S");
    expect(resetButton().hidden).toBe(true);
  });

  it("follows a change made elsewhere", async () => {
    await mount();
    listeners.settings.forEach((cb) => cb(withSave("Alt+F9")));
    await settle();
    expect(saveButton().textContent).toBe("⌥F9");
  });

  it("leaving the screen stops recording", async () => {
    const view = await mount();
    view.onShow();
    saveButton().click();
    view.onHide();
    keydown({ code: "KeyK", key: "k", metaKey: true });
    await settle();
    expect(api.updateSettings).not.toHaveBeenCalled();
    expect(saveButton().textContent).toBe("⇧⌘S");
  });
});

describe("the level meter", () => {
  it("only streams while the screen is open", async () => {
    const view = await mount(micSettings({ enabled: true }));
    view.onShow();
    expect(api.setMeterActive).toHaveBeenLastCalledWith(true);
    view.onHide();
    expect(api.setMeterActive).toHaveBeenLastCalledWith(false);
  });

  it("moves with the incoming level and falls back afterwards", async () => {
    const view = await mount(micSettings({ enabled: true }));
    view.onShow();
    await vi.advanceTimersByTimeAsync(50);
    listeners.level.forEach((cb) => cb(0.5));
    await vi.advanceTimersByTimeAsync(40);
    const loud = parseFloat(q<HTMLElement>(".meter-fill").style.width);
    expect(loud).toBeGreaterThan(50);

    await vi.advanceTimersByTimeAsync(1500);
    const quiet = parseFloat(q<HTMLElement>(".meter-fill").style.width);
    expect(quiet).toBeLessThan(loud);
    view.onHide();
  });

  it("stays flat while muted", async () => {
    const view = await mount(micSettings({ enabled: true, muted: true }));
    view.onShow();
    await vi.advanceTimersByTimeAsync(50);
    listeners.level.forEach((cb) => cb(0.9));
    await vi.advanceTimersByTimeAsync(100);
    expect(parseFloat(q<HTMLElement>(".meter-fill").style.width)).toBe(0);
    view.onHide();
  });
});

describe("the library section", () => {
  const detectSwitch = () =>
    q<HTMLInputElement>('input[aria-label="Detectar el juego al guardar un clip"]');
  const libraryPanel = () => q<HTMLElement>('[aria-labelledby="library-title"]');

  function libSettings(detectGame: boolean): Settings {
    const s = defaultSettings();
    return { ...s, library: { detectGame } };
  }

  it("shows game detection on by default", async () => {
    await mount();
    expect(detectSwitch().checked).toBe(true);
  });

  it("shows what was saved", async () => {
    await mount(libSettings(false));
    expect(detectSwitch().checked).toBe(false);
  });

  it("switching it off saves at once and changes nothing else", async () => {
    await mount();
    detectSwitch().checked = false;
    detectSwitch().dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(api.updateSettings).toHaveBeenCalledTimes(1);
    const saved = lastSaved();
    expect(saved.library.detectGame).toBe(false);
    expect(saved.microphone).toEqual(defaultSettings().microphone);
    expect(saved.shortcuts).toEqual(defaultSettings().shortcuts);
  });

  it("switching it back on saves that too", async () => {
    await mount(libSettings(false));
    detectSwitch().checked = true;
    detectSwitch().dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(lastSaved().library.detectGame).toBe(true);
  });

  it("follows a change made elsewhere", async () => {
    await mount();
    listeners.settings.forEach((cb) => cb(libSettings(false)));
    await settle();
    expect(detectSwitch().checked).toBe(false);
  });

  it("says what is recorded and that it stays on this computer", async () => {
    await mount();
    const text = libraryPanel().textContent!;
    expect(text).toContain("nombre de la aplicación");
    expect(text).toContain("esta computadora");
  });
});
