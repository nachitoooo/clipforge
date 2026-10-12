import type { ThemeChoice } from "./settings";

/** The color the app has always had. Mirrors `DEFAULT_ACCENT` in `settings.rs`. */
export const DEFAULT_ACCENT = "#ff7a1a";

export interface AccentPreset {
  id: string;
  label: string;
  hex: string;
}

/** Colors offered as swatches; any other `#rrggbb` can be typed. */
export const ACCENT_PRESETS: readonly AccentPreset[] = [
  { id: "orange", label: "Naranja", hex: "#ff7a1a" },
  { id: "red", label: "Rojo", hex: "#ff4d5e" },
  { id: "pink", label: "Rosa", hex: "#ff6fb5" },
  { id: "purple", label: "Violeta", hex: "#a77bff" },
  { id: "blue", label: "Azul", hex: "#4da3ff" },
  { id: "teal", label: "Turquesa", hex: "#2ec4b6" },
  { id: "green", label: "Verde", hex: "#3ddc84" },
  { id: "yellow", label: "Amarillo", hex: "#ffd166" },
  { id: "graphite", label: "Grafito", hex: "#8b93a7" },
];

export const THEME_CHOICES: readonly { id: ThemeChoice; label: string }[] = [
  { id: "system", label: "Sistema" },
  { id: "dark", label: "Oscuro" },
  { id: "light", label: "Claro" },
];

/** What is drawn in the end: `system` has been resolved. */
export type Scheme = "dark" | "light";

/**
 * Mirrors `normalize_accent` in `settings.rs`: `#RRGGBB`, with or without the
 * hash and in any case, as lowercase `#rrggbb`; `null` for anything else.
 */
export function normalizeHex(text: string): string | null {
  const trimmed = text.trim();
  const digits = trimmed.startsWith("#") ? trimmed.slice(1) : trimmed;
  return /^[0-9a-fA-F]{6}$/.test(digits) ? `#${digits.toLowerCase()}` : null;
}

export type Rgb = [number, number, number];

export function parseHex(hex: string): Rgb {
  const n = normalizeHex(hex) ?? DEFAULT_ACCENT;
  return [1, 3, 5].map((i) => parseInt(n.slice(i, i + 2), 16)) as Rgb;
}

export function toHex([r, g, b]: Rgb): string {
  return `#${[r, g, b].map((v) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0")).join("")}`;
}

/** WCAG relative luminance, 0 (black) to 1 (white). */
export function luminance(hex: string): number {
  const [r, g, b] = parseHex(hex).map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio, 1 (none) to 21 (black on white). */
export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** `hex` moved `amount` (0–1) of the way towards `target`. */
export function mix(hex: string, target: string, amount: number): string {
  const from = parseHex(hex);
  const to = parseHex(target);
  return toHex(from.map((v, i) => v + (to[i] - v) * amount) as Rgb);
}

/** The surface text sits on, per scheme (`--bg-card` in the stylesheet). */
const SURFACE: Record<Scheme, string> = { dark: "#14161d", light: "#ffffff" };

/**
 * The accent as text, an icon or a thin line: the color itself when it reads
 * well on the card, otherwise the nearest lighter (dark scheme) or darker
 * (light scheme) shade that does. Yellow on white would not.
 */
export function readableAccent(hex: string, scheme: Scheme, minimum = 4.5): string {
  const toward = scheme === "dark" ? "#ffffff" : "#000000";
  const surface = SURFACE[scheme];
  for (let step = 0; step <= 20; step++) {
    const candidate = mix(hex, toward, step * 0.05);
    if (contrast(candidate, surface) >= minimum) return candidate;
  }
  return toward;
}

/** The custom properties the stylesheet takes from the accent; the rest is derived there with `color-mix`. */
export function accentVars(hex: string, scheme: Scheme): Record<string, string> {
  const accent = normalizeHex(hex) ?? DEFAULT_ACCENT;
  return {
    "--accent": accent,
    "--accent-strong": mix(accent, "#ffffff", 0.18),
    "--accent-text": readableAccent(accent, scheme),
  };
}

/** `system` follows the operating system; the other two are what they say. */
export function resolveScheme(theme: ThemeChoice, systemPrefersLight: boolean): Scheme {
  if (theme === "light") return "light";
  if (theme === "dark") return "dark";
  return systemPrefersLight ? "light" : "dark";
}
