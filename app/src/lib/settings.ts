/** Mirrors `settings.rs` / `mic.rs` in src-tauri (JSON is camelCase). */
export type MicMode = "mixed" | "separate";

export interface MicrophoneSettings {
  enabled: boolean;
  /** `null` follows the system default input. */
  deviceId: string | null;
  mode: MicMode;
  gainPercent: number;
  muted: boolean;
  /** Global accelerator that toggles `muted`, e.g. `CommandOrControl+Shift+M`. */
  muteShortcut: string;
}

/** A save shortcut with a length of its own. */
export interface ClipShortcut {
  accelerator: string;
  seconds: number;
}

export interface ShortcutSettings {
  /** Global accelerator that saves a clip, e.g. `CommandOrControl+Shift+S`. */
  saveClip: string;
  /** How much the main shortcut, the tray entry and the window's button keep. */
  clipSeconds: number;
  /** More save shortcuts, each with its own length. */
  extraSaveClip: ClipShortcut[];
  /** Takes a screenshot; empty = no shortcut (the default). */
  screenshot: string;
}

export interface LibrarySettings {
  /**
   * Tag each saved clip with the application that was in front while it was
   * recorded (its game). Only the application's name, kept on this machine.
   */
  detectGame: boolean;
  /**
   * File each new clip in a folder named after its game (made if missing). Off
   * by default, and without effect while `detectGame` is off.
   */
  autoFolders: boolean;
}

export interface PerformanceSettings {
  /**
   * One minute of video at half the bitrate instead of two minutes: the replay
   * buffer takes a quarter of the memory. Changing it restarts the capture and
   * empties the buffer.
   */
  lowRam: boolean;
}

export interface Settings {
  version: number;
  shortcuts: ShortcutSettings;
  microphone: MicrophoneSettings;
  library: LibrarySettings;
  performance: PerformanceSettings;
}

/** What one memory mode costs; mirrors `memory::PlanInfo` in `src-tauri/src/memory.rs`. */
export interface PlanInfo {
  bufferSeconds: number;
  /** Average video bitrate, bits per second. */
  bitrate: number;
  /** About how much a full buffer weighs, in bytes. */
  estimatedBytes: number;
}

export interface MemoryPlans {
  normal: PlanInfo;
  lowRam: PlanInfo;
}

export interface MicStatus {
  enabled: boolean;
  active: boolean;
  muted: boolean;
  error: string | null;
  noSignal: boolean;
  supported: boolean;
}

export interface MicDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export const GAIN_MIN_PERCENT = 0;
export const GAIN_MAX_PERCENT = 200;
export const DEFAULT_GAIN_PERCENT = 100;
export const DEFAULT_SAVE_SHORTCUT = "CommandOrControl+Shift+S";
export const DEFAULT_MUTE_SHORTCUT = "CommandOrControl+Shift+M";
export const DEFAULT_CLIP_SECONDS = 30;
export const MIN_CLIP_SECONDS = 5;
/** The replay buffer's length: a clip can't be longer than what it holds. */
export const MAX_CLIP_SECONDS = 120;
/** Save shortcuts besides the main one. */
export const MAX_EXTRA_SAVE_SHORTCUTS = 4;
/** The lengths offered in the pickers. */
export const CLIP_LENGTH_CHOICES = [15, 30, 60, 120];

export function defaultSettings(): Settings {
  return {
    version: 1,
    shortcuts: { saveClip: DEFAULT_SAVE_SHORTCUT, clipSeconds: DEFAULT_CLIP_SECONDS, extraSaveClip: [], screenshot: "" },
    microphone: {
      enabled: false,
      deviceId: null,
      mode: "mixed",
      gainPercent: DEFAULT_GAIN_PERCENT,
      muted: false,
      muteShortcut: DEFAULT_MUTE_SHORTCUT,
    },
    library: { detectGame: true, autoFolders: false },
    performance: { lowRam: false },
  };
}

