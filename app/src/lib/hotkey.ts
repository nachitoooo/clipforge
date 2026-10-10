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

/**
 * Keys that may be a shortcut on their own: none of them types a character or
 * edits text, so claiming one system-wide costs nothing anyone writes with.
 * Mirrors `works_alone` in `settings.rs`; every other key needs a modifier.
 */
const WORKS_ALONE = /^(F([1-9]|1\d|2[0-4])|Insert|Home|End|PageUp|PageDown|Pause|ScrollLock|PrintScreen)$/;

export function worksAlone(keyToken: string): boolean {
  return WORKS_ALONE.test(keyToken);
}

/** Named keys the plugin's parser understands, besides letters, digits and F-keys. */
const NAMED_KEYS =
  /^(Space|Enter|Tab|Backspace|Delete|Insert|Home|End|PageUp|PageDown|Pause|ScrollLock|PrintScreen|Backquote|Minus|Equal|BracketLeft|BracketRight|Backslash|Semicolon|Comma|Period|Slash|Arrow(Up|Down|Left|Right)|Numpad(\d|Add|Subtract|Multiply|Divide|Decimal|Enter|Equal))$/;

/** The key part of the accelerator for a physical key, or `null` if unusable. */
function keyToken(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit(\d)$/.exec(code);
  if (digit) return digit[1];
  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code;
  if (NAMED_KEYS.test(code)) return code;
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

  if (modifiers.length === 0 && !worksAlone(key)) {
    return {
      kind: "invalid",
      reason: `Sola, esa tecla dejaría de funcionar en las demás aplicaciones. Combiná la tecla con ${isMac ? "⌘, ⌃ o ⌥" : "Ctrl, Alt o Win"}, o usá una de función (F1–F24), Insert, Inicio, Fin, RePág, AvPág, Pausa o ImprPant`,
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
  BACKSPACE: "Retroceso",
  DELETE: "Supr",
  INSERT: "Insert",
  HOME: "Inicio",
  END: "Fin",
  PAGEUP: "RePág",
  PAGEDOWN: "AvPág",
  PAUSE: "Pausa",
  PRINTSCREEN: "ImprPant",
  SCROLLLOCK: "BloqDesp",
  ENTER: "Enter",
  TAB: "Tab",
  BACKQUOTE: "`",
  MINUS: "-",
  EQUAL: "=",
  BRACKETLEFT: "[",
  BRACKETRIGHT: "]",
  BACKSLASH: "\\",
  SEMICOLON: ";",
  COMMA: ",",
  PERIOD: ".",
  SLASH: "/",
  NUMPADADD: "Núm +",
  NUMPADSUBTRACT: "Núm -",
  NUMPADMULTIPLY: "Núm *",
  NUMPADDIVIDE: "Núm /",
  NUMPADDECIMAL: "Núm .",
  NUMPADENTER: "Núm Enter",
  NUMPADEQUAL: "Núm =",
};

/** What a key is called on the button: `K`, `F9`, `Inicio`, `Núm 0`. */
function keyName(key: string): string {
  const numpadDigit = /^NUMPAD(\d)$/.exec(key);
  return numpadDigit ? `Núm ${numpadDigit[1]}` : (KEY_NAMES[key] ?? key);
}

/** `⌘⇧M` on a Mac, `Ctrl+Shift+M` elsewhere. */
export function displayAccelerator(accelerator: string, isMac: boolean): string {
  const normalized = normalizeAccelerator(accelerator).split("+");
  const key = normalized.pop() ?? "";
  const shownKey = keyName(key);
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
