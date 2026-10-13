import { describe, expect, it } from "vitest";
import {
  CLIP_LENGTH_CHOICES,
  DEFAULT_MUTE_SHORTCUT,
  DEFAULT_SAVE_SHORTCUT,
  clampGain,
  clipLengthOptions,
  defaultSettings,
  formatClipLength,
  formatBitrate,
  formatGain,
  formatMemory,
  gainToDb,
  memoryModeText,
  BUFFER_FOLLOWS_CLIPS,
  micStateSummary,
  shortcutOwners,
  withAddedShortcut,
  withExtraShortcut,
  withLibrary,
  withMicrophone,
  withShortcuts,
  withPerformance,
  withoutExtraShortcut,
  type MemoryPlans,
  type MicStatus,
} from "./settings";

describe("clampGain", () => {
  it("keeps whole percents inside 0–200", () => {
    expect(clampGain(100)).toBe(100);
    expect(clampGain(-5)).toBe(0);
    expect(clampGain(999)).toBe(200);
    expect(clampGain(49.6)).toBe(50);
  });

  it("falls back to 100 for non-numbers", () => {
    expect(clampGain(NaN)).toBe(100);
    expect(clampGain(Infinity)).toBe(100);
  });
});

describe("gain in decibels", () => {
  it("is 0 dB at 100 %, +6 dB at 200 %, -6 dB at 50 %, silence at 0", () => {
    expect(gainToDb(100)).toBeCloseTo(0, 6);
    expect(gainToDb(200)).toBeCloseTo(6.02, 2);
    expect(gainToDb(50)).toBeCloseTo(-6.02, 2);
    expect(gainToDb(0)).toBe(-Infinity);
  });

  it("formats with a decimal comma and a real minus sign", () => {
    expect(formatGain(100)).toBe("100 % · 0,0 dB");
    expect(formatGain(150)).toBe("150 % · +3,5 dB");
    expect(formatGain(50)).toBe("50 % · −6,0 dB");
    expect(formatGain(0)).toBe("0 % · silencio");
    expect(formatGain(500)).toBe("200 % · +6,0 dB"); // clamped
  });
});

describe("settings helpers", () => {
  it("defaults to a private, mixed, unmuted microphone at unity gain", () => {
    const s = defaultSettings();
    expect(s.microphone.enabled).toBe(false);
    expect(s.microphone.mode).toBe("mixed");
    expect(s.microphone.gainPercent).toBe(100);
    expect(s.microphone.muteShortcut).toBe("CommandOrControl+Shift+M");
  });

  it("defaults the save shortcut to the one ClipForge always had", () => {
    expect(defaultSettings().shortcuts.saveClip).toBe("CommandOrControl+Shift+S");
    expect(DEFAULT_SAVE_SHORTCUT).toBe("CommandOrControl+Shift+S");
    expect(DEFAULT_MUTE_SHORTCUT).toBe("CommandOrControl+Shift+M");
  });

  it("patches the shortcuts without touching the rest or the original", () => {
    const original = defaultSettings();
    const next = withShortcuts(original, { saveClip: "Alt+F9" });
    expect(next.shortcuts.saveClip).toBe("Alt+F9");
    expect(next.microphone).toBe(original.microphone);
    expect(original.shortcuts.saveClip).toBe("CommandOrControl+Shift+S");
  });

  it("game detection is on by default, and automatic folders and copying are off", () => {
    expect(defaultSettings().library.detectGame).toBe(true);
    expect(defaultSettings().library.autoFolders).toBe(false);
    expect(defaultSettings().library.copyOnSave).toBe(false);
  });

  it("patches the library section without touching the rest or the original", () => {
    const original = defaultSettings();
    const next = withLibrary(original, { detectGame: false });
    expect(next.library.detectGame).toBe(false);
    expect(next.microphone).toBe(original.microphone);
    expect(next.shortcuts).toBe(original.shortcuts);
    expect(original.library.detectGame).toBe(true);
  });

  it("patches the microphone without mutating the original", () => {
    const original = defaultSettings();
    const next = withMicrophone(original, { enabled: true, gainPercent: 120 });
    expect(next.microphone.enabled).toBe(true);
    expect(next.microphone.gainPercent).toBe(120);
    expect(next.microphone.mode).toBe("mixed");
    expect(original.microphone.enabled).toBe(false);
  });
});

