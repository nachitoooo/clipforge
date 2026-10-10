import { describe, expect, it } from "vitest";
import { MAX_CLIP_NAME_LENGTH, validateClipName } from "./clipname";

describe("validateClipName", () => {
  it("accepts normal names and returns the bare stem", () => {
    expect(validateClipName("  Ace en Mirage ")).toEqual({
      ok: true,
      stem: "Ace en Mirage",
    });
    expect(validateClipName("clutch.mp4")).toEqual({ ok: true, stem: "clutch" });
    expect(validateClipName("clutch.MP4")).toEqual({ ok: true, stem: "clutch" });
    expect(validateClipName("año nuevo ñandú")).toEqual({
      ok: true,
      stem: "año nuevo ñandú",
    });
  });

  it("accepts the maximum length and rejects one more", () => {
    expect(validateClipName("a".repeat(MAX_CLIP_NAME_LENGTH)).ok).toBe(true);
    expect(validateClipName("a".repeat(MAX_CLIP_NAME_LENGTH + 1)).ok).toBe(false);
  });

  it("rejects empty names", () => {
    for (const input of ["", "   ", ".mp4", " .MP4 "]) {
      expect(validateClipName(input).ok, JSON.stringify(input)).toBe(false);
    }
  });

  it("rejects hidden-file names", () => {
    expect(validateClipName(".hidden").ok).toBe(false);
  });

  it("rejects path separators and reserved characters", () => {
    for (const input of ["a/b", "a\\b", "a:b", "a*b", "a?b", 'a"b', "a<b", "a>b", "a|b", "../x"]) {
      expect(validateClipName(input).ok, input).toBe(false);
    }
  });

  it("rejects control characters", () => {
    expect(validateClipName("tab\there").ok).toBe(false);
    expect(validateClipName("nul\0x").ok).toBe(false);
  });

  it("reports a human-readable error", () => {
    const result = validateClipName("a/b");
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toMatch(/no permitidos/);
  });
});
