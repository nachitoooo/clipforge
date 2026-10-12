import type { AppearanceSettings } from "../lib/settings";
import { accentVars, resolveScheme, type Scheme } from "../lib/theme";

export interface Appearance {
  /** Show this look now. */
  apply(appearance: AppearanceSettings): void;
  /** What is drawn at the moment (`system` already resolved). */
  scheme(): Scheme;
}

/**
 * Puts the chosen theme and color on the page: `data-theme` on the root element
 * (the stylesheet has a set of colors per theme) and the accent as custom
 * properties. With the `system` theme it also follows the operating system when
 * that changes while the app is open.
 */
export function createAppearance(root: HTMLElement = document.documentElement): Appearance {
  const system =
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: light)")
      : null;
  let current: AppearanceSettings | null = null;
  let drawn: Scheme = "dark";

  function render() {
    if (!current) return;
    drawn = resolveScheme(current.theme, system?.matches ?? false);
    root.dataset.theme = drawn;
    for (const [name, value] of Object.entries(accentVars(current.accent, drawn))) {
      root.style.setProperty(name, value);
    }
  }

  system?.addEventListener?.("change", () => {
    if (current?.theme === "system") render();
  });

  return {
    apply(appearance) {
      current = { ...appearance };
      render();
    },
    scheme: () => drawn,
  };
}