describe("micStateSummary", () => {
  const base: MicStatus = {
    enabled: true,
    active: true,
    muted: false,
    error: null,
    noSignal: false,
    supported: true,
  };

  it("covers every state with the right tone", () => {
    expect(micStateSummary(null).tone).toBe("off");
    expect(micStateSummary({ ...base, supported: false }).tone).toBe("off");
    expect(micStateSummary({ ...base, enabled: false, active: false }).text).toBe("Desactivado");
    expect(micStateSummary({ ...base, active: false, error: "boom" }).tone).toBe("error");
    expect(micStateSummary({ ...base, active: false }).tone).toBe("warn");
    expect(micStateSummary({ ...base, muted: true }).text).toBe("Silenciado");
    expect(micStateSummary({ ...base, noSignal: true }).tone).toBe("warn");
    expect(micStateSummary(base)).toEqual({ tone: "ok", text: "Grabando tu voz" });
  });

  it("an error wins over everything else", () => {
    const s = micStateSummary({ ...base, active: false, error: "x", muted: true, noSignal: true });
    expect(s.tone).toBe("error");
  });
});

describe("clip lengths", () => {
  it("reads them the way people say them", () => {
    expect(
      [5, 30, 59, 60, 90, 120].map(formatClipLength),
    ).toEqual(["5 s", "30 s", "59 s", "1 min", "1 min 30 s", "2 min"]);
  });

  it("offers the usual lengths, plus a hand-set one in its place", () => {
    expect(clipLengthOptions(30)).toBe(CLIP_LENGTH_CHOICES);
    expect(clipLengthOptions(45)).toEqual([15, 30, 45, 60, 120]);
    expect(clipLengthOptions(5)).toEqual([5, 15, 30, 60, 120]);
  });

  it("defaults to 30 seconds and no extra shortcuts", () => {
    const { shortcuts } = defaultSettings();
    expect(shortcuts.clipSeconds).toBe(30);
    expect(shortcuts.extraSaveClip).toEqual([]);
  });
});

describe("extra save shortcuts", () => {
  const two = withAddedShortcut(
    withAddedShortcut(defaultSettings(), { accelerator: "F9", seconds: 15 }),
    { accelerator: "F10", seconds: 60 },
  );

  it("adds at the end without touching the original", () => {
    const original = defaultSettings();
    const next = withAddedShortcut(original, { accelerator: "F9", seconds: 15 });
    expect(next.shortcuts.extraSaveClip).toEqual([{ accelerator: "F9", seconds: 15 }]);
    expect(original.shortcuts.extraSaveClip).toEqual([]);
    expect(next.microphone).toBe(original.microphone);
    expect(two.shortcuts.extraSaveClip.map((e) => e.accelerator)).toEqual(["F9", "F10"]);
  });

  it("patches only the one asked for", () => {
    const next = withExtraShortcut(two, 1, { seconds: 120 });
    expect(next.shortcuts.extraSaveClip).toEqual([
      { accelerator: "F9", seconds: 15 },
      { accelerator: "F10", seconds: 120 },
    ]);
    expect(two.shortcuts.extraSaveClip[1].seconds).toBe(60);
    expect(withExtraShortcut(two, 5, { seconds: 1 }).shortcuts.extraSaveClip).toEqual(
      two.shortcuts.extraSaveClip,
    );
  });

  it("removes one and keeps the rest in order", () => {
    expect(withoutExtraShortcut(two, 0).shortcuts.extraSaveClip).toEqual([
      { accelerator: "F10", seconds: 60 },
    ]);
    expect(withoutExtraShortcut(two, 1).shortcuts.extraSaveClip).toHaveLength(1);
  });

  it("lists every shortcut in use with what it does", () => {
    const owners = shortcutOwners(two);
    expect(owners).toEqual([
      { accelerator: "CommandOrControl+Shift+S", owner: "Guardar clip (30 s)" },
      { accelerator: "F9", owner: "Guardar clip (15 s)" },
      { accelerator: "F10", owner: "Guardar clip (1 min)" },
      { accelerator: "CommandOrControl+Shift+M", owner: "Silenciar el micrófono" },
    ]);
  });

  it("leaves out the shortcut being recorded, so it can keep its own keys", () => {
    const accelerators = (except: Parameters<typeof shortcutOwners>[1]) =>
      shortcutOwners(two, except).map((o) => o.accelerator);
    expect(accelerators("save")).not.toContain("CommandOrControl+Shift+S");
    expect(accelerators("mute")).not.toContain("CommandOrControl+Shift+M");
    expect(accelerators({ extra: 0 })).toEqual([
      "CommandOrControl+Shift+S",
      "F10",
      "CommandOrControl+Shift+M",
    ]);
    expect(accelerators(null)).toHaveLength(4);
  });
});

