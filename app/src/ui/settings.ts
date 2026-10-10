import * as api from "../api";
import { ownerOf } from "../lib/hotkey";
import { PeakMeter, peakToPercent } from "../lib/level";
import {
  GAIN_MAX_PERCENT,
  DEFAULT_SAVE_SHORTCUT,
  GAIN_MIN_PERCENT,
  MIC_MODE_LABEL,
  MIC_MODE_NOTE,
  clampGain,
  defaultSettings,
  formatGain,
  micStateSummary,
  withMicrophone,
  withShortcuts,
  type MicDevice,
  type MicMode,
  type MicStatus,
  type MicrophoneSettings,
  type Settings,
  type ShortcutSettings,
} from "../lib/settings";
import { h, svg } from "./dom";
import { icons } from "./icons";
import { createShortcutRecorder } from "./shortcut";
import { switchControl } from "./switch";
import { toast } from "./toast";

export interface SettingsView {
  /** The Settings screen became visible. */
  onShow(): void;
  onHide(): void;
}

const isMac = navigator.userAgent.includes("Mac");
const SAVE_DEBOUNCE_MS = 150;

/** A labelled row: text on the left, the control on the right. */
function row(label: string, hint: string | null, control: HTMLElement): HTMLElement {
  return h(
    "div",
    { class: "row" },
    h(
      "div",
      { class: "row-text" },
      h("div", { class: "row-label" }, label),
      hint ? h("div", { class: "row-hint muted" }, hint) : null,
    ),
    control,
  );
}

