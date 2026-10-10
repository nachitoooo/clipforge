import { describe, expect, it } from "vitest";
import {
  captureAccelerator,
  displayAccelerator,
  normalizeAccelerator,
  ownerOf,
  sameAccelerator,
  worksAlone,
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

  it("refuses keys it can't express", () => {
    for (const code of ["Quote", "ContextMenu", "CapsLock", "IntlBackslash", "AudioVolumeUp", "Unidentified"]) {
      const result = captureAccelerator(press(code, { altKey: true }), false);
      expect(result.kind, code).toBe("invalid");
    }
  });

  it("accepts punctuation, editing keys and the numpad with a modifier", () => {
    for (const code of [
      "Enter",
      "Tab",
      "Backspace",
      "Delete",
      "Backquote",
      "Minus",
      "Equal",
      "BracketLeft",
      "BracketRight",
      "Backslash",
      "Semicolon",
      "Comma",
      "Period",
      "Slash",
      "Numpad0",
      "Numpad9",
      "NumpadAdd",
      "NumpadSubtract",
      "NumpadMultiply",
      "NumpadDivide",
      "NumpadDecimal",
      "NumpadEnter",
      "NumpadEqual",
    ]) {
      expect(captureAccelerator(press(code, { ctrlKey: true }), false), code).toEqual({
        kind: "accelerator",
        accelerator: `CommandOrControl+${code}`,
      });
    }
  });
});

describe("a single key as a shortcut", () => {
  const alone = [
    ...Array.from({ length: 24 }, (_, i) => `F${i + 1}`),
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Pause",
    "ScrollLock",
    "PrintScreen",
  ];

  it("works for function keys and keys that type nothing", () => {
    for (const code of alone) {
      expect(captureAccelerator(press(code), true), code).toEqual({
        kind: "accelerator",
        accelerator: code,
      });
      expect(captureAccelerator(press(code), false), code).toEqual({
        kind: "accelerator",
        accelerator: code,
      });
      expect(worksAlone(code), code).toBe(true);
    }
  });

  it("can still carry modifiers", () => {
    expect(captureAccelerator(press("F9", { shiftKey: true }), true)).toEqual({
      kind: "accelerator",
      accelerator: "Shift+F9",
    });
    expect(captureAccelerator(press("F9", { metaKey: true }), true)).toEqual({
      kind: "accelerator",
      accelerator: "CommandOrControl+F9",
    });
    expect(captureAccelerator(press("Home", { ctrlKey: true, altKey: true }), false)).toEqual({
      kind: "accelerator",
      accelerator: "CommandOrControl+Alt+Home",
    });
  });

  it("is refused for keys used to write or move around, with the reason", () => {
    for (const code of [
      "KeyM",
      "Digit5",
      "Space",
      "Enter",
      "Tab",
      "Backspace",
      "Delete",
      "ArrowUp",
      "ArrowLeft",
      "Numpad0",
      "NumpadAdd",
      "Comma",
      "Slash",
    ]) {
      const result = captureAccelerator(press(code), true);
      expect(result.kind, code).toBe("invalid");
      if (result.kind === "invalid") {
        expect(result.reason).toContain("Sola");
        expect(result.reason).toContain("Combiná");
        expect(result.reason).toContain("F1–F24");
      }
      expect(worksAlone(code), code).toBe(false);
    }
  });

  it("Shift alone does not make a typing key safe", () => {
    expect(captureAccelerator(press("KeyM", { shiftKey: true }), false).kind).toBe("invalid");
    expect(captureAccelerator(press("Space", { shiftKey: true }), false).kind).toBe("invalid");
  });

  it("the reason names the platform's modifiers", () => {
    const mac = captureAccelerator(press("KeyM"), true);
    const pc = captureAccelerator(press("KeyM"), false);
    expect(mac.kind === "invalid" && mac.reason).toContain("⌘, ⌃ o ⌥");
    expect(pc.kind === "invalid" && pc.reason).toContain("Ctrl, Alt o Win");
  });

  it("Escape is never a shortcut: it cancels the recording", () => {
    expect(captureAccelerator(press("Escape"), true)).toEqual({ kind: "cancel" });
    expect(worksAlone("Escape")).toBe(false);
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

  it("prints single keys as they are", () => {
    expect(displayAccelerator("F9", true)).toBe("F9");
    expect(displayAccelerator("F9", false)).toBe("F9");
    expect(displayAccelerator("Shift+F9", true)).toBe("⇧F9");
    expect(displayAccelerator("Shift+F9", false)).toBe("Shift+F9");
  });

  it("names the other keys in Spanish", () => {
    expect(displayAccelerator("Home", false)).toBe("Inicio");
    expect(displayAccelerator("End", false)).toBe("Fin");
    expect(displayAccelerator("PageUp", false)).toBe("RePág");
    expect(displayAccelerator("PageDown", false)).toBe("AvPág");
    expect(displayAccelerator("Pause", false)).toBe("Pausa");
    expect(displayAccelerator("PrintScreen", false)).toBe("ImprPant");
    expect(displayAccelerator("Insert", false)).toBe("Insert");
    expect(displayAccelerator("Control+Numpad0", true)).toBe("⌃Núm 0");
    expect(displayAccelerator("CommandOrControl+NumpadAdd", false)).toBe("Ctrl+Núm +");
    expect(displayAccelerator("Alt+Backspace", false)).toBe("Alt+Retroceso");
    expect(displayAccelerator("Control+Slash", false)).toBe("Ctrl+/");
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
