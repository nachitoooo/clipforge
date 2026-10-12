import { describe, expect, it } from "vitest";
import {
  ACCENT_PRESETS,
  DEFAULT_ACCENT,
  accentVars,
  contrast,
  luminance,
  mix,
  normalizeHex,
  parseHex,
  readableAccent,
  resolveScheme,
  toHex,
} from "./theme";
import { defaultSettings } from "./settings";

describe("normalizeHex (the same rule as the Rust side)", () => {
  it("takes #rrggbb with or without the hash and in any case", () => {
    expect(normalizeHex("#FF7A1A")).toBe("#ff7a1a");
    expect(normalizeHex("  4da3ff ")).toBe("#4da3ff");
  });

  it("refuses everything else", () => {
    for (const bad of ["", "#", "#fff", "#12345", "#1234567", "#gg0000", "red", "rgb(1,2,3)", "##ff7a1a"]) {
      expect(normalizeHex(bad), bad).toBeNull();
    }
  });
});

describe("colors", () => {
  it("round-trips a color through its channels", () => {
    expect(parseHex("#ff7a1a")).toEqual([255, 122, 26]);
    expect(toHex([255, 122, 26])).toBe("#ff7a1a");
    expect(toHex([300, -5, 0.4])).toBe("#ff0000");
  });

  it("falls back to the default color for something that is not one", () => {
    expect(parseHex("nonsense")).toEqual(parseHex(DEFAULT_ACCENT));
  });

  it("measures contrast like WCAG", () => {
    expect(luminance("#000000")).toBe(0);
    expect(luminance("#ffffff")).toBeCloseTo(1, 5);
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 3);
    expect(contrast("#777777", "#777777")).toBeCloseTo(1, 5);
    expect(contrast("#ffffff", "#000000")).toBeCloseTo(contrast("#000000", "#ffffff"), 8);
  });

  it("mixes towards a target", () => {
    expect(mix("#000000", "#ffffff", 0)).toBe("#000000");
    expect(mix("#000000", "#ffffff", 1)).toBe("#ffffff");
    expect(mix("#000000", "#ffffff", 0.5)).toBe("#808080");
  });
});

describe("the accent as text", () => {
  it("keeps a color that already reads well", () => {
    expect(readableAccent("#ff7a1a", "dark")).toBe("#ff7a1a");
    expect(readableAccent("#a70f1f", "light")).toBe("#a70f1f");
  });

  it("darkens a pale color for the light scheme until it can be read on white", () => {
    const yellow = readableAccent("#ffd166", "light");
    expect(yellow).not.toBe("#ffd166");
    expect(contrast(yellow, "#ffffff")).toBeGreaterThanOrEqual(4.5);
  });

  it("lightens a dark color for the dark scheme until it can be read on the card", () => {
    const navy = readableAccent("#1b2a80", "dark");
    expect(navy).not.toBe("#1b2a80");
    expect(contrast(navy, "#14161d")).toBeGreaterThanOrEqual(4.5);
  });

  it("every preset can be read in both schemes", () => {
    for (const preset of ACCENT_PRESETS) {
      expect(contrast(readableAccent(preset.hex, "dark"), "#14161d"), preset.id).toBeGreaterThanOrEqual(4.5);
      expect(contrast(readableAccent(preset.hex, "light"), "#ffffff"), preset.id).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("goes all the way to black or white when the minimum can't be met", () => {
    expect(readableAccent("#ffffff", "light")).toBe("#737373"); // grey is enough on white
    expect(readableAccent("#808080", "light", 30)).toBe("#000000");
    expect(readableAccent("#808080", "dark", 30)).toBe("#ffffff");
  });
});

describe("accentVars", () => {
  it("hands the stylesheet the accent, a lighter one for hover and a readable one for text", () => {
    const vars = accentVars("#4DA3FF", "dark");
    expect(vars["--accent"]).toBe("#4da3ff");
    expect(luminance(vars["--accent-strong"])).toBeGreaterThan(luminance("#4da3ff"));
    expect(contrast(vars["--accent-text"], "#14161d")).toBeGreaterThanOrEqual(4.5);
  });

  it("uses the default color when given something that is not one", () => {
    expect(accentVars("oops", "light")["--accent"]).toBe(DEFAULT_ACCENT);
  });

  it("changes the text color with the scheme, not the accent itself", () => {
    const dark = accentVars("#ffd166", "dark");
    const light = accentVars("#ffd166", "light");
    expect(dark["--accent"]).toBe(light["--accent"]);
    expect(dark["--accent-text"]).not.toBe(light["--accent-text"]);
  });
});

describe("resolveScheme", () => {
  it("light and dark are what they say, whatever the system prefers", () => {
    expect(resolveScheme("light", false)).toBe("light");
    expect(resolveScheme("dark", true)).toBe("dark");
  });

  it("system follows the operating system", () => {
    expect(resolveScheme("system", true)).toBe("light");
    expect(resolveScheme("system", false)).toBe("dark");
  });
});

describe("the defaults", () => {
  it("are the look the app always had", () => {
    expect(defaultSettings().appearance).toEqual({ theme: "dark", accent: DEFAULT_ACCENT });
  });

  it("start with a preset the picker can show as chosen", () => {
    expect(ACCENT_PRESETS.map((p) => p.hex)).toContain(DEFAULT_ACCENT);
  });
});