export function createSettings(root: HTMLElement): SettingsView {
  let settings: Settings = defaultSettings();
  let lastSaved: Settings = settings;
  let status: MicStatus | null = null;
  let devices: MicDevice[] = [];
  let visible = false;
  let saveTimer: number | undefined;
  let queue: Promise<void> = Promise.resolve();

  // ---- controls (built once, then updated in place so a slider being
  // dragged is never torn down) ------------------------------------------

  const enabledSwitch = switchControl("Grabar el micrófono", (checked) =>
    change({ enabled: checked }),
  );
  const stateChip = h("span", { class: "chip" });
  const banner = h("div", { class: "banner-slot" });

  const deviceSelect = h("select", { class: "select", "aria-label": "Micrófono" });
  deviceSelect.addEventListener("change", () =>
    change({ deviceId: deviceSelect.value === "" ? null : deviceSelect.value }),
  );

  const modeInputs = new Map<MicMode, HTMLInputElement>();
  const modeCards = h(
    "div",
    { class: "mode-cards", role: "radiogroup", "aria-label": "Cómo se guarda tu voz" },
    ...(["mixed", "separate"] as MicMode[]).map((mode) => {
      const input = h("input", { type: "radio", name: "mic-mode", value: mode });
      input.addEventListener("change", () => input.checked && change({ mode }));
      modeInputs.set(mode, input);
      return h("label", { class: "mode-card" }, input, h("span", {}, MIC_MODE_LABEL[mode]));
    }),
  );
  const modeNote = h("p", { class: "mode-note muted" });

  const gainSlider = h("input", {
    type: "range",
    class: "range",
    min: String(GAIN_MIN_PERCENT),
    max: String(GAIN_MAX_PERCENT),
    step: "1",
    "aria-label": "Volumen del micrófono",
  });
  const gainValue = h("span", { class: "range-value" });
  gainSlider.addEventListener("input", () => {
    const gainPercent = clampGain(Number(gainSlider.value));
    gainValue.textContent = formatGain(gainPercent);
    change({ gainPercent }, true);
  });

  const meterFill = h("div", { class: "meter-fill" });
  const meterHold = h("div", { class: "meter-hold" });
  const meterHint = h("div", { class: "row-hint muted" });
  const meterBar = h("div", { class: "meter", role: "meter", "aria-label": "Nivel del micrófono" }, meterFill, meterHold);

  const muteSwitch = switchControl("Silenciar el micrófono", (checked) =>
    change({ muted: checked }),
  );
  const muteRecorder = createShortcutRecorder({
    id: "mute-mic",
    isMac,
    title: "Cambiar el atajo para silenciar",
    current: () => settings.microphone.muteShortcut,
    conflict: (accelerator) =>
      ownerOf(accelerator, [{ accelerator: settings.shortcuts.saveClip, owner: "Guardar clip" }]),
    onChange: (muteShortcut) => change({ muteShortcut }),
  });

  // ---- global shortcuts panel -------------------------------------------

  const saveRecorder = createShortcutRecorder({
    id: "save-clip",
    isMac,
    title: "Cambiar el atajo para guardar un clip",
    current: () => settings.shortcuts.saveClip,
    conflict: (accelerator) =>
      ownerOf(accelerator, [
        { accelerator: settings.microphone.muteShortcut, owner: "Silenciar el micrófono" },
      ]),
    onChange: (saveClip) => changeShortcuts({ saveClip }),
  });
  const resetSaveShortcut = h(
    "button",
    {
      class: "btn",
      title: "Volver al atajo de siempre",
      onclick: () => {
        saveRecorder.stop();
        changeShortcuts({ saveClip: DEFAULT_SAVE_SHORTCUT });
      },
    },
    "Restablecer",
  );
  const shortcutsPanel = h(
    "section",
    { class: "panel", "aria-labelledby": "shortcuts-title" },
    h(
      "div",
      { class: "panel-head" },
      h(
        "div",
        { class: "panel-title" },
        svg(icons.keyboard),
        h("h2", { id: "shortcuts-title" }, "Atajos"),
      ),
    ),
    h(
      "p",
      { class: "panel-intro muted" },
      "Funcionan desde cualquier aplicación o juego, aunque ClipForge esté en segundo plano.",
    ),
    h(
      "div",
      { class: "panel-body" },
      row(
        "Guardar clip",
        "Guarda los últimos 30 segundos del buffer",
        h("div", { class: "mute-controls" }, saveRecorder.button, resetSaveShortcut),
      ),
      saveRecorder.note,
    ),
  );

  const optionsBody = h(
    "div",
    { class: "panel-body" },
    row("Dispositivo", "Qué micrófono usar", deviceSelect),
    h(
      "div",
      { class: "row row-stack" },
      h(
        "div",
        { class: "row-text" },
        h("div", { class: "row-label" }, "Cómo se guarda tu voz en el clip"),
      ),
      modeCards,
      modeNote,
      h(
        "p",
        { class: "row-hint muted" },
        "Sin auriculares, el micrófono puede captar el sonido del juego y se oiría repetido.",
      ),
    ),
    row("Volumen", "Subí o bajá tu voz respecto al juego", h("div", { class: "range-wrap" }, gainSlider, gainValue)),
    h(
      "div",
      { class: "row row-stack" },
      h("div", { class: "row-text" }, h("div", { class: "row-label" }, "Nivel")),
      meterBar,
      meterHint,
    ),
    row(
      "Silenciar",
      "Corta tu voz sin apagar el micrófono",
      h("div", { class: "mute-controls" }, muteSwitch.el, muteRecorder.button),
    ),
    muteRecorder.note,
  );

  const panel = h(
    "section",
    { class: "panel", "aria-labelledby": "mic-title" },
    h(
      "div",
      { class: "panel-head" },
      h(
        "div",
        { class: "panel-title" },
        svg(icons.mic),
        h("h2", { id: "mic-title" }, "Micrófono"),
        stateChip,
      ),
      enabledSwitch.el,
    ),
    h(
      "p",
      { class: "panel-intro muted" },
      "Grabá tu voz junto con el juego. Solo funciona mientras el buffer está grabando, y nunca sale de tu computadora.",
    ),
    banner,
    optionsBody,
  );

  root.replaceChildren(
    h(
      "header",
      { class: "view-header" },
      h("div", {}, h("h1", {}, "Configuración"), h("p", { class: "muted" }, "Cambios con efecto inmediato")),
    ),
    h("div", { class: "view-body" }, h("div", { class: "settings" }, shortcutsPanel, panel)),
  );

  // ---- rendering from state --------------------------------------------

  function renderDevices(): void {
    const wanted = settings.microphone.deviceId;
    const known = wanted === null || devices.some((d) => d.id === wanted);
    deviceSelect.replaceChildren(
      h("option", { value: "" }, "Predeterminado del sistema"),
      ...devices.map((d) => h("option", { value: d.id }, d.isDefault ? `${d.name} (predeterminado)` : d.name)),
      ...(known ? [] : [h("option", { value: wanted ?? "" }, "Micrófono guardado (no conectado)")]),
    );
    deviceSelect.value = wanted ?? "";
  }

  function renderBanner(): void {
    const parts: HTMLElement[] = [];
    if (status && status.enabled && status.error) {
      parts.push(
        h(
          "div",
          { class: "banner", role: "alert" },
          h("div", {}, h("strong", {}, "No se pudo usar el micrófono. "), h("span", {}, status.error)),
          h(
            "div",
            { class: "banner-actions" },
            settings.microphone.deviceId !== null
              ? h("button", { class: "btn", onclick: () => change({ deviceId: null }) }, "Usar el predeterminado")
              : null,
            h("button", { class: "btn", onclick: () => void api.retryMicrophone() }, "Reintentar"),
          ),
        ),
      );
    } else if (status && status.noSignal) {
      parts.push(
        h(
          "div",
          { class: "banner banner-warn", role: "status" },
          h(
            "div",
            {},
            h("strong", {}, "No se oye nada. "),
            h(
              "span",
              {},
              "Si estás hablando, revisá que ClipForge tenga permiso para usar el micrófono (macOS: Ajustes del Sistema → Privacidad y seguridad → Micrófono; Windows: Privacidad → Micrófono).",
            ),
          ),
        ),
      );
    } else if (status && !status.supported) {
      parts.push(
        h("div", { class: "banner banner-warn" }, "Este sistema operativo todavía no soporta el micrófono."),
      );
    }
    banner.replaceChildren(...parts);
  }

  function renderValues(): void {
    const mic = settings.microphone;
    enabledSwitch.input.checked = mic.enabled;
    optionsBody.hidden = !mic.enabled;
    const summary = micStateSummary(status);
    stateChip.textContent = summary.text;
    stateChip.className = `chip chip-${summary.tone}`;

    renderDevices();
    for (const [mode, input] of modeInputs) input.checked = mic.mode === mode;
    modeNote.textContent = MIC_MODE_NOTE[mic.mode];
    if (document.activeElement !== gainSlider) gainSlider.value = String(mic.gainPercent);
    gainValue.textContent = formatGain(mic.gainPercent);
    muteSwitch.input.checked = mic.muted;
    muteRecorder.render();
    saveRecorder.render();
    resetSaveShortcut.hidden = settings.shortcuts.saveClip === DEFAULT_SAVE_SHORTCUT;
    meterHint.textContent = !mic.enabled
      ? ""
      : mic.muted
        ? "Silenciado: no se graba tu voz."
        : "Hablá para ver la barra moverse.";
    renderBanner();
  }

  // ---- saving ----------------------------------------------------------

  function change(patch: Partial<MicrophoneSettings>, debounce = false): void {
    update(withMicrophone(settings, patch), debounce);
  }

  function changeShortcuts(patch: Partial<ShortcutSettings>): void {
    update(withShortcuts(settings, patch), false);
  }

  function update(next: Settings, debounce: boolean): void {
    settings = next;
    renderValues();
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => {
      queue = queue.then(save);
    }, debounce ? SAVE_DEBOUNCE_MS : 0);
  }

  async function save(): Promise<void> {
    const toSave = settings;
    try {
      const saved = await api.updateSettings(toSave);
      lastSaved = saved;
      // Adopt the backend's normalized copy unless the user kept editing.
      if (settings === toSave) {
        settings = saved;
        renderValues();
      }
    } catch (e) {
      toast(String(e), "error");
      settings = lastSaved;
      renderValues();
    }
  }

  // ---- live level meter --------------------------------------------------

  const meter = new PeakMeter();
  let latestPeak: number | null = null;
  let frame = 0;

  function animate(now: number): void {
    const reading = latestPeak === null ? null : peakToPercent(latestPeak);
    latestPeak = null;
    meter.update(settings.microphone.enabled && !settings.microphone.muted ? reading : null, now);
    meterFill.style.width = `${meter.level}%`;
    meterHold.style.left = `${meter.hold}%`;
    meterHold.style.opacity = meter.hold > 1 ? "1" : "0";
    meterBar.setAttribute("aria-valuenow", String(Math.round(meter.level)));
    frame = visible ? requestAnimationFrame(animate) : 0;
  }

  // ---- events and loading ------------------------------------------------

  void api.onMicLevel((peak) => {
    latestPeak = Math.max(latestPeak ?? 0, peak);
  });
  void api.onMicStatus((next) => {
    status = next;
    renderValues();
  });
  void api.onSettingsChanged((next) => {
    // Another path changed them (the mute hotkey, ...): follow along.
    if (JSON.stringify(next) !== JSON.stringify(settings)) {
      settings = next;
      lastSaved = next;
      renderValues();
    }
  });

  async function load(): Promise<void> {
    try {
      [settings, status] = await Promise.all([api.getSettings(), api.microphoneStatus()]);
      lastSaved = settings;
    } catch (e) {
      toast(String(e), "error");
    }
    renderValues();
  }

  async function refreshDevices(): Promise<void> {
    try {
      devices = await api.listMicrophones();
    } catch (e) {
      devices = [];
      toast(String(e), "error");
    }
    renderValues();
  }

  renderValues();
  void load();

  return {
    onShow() {
      visible = true;
      void api.setMeterActive(true);
      void refreshDevices();
      void api.microphoneStatus().then((s) => {
        status = s;
        renderValues();
      });
      if (frame === 0) frame = requestAnimationFrame(animate);
    },
    onHide() {
      visible = false;
      muteRecorder.stop();
      saveRecorder.stop();
      void api.setMeterActive(false);
      meter.reset();
    },
  };
}
