import { describe, expect, it } from "vitest";
import { bufferLabel } from "./buffer";

describe("bufferLabel", () => {
  it("shows a neutral label until the status loads", () => {
    expect(bufferLabel(null)).toBe("Buffer…");
  });

  it("distinguishes running, stopped on purpose, and failed to start", () => {
    expect(bufferLabel({ running: true, error: null })).toBe("Grabando buffer");
    expect(bufferLabel({ running: false, error: null })).toBe("Buffer detenido");
    expect(bufferLabel({ running: false, error: "permiso denegado" })).toBe("Sin captura");
  });

  it("running wins over a stale error", () => {
    expect(bufferLabel({ running: true, error: "viejo" })).toBe("Grabando buffer");
  });
});
