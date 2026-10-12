// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createAppearance } from "./appearance";

type Listener = () => void;

/** A stand-in for `matchMedia("(prefers-color-scheme: light)")` the test can flip. */
function fakeSystem(startsLight: boolean) {
  const listeners = new Set<Listener>();
  const query = {
    matches: startsLight,
    addEventListener: (_: string, l: Listener) => listeners.add(l),
    removeEventListener: (_: string, l: Listener) => listeners.delete(l),
  };
  vi.stubGlobal("matchMedia", () => query);
  return {
    set(light: boolean) {
      query.matches = light;
      for (const l of listeners) l();
    },
  };
}

let root: HTMLElement;

beforeEach(() => {
  root = document.createElement("div");
});

afterEach(() => vi.unstubAllGlobals());

describe("applying a look", () => {
  it("sets the theme and the accent colors on the root", () => {
    fakeSystem(false);
    const appearance = createAppearance(root);
    appearance.apply({ theme: "light", accent: "#4da3ff" });
    expect(root.dataset.theme).toBe("light");
    expect(root.style.getPropertyValue("--accent")).toBe("#4da3ff");
    expect(root.style.getPropertyValue("--accent-strong")).toMatch(/^#[0-9a-f]{6}$/);
    expect(root.style.getPropertyValue("--accent-text")).toMatch(/^#[0-9a-f]{6}$/);
    expect(appearance.scheme()).toBe("light");
  });

  it("changing the color changes the properties, changing the theme keeps the color", () => {
    fakeSystem(false);
    const appearance = createAppearance(root);
    appearance.apply({ theme: "dark", accent: "#ff7a1a" });
    appearance.apply({ theme: "dark", accent: "#a77bff" });
    expect(root.style.getPropertyValue("--accent")).toBe("#a77bff");
    appearance.apply({ theme: "light", accent: "#a77bff" });
    expect(root.style.getPropertyValue("--accent")).toBe("#a77bff");
    expect(root.dataset.theme).toBe("light");
  });

  it("the readable accent follows the theme", () => {
    fakeSystem(false);
    const appearance = createAppearance(root);
    appearance.apply({ theme: "dark", accent: "#ffd166" });
    const onDark = root.style.getPropertyValue("--accent-text");
    appearance.apply({ theme: "light", accent: "#ffd166" });
    expect(root.style.getPropertyValue("--accent-text")).not.toBe(onDark);
  });

  it("does not hold on to the object it was given", () => {
    fakeSystem(false);
    const appearance = createAppearance(root);
    const given = { theme: "dark" as const, accent: "#ff7a1a" };
    appearance.apply(given);
    given.theme = "light" as never;
    expect(appearance.scheme()).toBe("dark");
  });
});

describe("the system theme", () => {
  it("is dark or light as the operating system says", () => {
    fakeSystem(true);
    const appearance = createAppearance(root);
    appearance.apply({ theme: "system", accent: "#ff7a1a" });
    expect(root.dataset.theme).toBe("light");
  });

  it("follows the operating system while the app is open", () => {
    const system = fakeSystem(false);
    const appearance = createAppearance(root);
    appearance.apply({ theme: "system", accent: "#ff7a1a" });
    expect(root.dataset.theme).toBe("dark");
    system.set(true);
    expect(root.dataset.theme).toBe("light");
    system.set(false);
    expect(root.dataset.theme).toBe("dark");
  });

  it("is ignored when the theme was chosen by hand", () => {
    const system = fakeSystem(false);
    const appearance = createAppearance(root);
    appearance.apply({ theme: "dark", accent: "#ff7a1a" });
    system.set(true);
    expect(root.dataset.theme).toBe("dark");
  });

  it("still works where the browser can't tell (no matchMedia)", () => {
    vi.stubGlobal("matchMedia", undefined);
    const appearance = createAppearance(root);
    appearance.apply({ theme: "system", accent: "#ff7a1a" });
    expect(root.dataset.theme).toBe("dark");
  });
});

describe("before any look is applied", () => {
  it("draws dark, like the stylesheet's own default", () => {
    fakeSystem(true);
    expect(createAppearance(root).scheme()).toBe("dark");
    expect(root.dataset.theme).toBeUndefined();
  });
});
