import * as api from "../api";
import { ownerOf } from "../lib/hotkey";
import { PeakMeter, peakToPercent } from "../lib/level";
import {
  GAIN_MAX_PERCENT,
  DEFAULT_SAVE_SHORTCUT,
  GAIN_MIN_PERCENT,
  MAX_EXTRA_SAVE_SHORTCUTS,
  MIC_MODE_LABEL,
  MIC_MODE_NOTE,
  clampGain,
  clipLengthOptions,
  defaultSettings,
  formatClipLength,
  formatGain,
  memoryModeText,
  micStateSummary,
  shortcutOwners,
  withAddedShortcut,
  withExtraShortcut,
  withMicrophone,
  withAppearance,
  withLibrary,
  withPerformance,
  withShortcuts,
  withoutExtraShortcut,
  type AppearanceSettings,
  type LibrarySettings,
  type MemoryPlans,
  type MicDevice,
  type MicMode,
  type MicStatus,
  type MicrophoneSettings,
  type PerformanceSettings,
  type Settings,
  type ShortcutSettings,
  type ThemeChoice,
} from "../lib/settings";
import { ACCENT_PRESETS, THEME_CHOICES, normalizeHex } from "../lib/theme";
import { h, svg } from "./dom";
import { icons } from "./icons";
import { createShortcutRecorder, type ShortcutRecorder } from "./shortcut";
import { confirmDialog } from "./modal";
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
    conflict: (accelerator) => ownerOf(accelerator, shortcutOwners(settings, "mute")),
    onChange: (muteShortcut) => change({ muteShortcut }),
  });

  // ---- global shortcuts panel -------------------------------------------

  /** A length picker; the options follow the value so a hand-set one shows too. */
  function lengthSelect(label: string, onPick: (seconds: number) => void) {
    const el = h("select", { class: "select select-sm", "aria-label": label });
    el.addEventListener("change", () => onPick(Number(el.value)));
    return {
      el,
      set(seconds: number): void {
        const options = clipLengthOptions(seconds);
        if (el.dataset.options !== options.join(",")) {
          el.dataset.options = options.join(",");
          el.replaceChildren(
            ...options.map((s) => h("option", { value: String(s) }, formatClipLength(s))),
          );
        }
        el.value = String(seconds);
      },
    };
  }

  /** A row whose hint is rewritten as the settings change. */
  function liveRow(label: string, control: HTMLElement) {
    const hint = h("div", { class: "row-hint muted" });
    const el = h(
      "div",
      { class: "row" },
      h("div", { class: "row-text" }, h("div", { class: "row-label" }, label), hint),
      control,
    );
    return { el, hint };
  }

  const saveRecorder = createShortcutRecorder({
    id: "save-clip",
    isMac,
    title: "Cambiar el atajo para guardar un clip",
    current: () => settings.shortcuts.saveClip,
    conflict: (accelerator) => ownerOf(accelerator, shortcutOwners(settings, "save")),
    onChange: (saveClip) => changeShortcuts({ saveClip }),
  });
  const saveLength = lengthSelect("Duración del clip", (clipSeconds) =>
    changeShortcuts({ clipSeconds }),
  );
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
  const saveRow = liveRow(
    "Guardar clip",
    h("div", { class: "mute-controls" }, saveLength.el, saveRecorder.button, resetSaveShortcut),
  );

  // Extra save shortcuts, each with its own length. The rows are rebuilt only
  // when a shortcut is added or removed (a length changes in place, so the
  // picker keeps its focus). `draft` is a row still waiting for its keys.
  interface ExtraRow {
    /** The row and the note under it: siblings, so the row separators line up. */
    els: HTMLElement[];
    recorder: ShortcutRecorder;
    sync(): void;
  }
  let extraRows: ExtraRow[] = [];
  let extraKey = "";
  let draft: { seconds: number } | null = null;
  const extraSlot = h("div", { class: "extra-shortcuts" });

  function buildExtraRow(index: number): ExtraRow {
    const isDraft = index >= settings.shortcuts.extraSaveClip.length;
    const extra = () => settings.shortcuts.extraSaveClip[index];
    const seconds = () => (isDraft ? (draft?.seconds ?? 0) : (extra()?.seconds ?? 0));
    const recorder = createShortcutRecorder({
      id: `save-extra-${index}`,
      isMac,
      title: "Cambiar el atajo",
      current: () => (isDraft ? "" : (extra()?.accelerator ?? "")),
      empty: "Elegí una tecla",
      conflict: (accelerator) =>
        ownerOf(accelerator, shortcutOwners(settings, isDraft ? null : { extra: index })),
      onChange: (accelerator) => {
        if (isDraft) {
          const added = withAddedShortcut(settings, { accelerator, seconds: seconds() });
          draft = null;
          update(added, false);
        } else {
          update(withExtraShortcut(settings, index, { accelerator }), false);
        }
      },
    });
    const length = lengthSelect("Duración del clip de este atajo", (picked) => {
      if (isDraft) {
        // Choosing a length is not choosing keys: stop listening so the
        // arrow keys still work on the picker.
        recorder.stop();
        draft = { seconds: picked };
        row.sync();
      } else {
        update(withExtraShortcut(settings, index, { seconds: picked }), false);
      }
    });
    const remove = h(
      "button",
      {
        class: "btn",
        title: "Quitar este atajo",
        "data-remove-extra": String(index),
        onclick: () => {
          if (isDraft) {
            draft = null;
            renderValues();
          } else {
            update(withoutExtraShortcut(settings, index), false);
          }
        },
      },
      "Quitar",
    );
    const live = liveRow(
      "Otro atajo para guardar",
      h("div", { class: "mute-controls" }, length.el, recorder.button, remove),
    );
    const row: ExtraRow = {
      els: [live.el, recorder.note],
      recorder,
      sync() {
        length.set(seconds());
        live.hint.textContent = `Guarda los últimos ${formatClipLength(seconds())} del buffer`;
        recorder.render();
      },
    };
    return row;
  }

  const addShortcut = h(
    "button",
    {
      class: "btn btn-nowrap",
      "data-add-shortcut": "",
      onclick: () => {
        draft = { seconds: 15 };
        renderValues();
        extraRows[extraRows.length - 1]?.recorder.button.click();
      },
    },
    "Agregar atajo",
  );
  const addRow = row(
    "Más atajos",
    `Hasta ${MAX_EXTRA_SAVE_SHORTCUTS} más, cada uno con su duración: por ejemplo F9 para los últimos 15 segundos y F10 para el último minuto.`,
    addShortcut,
  );

  function renderExtras(): void {
    const count = settings.shortcuts.extraSaveClip.length;
    const key = `${settings.shortcuts.extraSaveClip.map((e) => e.accelerator).join("|")}#${draft !== null}`;
    if (key !== extraKey) {
      extraKey = key;
      for (const old of extraRows) old.recorder.stop();
      extraRows = Array.from({ length: count + (draft ? 1 : 0) }, (_, i) => buildExtraRow(i));
      extraSlot.replaceChildren(...extraRows.flatMap((r) => r.els));
    }
    for (const r of extraRows) r.sync();
    addRow.hidden = count >= MAX_EXTRA_SAVE_SHORTCUTS || draft !== null;
  }

  // The screenshot shortcut: none until one is chosen.
  const screenshotRecorder = createShortcutRecorder({
    id: "screenshot",
    isMac,
    title: "Elegir el atajo para sacar una captura de pantalla",
    current: () => settings.shortcuts.screenshot,
    empty: "Sin atajo",
    conflict: (accelerator) => ownerOf(accelerator, shortcutOwners(settings, "screenshot")),
    onChange: (screenshot) => changeShortcuts({ screenshot }),
  });
  const clearScreenshot = h(
    "button",
    {
      class: "btn",
      title: "Quitar el atajo (la captura sigue en la bandeja y en la ventana)",
      "data-clear-screenshot": "",
      onclick: () => {
        screenshotRecorder.stop();
        changeShortcuts({ screenshot: "" });
      },
    },
    "Quitar",
  );
  const screenshotRow = row(
    "Captura de pantalla",
    "Guarda una imagen de toda la pantalla junto a tus clips. Sin atajo también se puede sacar desde la bandeja y desde el botón «Captura» de Recientes.",
    h("div", { class: "mute-controls" }, screenshotRecorder.button, clearScreenshot),
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
      "Funcionan desde cualquier aplicación o juego, aunque ClipForge esté en segundo plano. Podés usar una tecla sola si es de función (F1–F24) o una que no escribe (Insert, Inicio, Fin, RePág, AvPág, Pausa, ImprPant); las demás necesitan ",
      isMac ? "⌘, ⌃ o ⌥" : "Ctrl, Alt o Win",
      ". Un clip no puede durar más de lo que el buffer ya juntó (hasta 2 minutos).",
    ),
    h(
      "div",
      { class: "panel-body" },
      saveRow.el,
      saveRecorder.note,
      extraSlot,
      addRow,
      screenshotRow,
      screenshotRecorder.note,
    ),
  );

  // ---- appearance panel -------------------------------------------------

  const themeIcons: Record<ThemeChoice, string> = {
    system: icons.monitor,
    dark: icons.moon,
    light: icons.sun,
  };
  const themeButtons = THEME_CHOICES.map((choice) =>
    h(
      "button",
      {
        type: "button",
        class: "segment",
        role: "radio",
        "aria-checked": "false",
        "data-theme": choice.id,
        onclick: () => changeAppearance({ theme: choice.id }),
      },
      svg(themeIcons[choice.id]),
      choice.label,
    ),
  );
  const swatchButtons = ACCENT_PRESETS.map((preset) =>
    h("button", {
      type: "button",
      class: "swatch",
      role: "radio",
      title: preset.label,
      "aria-label": preset.label,
      "aria-checked": "false",
      "data-accent": preset.hex,
      style: `background:${preset.hex}`,
      onclick: () => changeAppearance({ accent: preset.hex }),
    }),
  );
  const hexInput = h("input", {
    type: "text",
    class: "text-input hex-input",
    "aria-label": "Color en hexadecimal",
    placeholder: "#ff7a1a",
    maxlength: "7",
    spellcheck: "false",
    autocomplete: "off",
  });
  hexInput.addEventListener("input", () => {
    const hex = normalizeHex(hexInput.value);
    hexInput.classList.toggle("is-invalid", hexInput.value.trim() !== "" && hex === null);
    if (hex !== null && hex !== settings.appearance.accent) changeAppearance({ accent: hex });
  });
  // Leaving the box with half a color in it puts the real one back.
  hexInput.addEventListener("blur", () => {
    hexInput.classList.remove("is-invalid");
    hexInput.value = settings.appearance.accent;
  });
  const appearancePanel = h(
    "section",
    { class: "panel", "aria-labelledby": "appearance-title" },
    h(
      "div",
      { class: "panel-head" },
      h(
        "div",
        { class: "panel-title" },
        svg(icons.palette),
        h("h2", { id: "appearance-title" }, "Apariencia"),
      ),
    ),
    h(
      "div",
      { class: "panel-body" },
      row(
        "Tema",
        "Claro, oscuro, o el mismo que use tu sistema.",
        h("div", { class: "segmented", role: "radiogroup", "aria-label": "Tema" }, ...themeButtons),
      ),
      h(
        "div",
        { class: "row row-stack" },
        h(
          "div",
          { class: "row-text" },
          h("div", { class: "row-label" }, "Color principal"),
          h("div", { class: "row-hint muted" }, "Se usa en los detalles: la sección activa, los interruptores, los bordes al enfocar."),
        ),
        h(
          "div",
          { class: "accent-picker" },
          h("div", { class: "swatches", role: "radiogroup", "aria-label": "Color principal" }, ...swatchButtons),
          hexInput,
        ),
      ),
    ),
  );

  // ---- library panel ----------------------------------------------------

  const detectSwitch = switchControl("Detectar el juego al guardar un clip", (checked) =>
    changeLibrary({ detectGame: checked }),
  );
  const autoFoldersSwitch = switchControl("Archivar cada clip en la carpeta de su juego", (checked) =>
    changeLibrary({ autoFolders: checked }),
  );
  const autoFoldersRow = liveRow("Archivar cada clip en la carpeta de su juego", autoFoldersSwitch.el);
  const libraryPanel = h(
    "section",
    { class: "panel", "aria-labelledby": "library-title" },
    h(
      "div",
      { class: "panel-head" },
      h(
        "div",
        { class: "panel-title" },
        svg(icons.library),
        h("h2", { id: "library-title" }, "Biblioteca"),
      ),
    ),
    h(
      "div",
      { class: "panel-body" },
      row(
        "Detectar el juego al guardar un clip",
        "Anota qué aplicación estaba en primer plano mientras se grababa, para buscar y filtrar tus clips por juego. Solo guarda el nombre de la aplicación, no lo que se ve en pantalla, y se queda en esta computadora. Los clips que ya tenés no cambian; el juego de cada uno se puede corregir a mano.",
        detectSwitch.el,
      ),
      autoFoldersRow.el,
    ),
  );

  // ---- performance panel --------------------------------------------------

  let plans: MemoryPlans | null = null;
  const lowRamSwitch = switchControl("Modo poca memoria (RAM)", (checked) => void askLowRam(checked));
  const lowRamRow = liveRow("Modo poca memoria (RAM)", lowRamSwitch.el);
  const lowRamWarning = h(
    "p",
    { class: "row-hint muted" },
    "Cambiarlo reinicia la grabación y vacía el buffer: lo que se grabó hasta ese momento se pierde. Los clips ya guardados no cambian.",
  );
  const performancePanel = h(
    "section",
    { class: "panel", "aria-labelledby": "performance-title" },
    h(
      "div",
      { class: "panel-head" },
      h(
        "div",
        { class: "panel-title" },
        svg(icons.bolt),
        h("h2", { id: "performance-title" }, "Rendimiento"),
      ),
    ),
    h("div", { class: "panel-body" }, lowRamRow.el, lowRamWarning),
  );

  /** Switching it restarts the capture and empties the buffer: ask first. */
  async function askLowRam(checked: boolean): Promise<void> {
    const ok = await confirmDialog({
      title: checked ? "Activar el modo poca memoria" : "Volver al modo normal",
      message:
        "Se reinicia la grabación y el buffer actual se vacía: lo que grabaste en los últimos minutos se pierde. Los clips ya guardados no cambian.",
      confirmLabel: "Reiniciar la grabación",
    });
    if (ok) {
      changePerformance({ lowRam: checked });
      toast(
        checked
          ? "Modo poca memoria activado: la grabación se reinicia"
          : "Modo normal: la grabación se reinicia",
      );
    } else {
      lowRamSwitch.input.checked = settings.performance.lowRam; // as it was
    }
  }

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
    h("div", { class: "view-body" }, h("div", { class: "settings" }, appearancePanel, shortcutsPanel, panel, libraryPanel, performancePanel)),
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

  function renderAppearance(): void {
    const { theme, accent } = settings.appearance;
    for (const button of themeButtons) button.setAttribute("aria-checked", String(button.dataset.theme === theme));
    for (const button of swatchButtons) button.setAttribute("aria-checked", String(button.dataset.accent === accent));
    if (document.activeElement !== hexInput) hexInput.value = accent;
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
    saveLength.set(settings.shortcuts.clipSeconds);
    saveRow.hint.textContent = `Guarda los últimos ${formatClipLength(settings.shortcuts.clipSeconds)} del buffer`;
    renderExtras();
    screenshotRecorder.render();
    clearScreenshot.hidden = settings.shortcuts.screenshot === "";
    resetSaveShortcut.hidden = settings.shortcuts.saveClip === DEFAULT_SAVE_SHORTCUT;
    renderAppearance();
    detectSwitch.input.checked = settings.library.detectGame;
    lowRamSwitch.input.checked = settings.performance.lowRam;
    lowRamRow.hint.textContent = memoryModeText(plans, settings.performance.lowRam);
    autoFoldersSwitch.input.checked = settings.library.autoFolders;
    autoFoldersSwitch.input.disabled = !settings.library.detectGame;
    autoFoldersRow.hint.textContent =
      "Al guardar un clip lo pone en una carpeta con el nombre de su juego. La crea si no existe; si ya tenés una con ese nombre, la usa tal cual. Los clips que ya tenés no se mueven, y si borrás una de estas carpetas, el próximo clip de ese juego la vuelve a crear." +
      (settings.library.detectGame ? "" : " Necesita que la detección del juego esté activada.");
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

  function changePerformance(patch: Partial<PerformanceSettings>): void {
    update(withPerformance(settings, patch), false);
  }

  function changeAppearance(patch: Partial<AppearanceSettings>): void {
    update(withAppearance(settings, patch), false);
  }

  function changeLibrary(patch: Partial<LibrarySettings>): void {
    update(withLibrary(settings, patch), false);
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
      [settings, status, plans] = await Promise.all([
        api.getSettings(),
        api.microphoneStatus(),
        api.memoryPlans().catch(() => null),
      ]);
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
      screenshotRecorder.stop();
      for (const r of extraRows) r.recorder.stop();
      if (draft) {
        draft = null;
        renderValues();
      }
      void api.setMeterActive(false);
      meter.reset();
    },
  };
}
