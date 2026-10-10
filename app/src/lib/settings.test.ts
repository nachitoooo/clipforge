import { describe, expect, it } from "vitest";
import {
  clampGain,
  defaultSettings,
  formatGain,
  gainToDb,
  micStateSummary,
  withMicrophone,
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
