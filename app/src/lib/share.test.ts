import { describe, expect, it } from "vitest";
import { SHARE_LIMITS, copiedMessage, limitsExceeded } from "./share";

const MB = 1024 * 1024;
const limit = (id: string) => SHARE_LIMITS.find((l) => l.id === id)!.bytes;

describe("limitsExceeded", () => {
  it("lists nothing for a small clip", () => {
    expect(limitsExceeded(5 * MB)).toEqual([]);
    expect(limitsExceeded(0)).toEqual([]);
  });

  it("a clip exactly at a limit still fits there", () => {
    expect(limitsExceeded(limit("discord")).map((l) => l.id)).toEqual([]);
    expect(limitsExceeded(limit("discord") + 1).map((l) => l.id)).toEqual(["discord"]);
  });

  it("lists every place that is too small, smallest first", () => {
    expect(limitsExceeded(30 * MB).map((l) => l.id)).toEqual(["discord", "mail"]);
    expect(limitsExceeded(500 * MB).map((l) => l.id)).toEqual(["discord", "mail", "discord-basic"]);
  });

  it("has a sensible table: positive, unique, named", () => {
    expect(new Set(SHARE_LIMITS.map((l) => l.id)).size).toBe(SHARE_LIMITS.length);
    for (const l of SHARE_LIMITS) {
      expect(l.bytes).toBeGreaterThan(0);
      expect(l.name.length).toBeGreaterThan(0);
    }
  });
});

describe("copiedMessage", () => {
  it("is a plain confirmation when the clip fits everywhere", () => {
    expect(copiedMessage(8 * MB)).toEqual({
      text: "Copiado: pegalo en el chat o en un mail",
      warn: false,
    });
  });

  it("warns, with the size and the limit, when it doesn't fit", () => {
    const { text, warn } = copiedMessage(22 * MB);
    expect(warn).toBe(true);
    expect(text).toBe("Copiado, pero pesa 22,0 MB y no entra en Discord sin Nitro (20 MB)");
  });

  it("names every place it doesn't fit", () => {
    expect(copiedMessage(30 * MB).text).toBe(
      "Copiado, pero pesa 30,0 MB y no entra en Discord sin Nitro (20 MB) ni un correo (Gmail) (25 MB)",
    );
    expect(copiedMessage(120 * MB).text).toContain(
      "Discord sin Nitro (20 MB), un correo (Gmail) (25 MB) ni Discord con Nitro Basic (50 MB)",
    );
  });
});