/** Whole percent inside the allowed range; non-numbers fall back to 100. */
export function clampGain(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_GAIN_PERCENT;
  return Math.min(GAIN_MAX_PERCENT, Math.max(GAIN_MIN_PERCENT, Math.round(value)));
}

/** Gain in decibels; `-Infinity` at 0 %. */
export function gainToDb(percent: number): number {
  const linear = clampGain(percent) / 100;
  return linear === 0 ? -Infinity : 20 * Math.log10(linear);
}

/** `100 % · 0 dB`, `150 % · +3,5 dB`, `0 % · silencio` (decimal comma). */
export function formatGain(percent: number): string {
  const p = clampGain(percent);
  if (p === 0) return "0 % · silencio";
  const db = gainToDb(p);
  const rounded = Math.round(db * 10) / 10;
  const text = Math.abs(rounded).toFixed(1).replace(".", ",");
  const sign = rounded > 0 ? "+" : rounded < 0 ? "−" : "";
  return `${p} % · ${sign}${text} dB`;
}

/** A copy of `settings` with the microphone section patched. */
export function withMicrophone(
  settings: Settings,
  patch: Partial<MicrophoneSettings>,
): Settings {
  return { ...settings, microphone: { ...settings.microphone, ...patch } };
}

/** A copy of `settings` with the shortcuts section patched. */
export function withShortcuts(settings: Settings, patch: Partial<ShortcutSettings>): Settings {
  return { ...settings, shortcuts: { ...settings.shortcuts, ...patch } };
}

/** A length as people say it: `30 s`, `1 min`, `1 min 30 s`. */
export function formatClipLength(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  if (minutes === 0) return `${rest} s`;
  return rest === 0 ? `${minutes} min` : `${minutes} min ${rest} s`;
}

/** The lengths to offer: the usual ones, plus `current` when it is none of them. */
export function clipLengthOptions(current: number): number[] {
  return CLIP_LENGTH_CHOICES.includes(current)
    ? CLIP_LENGTH_CHOICES
    : [...CLIP_LENGTH_CHOICES, current].sort((a, b) => a - b);
}

/** A copy of `settings` with one extra save shortcut patched. */
export function withExtraShortcut(
  settings: Settings,
  index: number,
  patch: Partial<ClipShortcut>,
): Settings {
  const extras = settings.shortcuts.extraSaveClip.map((extra, i) =>
    i === index ? { ...extra, ...patch } : extra,
  );
  return withShortcuts(settings, { extraSaveClip: extras });
}

/** A copy of `settings` with one more extra save shortcut (at the end). */
export function withAddedShortcut(settings: Settings, added: ClipShortcut): Settings {
  return withShortcuts(settings, {
    extraSaveClip: [...settings.shortcuts.extraSaveClip, added],
  });
}

/** A copy of `settings` without the extra save shortcut at `index`. */
export function withoutExtraShortcut(settings: Settings, index: number): Settings {
  return withShortcuts(settings, {
    extraSaveClip: settings.shortcuts.extraSaveClip.filter((_, i) => i !== index),
  });
}

/**
 * Every global shortcut in use and what it does, for refusing a repeat. The
 * extra save shortcut at `except` (the one being recorded) is left out.
 */
export function shortcutOwners(
  settings: Settings,
  except: { extra: number } | "save" | "mute" | "screenshot" | null = null,
): { accelerator: string; owner: string }[] {
  const owners: { accelerator: string; owner: string }[] = [];
  if (except !== "save") {
    owners.push({
      accelerator: settings.shortcuts.saveClip,
      owner: `Guardar clip (${formatClipLength(settings.shortcuts.clipSeconds)})`,
    });
  }
  settings.shortcuts.extraSaveClip.forEach((extra, i) => {
    if (typeof except === "object" && except !== null && except.extra === i) return;
    owners.push({
      accelerator: extra.accelerator,
      owner: `Guardar clip (${formatClipLength(extra.seconds)})`,
    });
  });
  if (except !== "mute") {
    owners.push({
      accelerator: settings.microphone.muteShortcut,
      owner: "Silenciar el micrófono",
    });
  }
  if (except !== "screenshot" && settings.shortcuts.screenshot !== "") {
    owners.push({ accelerator: settings.shortcuts.screenshot, owner: "Captura de pantalla" });
  }
  return owners;
}