describe("the screenshot shortcut", () => {
  it("is empty until one is chosen", () => {
    expect(defaultSettings().shortcuts.screenshot).toBe("");
  });

  it("is one of the shortcuts in use once chosen, and not before", () => {
    const none = shortcutOwners(defaultSettings());
    expect(none.some((o) => o.owner === "Captura de pantalla")).toBe(false);

    const chosen = withShortcuts(defaultSettings(), { screenshot: "F8" });
    expect(shortcutOwners(chosen)).toContainEqual({
      accelerator: "F8",
      owner: "Captura de pantalla",
    });
    // The one being recorded may keep its own keys.
    expect(shortcutOwners(chosen, "screenshot").map((o) => o.accelerator)).not.toContain("F8");
  });
});

describe("low RAM mode", () => {
  // With the usual 30 s clip: the buffer holds the clip plus a margin.
  const plans: MemoryPlans = {
    normal: { bufferSeconds: 34, maxBufferSeconds: 120, bitrate: 12_000_000, estimatedBytes: 51_000_000 },
    lowRam: { bufferSeconds: 34, maxBufferSeconds: 60, bitrate: 6_000_000, estimatedBytes: 25_500_000 },
  };
  // With a 2 min clip: both modes keep all they can.
  const longClips: MemoryPlans = {
    normal: { bufferSeconds: 120, maxBufferSeconds: 120, bitrate: 12_000_000, estimatedBytes: 180_000_000 },
    lowRam: { bufferSeconds: 60, maxBufferSeconds: 60, bitrate: 6_000_000, estimatedBytes: 45_000_000 },
  };

  it("is off by default", () => {
    expect(defaultSettings().performance.lowRam).toBe(false);
  });

  it("patches only the performance section", () => {
    const original = defaultSettings();
    const next = withPerformance(original, { lowRam: true });
    expect(next.performance.lowRam).toBe(true);
    expect(next.microphone).toBe(original.microphone);
    expect(next.library).toBe(original.library);
    expect(original.performance.lowRam).toBe(false);
  });

  it("writes memory and bitrate the way people read them", () => {
    expect(formatMemory(180_000_000)).toBe("180 MB");
    expect(formatMemory(44_600_000)).toBe("45 MB");
    expect(formatBitrate(12_000_000)).toBe("12 Mb/s");
    expect(formatBitrate(7_500_000)).toBe("7,5 Mb/s");
  });

  it("describes the mode that is on and the other one", () => {
    expect(memoryModeText(plans, false)).toBe(
      "Ahora: 34 s de buffer a 12 Mb/s (unos 51 MB de RAM). Activado: 34 s de buffer a 6 Mb/s (unos 26 MB de RAM). La imagen sería un poco más blanda en escenas rápidas.",
    );
    const on = memoryModeText(plans, true);
    expect(on.startsWith("Activado: 34 s de buffer a 6 Mb/s (unos 26 MB de RAM) en vez de 34 s")).toBe(true);
    expect(on).not.toContain("aunque pidas más");
  });

  it("says that clips are cut short only when the mode cannot keep what they ask for", () => {
    const off = memoryModeText(longClips, false);
    expect(off).toContain("Ahora: 2 min de buffer a 12 Mb/s (unos 180 MB de RAM)");
    expect(off).toContain("Activado: 1 min de buffer a 6 Mb/s (unos 45 MB de RAM)");
    expect(off).toContain("Los clips duran hasta 1 min, aunque pidas más");
    expect(memoryModeText(longClips, true)).toContain("Los clips duran hasta 1 min, aunque pidas más");
  });

  it("explains that the buffer follows the longest clip", () => {
    expect(BUFFER_FOLLOWS_CLIPS).toContain("lo que necesita tu clip más largo");
    expect(BUFFER_FOLLOWS_CLIPS).toContain("no dos minutos fijos");
    expect(BUFFER_FOLLOWS_CLIPS).toContain("necesita ese tiempo para llenarse");
  });

  it("has words even without the numbers", () => {
    expect(memoryModeText(null, false)).toContain("menos RAM");
    expect(memoryModeText(null, true)).toContain("menos RAM");
  });
});
