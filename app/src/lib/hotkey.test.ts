import { describe, expect, it } from "vitest";
import {
  captureAccelerator,
  displayAccelerator,
  normalizeAccelerator,
  ownerOf,
  sameAccelerator,
  type KeyEventLike,
} from "./hotkey";

function press(code: string, mods: Partial<KeyEventLike> = {}): KeyEventLike {
  return {
    key: code,
    code,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...mods,
  };
}

describe("captureAccelerator", () => {
  it("builds the portable accelerator from ⌘/Ctrl + Shift + a letter", () => {
    expect(captureAccelerator(press("KeyM", { metaKey: true, shiftKey: true }), true)).toEqual({
      kind: "accelerator",
      accelerator: "CommandOrControl+Shift+M",
    });
    expect(captureAccelerator(press("KeyM", { ctrlKey: true, shiftKey: true }), false)).toEqual({
      kind: "accelerator",
      accelerator: "CommandOrControl+Shift+M",
    });
  });

  it("keeps the platform's other modifiers distinct", () => {
    // On a Mac, physical Control is not the portable primary modifier.
    expect(captureAccelerator(press("KeyK", { ctrlKey: true }), true)).toEqual({
      kind: "accelerator",
      accelerator: "Control+K",
    });
    // On Windows the Windows key is Super.
    expect(captureAccelerator(press("KeyK", { metaKey: true }), false)).toEqual({
      kind: "accelerator",
      accelerator: "Super+K",
    });
  });

  it("orders modifiers canonically regardless of how they were pressed", () => {
    const r = captureAccelerator(
      press("F5", { shiftKey: true, altKey: true, ctrlKey: true }),
      false,
    );
    expect(r).toEqual({ kind: "accelerator", accelerator: "CommandOrControl+Alt+Shift+F5" });
  });

  it("accepts digits, function keys, space and arrows", () => {
    for (const [code, token] of [
      ["Digit7", "7"],
      ["F12", "F12"],
      ["Space", "Space"],
      ["ArrowUp", "ArrowUp"],
    ] as const) {
      expect(captureAccelerator(press(code, { altKey: true }), false)).toEqual({
        kind: "accelerator",
        accelerator: `Alt+${token}`,
      });
    }
  });

  it("waits while only modifiers are held", () => {
    for (const code of ["ShiftLeft", "ControlRight", "AltLeft", "MetaLeft"]) {
      expect(captureAccelerator(press(code, { shiftKey: true }), true)).toEqual({
        kind: "waiting",
      });
    }
  });

  it("Escape cancels, but Cmd+Escape is just an unusable key", () => {
    expect(captureAccelerator(press("Escape"), true)).toEqual({ kind: "cancel" });
    expect(captureAccelerator(press("Escape", { metaKey: true }), true).kind).toBe("invalid");
  });

  it("refuses a bare key or Shift alone (it would hijack normal typing)", () => {
    expect(captureAccelerator(press("KeyM"), true).kind).toBe("invalid");
    expect(captureAccelerator(press("KeyM", { shiftKey: true }), false).kind).toBe("invalid");
  });

  it("refuses keys it can't express", () => {
    expect(captureAccelerator(press("BracketLeft", { altKey: true }), false).kind).toBe("invalid");
    expect(captureAccelerator(press("Enter", { ctrlKey: true }), false).kind).toBe("invalid");
  });
});

describe("normalizeAccelerator / sameAccelerator", () => {
  it("treats aliases and order as equal", () => {
    expect(sameAccelerator("CmdOrCtrl+Shift+m", "CommandOrControl+Shift+M")).toBe(true);
    expect(sameAccelerator("Shift+CommandOrControl+M", "CommandOrControl+Shift+M")).toBe(true);
    expect(sameAccelerator("Option+F1", "Alt+F1")).toBe(true);
  });

  it("tells genuinely different shortcuts apart", () => {
    expect(sameAccelerator("CommandOrControl+Shift+M", "CommandOrControl+Shift+S")).toBe(false);
    expect(sameAccelerator("Control+K", "CommandOrControl+K")).toBe(false);
  });

  it("normalizes whitespace and duplicate modifiers", () => {
    expect(normalizeAccelerator(" Ctrl + Ctrl + a ")).toBe("control+A");
  });
});

describe("displayAccelerator", () => {
  it("uses glyphs on a Mac, in the conventional order", () => {
    expect(displayAccelerator("CommandOrControl+Shift+M", true)).toBe("⇧⌘M");
    expect(displayAccelerator("Control+Alt+F5", true)).toBe("⌃⌥F5");
  });

  it("uses names elsewhere", () => {
    expect(displayAccelerator("CommandOrControl+Shift+M", false)).toBe("Ctrl+Shift+M");
    expect(displayAccelerator("Super+K", false)).toBe("Win+K");
  });

  it("prints arrows and space readably", () => {
    expect(displayAccelerator("Alt+ArrowUp", false)).toBe("Alt+↑");
    expect(displayAccelerator("CommandOrControl+Space", false)).toBe("Ctrl+Espacio");
  });
});

describe("ownerOf", () => {
  const taken = [
    { accelerator: "CommandOrControl+Shift+S", owner: "Guardar clip" },
    { accelerator: "Alt+F9", owner: "Silenciar el micrófono" },
  ];

  it("names the feature that already owns a shortcut, however it is spelled", () => {
    expect(ownerOf("CommandOrControl+Shift+S", taken)).toBe("Guardar clip");
    expect(ownerOf("Shift+CmdOrCtrl+s", taken)).toBe("Guardar clip");
    expect(ownerOf("Option+F9", taken)).toBe("Silenciar el micrófono");
  });

  it("returns null for free shortcuts", () => {
    expect(ownerOf("CommandOrControl+Shift+M", taken)).toBeNull();
    expect(ownerOf("Control+Shift+S", taken)).toBeNull();
  });

  it("has nothing to conflict with when nothing is taken", () => {
    expect(ownerOf("CommandOrControl+Shift+S", [])).toBeNull();
  });
});