/** A copy of `settings` with the performance section patched. */
export function withPerformance(
  settings: Settings,
  patch: Partial<PerformanceSettings>,
): Settings {
  return { ...settings, performance: { ...settings.performance, ...patch } };
}

/** `180 MB`, `45 MB` (decimal, rounded: it is an estimate). */
export function formatMemory(bytes: number): string {
  return `${Math.round(bytes / 1_000_000)} MB`;
}

/** `12 Mb/s`, `6 Mb/s`, `7,5 Mb/s`. */
export function formatBitrate(bitsPerSecond: number): string {
  const mbps = bitsPerSecond / 1_000_000;
  const text = Number.isInteger(mbps) ? String(mbps) : mbps.toFixed(1).replace(".", ",");
  return `${text} Mb/s`;
}

function describePlan(plan: PlanInfo): string {
  return `${formatClipLength(plan.bufferSeconds)} de buffer a ${formatBitrate(plan.bitrate)} (unos ${formatMemory(plan.estimatedBytes)} de RAM)`;
}

/** What the low RAM switch does, in words, for the mode that is on now. */
export function memoryModeText(plans: MemoryPlans | null, lowRam: boolean): string {
  if (plans === null) {
    return "Guarda menos tiempo de video y con menos calidad para ocupar bastante menos RAM.";
  }
  const { normal, lowRam: low } = plans;
  return lowRam
    ? `Activado: ${describePlan(low)} en vez de ${describePlan(normal)}. Los clips duran hasta ${formatClipLength(low.bufferSeconds)}, aunque pidas más, y la imagen es un poco más blanda en escenas rápidas.`
    : `Ahora: ${describePlan(normal)}. Activado: ${describePlan(low)}, con clips de hasta ${formatClipLength(low.bufferSeconds)} y la imagen un poco más blanda en escenas rápidas.`;
}

/** A copy of `settings` with the library section patched. */
export function withLibrary(settings: Settings, patch: Partial<LibrarySettings>): Settings {
  return { ...settings, library: { ...settings.library, ...patch } };
}

export const MIC_MODE_LABEL: Record<MicMode, string> = {
  mixed: "Mezclado",
  separate: "Pista separada",
};

/** What each way of saving the microphone means for the person using it. */
export const MIC_MODE_NOTE: Record<MicMode, string> = {
  mixed:
    "Tu voz y el juego van juntos en una sola pista. Se escucha en cualquier lado (Discord, WhatsApp, el navegador), pero después no se puede silenciar la voz.",
  separate:
    "Tu voz va en una pista aparte, así se puede silenciar después. Ojo: muchos reproductores (Discord, los navegadores) solo tocan la primera pista y no se oiría tu voz al compartir el clip.",
};

export type Tone = "off" | "ok" | "warn" | "error";

/** One line + colour summarizing the microphone's health. */
export function micStateSummary(status: MicStatus | null): { tone: Tone; text: string } {
  if (status === null) return { tone: "off", text: "Comprobando…" };
  if (!status.supported) {
    return { tone: "off", text: "Este sistema todavía no soporta el micrófono" };
  }
  if (!status.enabled) return { tone: "off", text: "Desactivado" };
  if (status.error) return { tone: "error", text: "No funciona" };
  if (!status.active) {
    return { tone: "warn", text: "Se activa cuando empieza la grabación" };
  }
  if (status.muted) return { tone: "warn", text: "Silenciado" };
  if (status.noSignal) return { tone: "warn", text: "Activo, pero no se oye nada" };
  return { tone: "ok", text: "Grabando tu voz" };
}
