/**
 * Turning key presses into global-shortcut accelerators
 * (`CommandOrControl+Shift+M`), the format `tauri-plugin-global-shortcut`
 * parses, and back into something readable (`⌘⇧M` / `Ctrl+Shift+M`).
 */
export interface KeyEventLike {
  key: string;
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** A shortcut some feature already uses. */
export interface ShortcutOwner {
  accelerator: string;
  /** What the feature is called in messages, e.g. `Guardar clip`. */
  owner: string;
}

export type CaptureResult =
  | { kind: "waiting" } // only modifiers so far
  | { kind: "cancel" }
  | { kind: "accelerator"; accelerator: string }
  | { kind: "invalid"; reason: string };

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
  "OS",
]);

/** The key part of the accelerator for a physical key, or `null` if unusable. */
function keyToken(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit(\d)$/.exec(code);
  if (digit) return digit[1];
  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code;
  if (code === "Space") return "Space";
  if (/^Arrow(Up|Down|Left|Right)$/.test(code)) return code;
  return null;
}

/** Interpret one `keydown` while the user is recording a shortcut. */
export function captureAccelerator(e: KeyEventLike, isMac: boolean): CaptureResult {
  const bareEscape = e.code === "Escape" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey;
  if (bareEscape) return { kind: "cancel" };
  if (MODIFIER_CODES.has(e.code)) return { kind: "waiting" };

  const key = keyToken(e.code);
  if (key === null) {
    return { kind: "invalid", reason: "Esa tecla no se puede usar en un atajo" };
  }

  const modifiers: string[] = [];
  if (isMac) {
    if (e.metaKey) modifiers.push("CommandOrControl");
    if (e.ctrlKey) modifiers.push("Control");
  } else {
    if (e.ctrlKey) modifiers.push("CommandOrControl");
    if (e.metaKey) modifiers.push("Super");
  }
  if (e.altKey) modifiers.push("Alt");

  if (modifiers.length === 0) {
    return {
      kind: "invalid",
      reason: `Combiná la tecla con ${isMac ? "⌘, ⌃ o ⌥" : "Ctrl, Alt o Win"}`,
    };
  }
  if (e.shiftKey) modifiers.push("Shift");
  return { kind: "accelerator", accelerator: [...modifiers, key].join("+") };
}

const MODIFIER_ORDER = ["commandorcontrol", "command", "control", "super", "alt", "shift"];

const ALIASES: Record<string, string> = {
  cmdorctrl: "commandorcontrol",
  commandorcontrol: "commandorcontrol",
  cmd: "command",
  command: "command",
  ctrl: "control",
  control: "control",
  meta: "super",
  win: "super",
  super: "super",
  option: "alt",
  alt: "alt",
  shift: "shift",
};

/** Canonical form for comparing accelerators written differently. */
export function normalizeAccelerator(accelerator: string): string {
  const parts = accelerator
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
  const key = parts.pop()?.toUpperCase() ?? "";
  const mods = parts
    .map((p) => ALIASES[p.toLowerCase()] ?? p.toLowerCase())
    .sort((a, b) => MODIFIER_ORDER.indexOf(a) - MODIFIER_ORDER.indexOf(b));
  return [...new Set(mods), key].join("+");
}

export function sameAccelerator(a: string, b: string): boolean {
  return normalizeAccelerator(a) === normalizeAccelerator(b);
}

const MAC_GLYPHS: Record<string, string> = {
  commandorcontrol: "⌘",
  command: "⌘",
  control: "⌃",
  super: "⌘",
  alt: "⌥",
  shift: "⇧",
};
const PC_NAMES: Record<string, string> = {
  commandorcontrol: "Ctrl",
  command: "Win",
  control: "Ctrl",
  super: "Win",
  alt: "Alt",
  shift: "Shift",
};
const KEY_NAMES: Record<string, string> = {
  ARROWUP: "↑",
  ARROWDOWN: "↓",
  ARROWLEFT: "←",
  ARROWRIGHT: "→",
  SPACE: "Espacio",
};

/** `⌘⇧M` on a Mac, `Ctrl+Shift+M` elsewhere. */
export function displayAccelerator(accelerator: string, isMac: boolean): string {
  const normalized = normalizeAccelerator(accelerator).split("+");
  const key = normalized.pop() ?? "";
  const shownKey = KEY_NAMES[key] ?? key;
  if (isMac) {
    const order = ["control", "alt", "shift", "command", "commandorcontrol", "super"];
    const glyphs = normalized
      .sort((a, b) => order.indexOf(a) - order.indexOf(b))
      .map((m) => MAC_GLYPHS[m] ?? m);
    return glyphs.join("") + shownKey;
  }
  return [...normalized.map((m) => PC_NAMES[m] ?? m), shownKey].join("+");
}

/** Name of the feature in `taken` that already uses `accelerator`, if any. */
export function ownerOf(accelerator: string, taken: ShortcutOwner[]): string | null {
  const hit = taken.find((t) => sameAccelerator(t.accelerator, accelerator));
  return hit?.owner ?? null;
}
