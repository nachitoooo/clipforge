// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClipInfo } from "../lib/clips";
import { NONE, type ClipFilter } from "../lib/filter";
import type { FolderInfo } from "../lib/folders";
import { createFilterBar, type FilterBar } from "./filterbar";

const clip = (name: string, game: string | null, folders: string[] = []): ClipInfo => ({
  name: `${name}.mp4`,
  kind: "clip",
  path: `/clips/${name}.mp4`,
  sizeBytes: 1,
  modifiedMs: 0,
  folders,
  game,
});

const folder = (id: string, name: string, clipCount = 0): FolderInfo => ({
  id,
  name,
  color: "blue",
  icon: "star",
  clipCount,
  sizeBytes: 0,
  createdMs: 0,
});

const clips = [
  clip("a", "Valorant", ["f1"]),
  clip("b", "Valorant"),
  clip("c", "Fortnite"),
  clip("d", null),
];
const folders = [folder("f1", "Mejores", 1)];

let changes: ClipFilter[];
let bar: FilterBar;

function make(folderSelector = true) {
  document.body.innerHTML = '<div id="host"></div>';
  changes = [];
  bar = createFilterBar({ onChange: (f) => changes.push(f), folderSelector });
  document.getElementById("host")!.append(bar.el);
  bar.setData({ clips, folders });
}

beforeEach(() => make());

const q = <T extends Element>(selector: string) => bar.el.querySelector(selector) as T;
const select = (label: string) => q<HTMLSelectElement>(`select[aria-label="${label}"]`);
const search = () => q<HTMLInputElement>("input");
const clearButton = () => q<HTMLButtonElement>("button");
const last = () => changes[changes.length - 1];
const labels = (el: HTMLSelectElement) => [...el.options].map((o) => o.textContent);

function choose(el: HTMLSelectElement, value: string) {
  el.value = value;
  el.dispatchEvent(new Event("change", { bubbles: true }));
}

function type(text: string) {
  search().value = text;
  search().dispatchEvent(new Event("input", { bubbles: true }));
}

describe("the options it offers", () => {
  it("lists the games with their counts, most used first, and the clips without one", () => {
    expect(labels(select("Juego"))).toEqual([
      "Todos los juegos",
      "Valorant (2)",
      "Fortnite (1)",
      "Sin juego (1)",
    ]);
  });

  it("lists the folders with their counts and the clips in none", () => {
    expect(labels(select("Carpeta"))).toEqual(["Todas las carpetas", "Mejores (1)", "Sin carpeta (3)"]);
  });

  it("offers the dates and the orders", () => {
    expect(labels(select("Fecha"))).toEqual([
      "Todo el tiempo",
      "Hoy",
      "Últimos 7 días",
      "Últimos 30 días",
      "Este año",
    ]);
    expect(labels(select("Ordenar por"))).toEqual(["Más nuevos", "Más viejos", "Nombre (A–Z)", "Más pesados"]);
  });

  it("hides the game selector when no clip has a game, and the folder one when there are none", () => {
    bar.setData({ clips: [clip("a", null)], folders: [] });
    expect(select("Juego").hidden).toBe(true);
    expect(select("Carpeta").hidden).toBe(true);
    bar.setData({ clips, folders });
    expect(select("Juego").hidden).toBe(false);
    expect(select("Carpeta").hidden).toBe(false);
  });

  it("leaves the folder selector out when asked (inside a folder)", () => {
    make(false);
    expect(bar.el.querySelector('select[aria-label="Carpeta"]')).toBeNull();
  });
});

describe("changing the filter", () => {
  it("starts with nothing hidden and no clear button", () => {
    expect(bar.get()).toEqual({ query: "", game: null, folder: null, range: "all", sort: "newest", kind: "all" });
    expect(clearButton().hidden).toBe(true);
  });

  it("typing searches at once", () => {
    type("ace");
    expect(last()?.query).toBe("ace");
    expect(clearButton().hidden).toBe(false);
  });

  it("choosing a game, a folder, a date and an order each tells the view", () => {
    choose(select("Juego"), "valorant");
    expect(last()?.game).toBe("valorant");
    choose(select("Carpeta"), "f1");
    expect(last()?.folder).toBe("f1");
    choose(select("Fecha"), "week");
    expect(last()?.range).toBe("week");
    choose(select("Ordenar por"), "size");
    expect(last()).toEqual({ query: "", game: "valorant", folder: "f1", range: "week", sort: "size", kind: "all" });
  });

  it("can pick the clips with no game or no folder", () => {
    choose(select("Juego"), NONE);
    choose(select("Carpeta"), NONE);
    expect(last()).toMatchObject({ game: NONE, folder: NONE });
  });

  it("choosing 'all' goes back to no filter", () => {
    choose(select("Juego"), "valorant");
    choose(select("Juego"), "");
    expect(last()?.game).toBeNull();
    expect(clearButton().hidden).toBe(true);
  });

  it("changing only the order is not filtering", () => {
    choose(select("Ordenar por"), "name");
    expect(clearButton().hidden).toBe(true);
  });

  it("Escape in the search box clears the text", () => {
    type("ace");
    search().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(search().value).toBe("");
    expect(last()?.query).toBe("");
  });

  it("the filter handed out is a copy", () => {
    const given = bar.get();
    given.query = "tampered";
    expect(bar.get().query).toBe("");
  });
});

describe("clearing", () => {
  it("the button resets every filter, keeps the order and tells the view", () => {
    type("ace");
    choose(select("Juego"), "valorant");
    choose(select("Fecha"), "today");
    choose(select("Ordenar por"), "name");
    clearButton().click();
    expect(last()).toEqual({ query: "", game: null, folder: null, range: "all", sort: "name", kind: "all" });
    expect(search().value).toBe("");
    expect(select("Juego").value).toBe("");
    expect(select("Fecha").value).toBe("all");
    expect(select("Ordenar por").value).toBe("name");
    expect(clearButton().hidden).toBe(true);
  });

  it("reset() clears without telling the view", () => {
    type("ace");
    const before = changes.length;
    bar.reset();
    expect(changes).toHaveLength(before);
    expect(bar.get().query).toBe("");
    expect(search().value).toBe("");
  });
});

describe("new data", () => {
  it("keeps a selection that still exists", () => {
    choose(select("Juego"), "fortnite");
    bar.setData({ clips: [...clips, clip("e", "Fortnite")], folders });
    expect(bar.get().game).toBe("fortnite");
    expect(select("Juego").value).toBe("fortnite");
    expect(labels(select("Juego"))).toContain("Fortnite (2)");
  });

  it("drops a game that has no clips left", () => {
    choose(select("Juego"), "fortnite");
    bar.setData({ clips: clips.filter((c) => c.game !== "Fortnite"), folders });
    expect(bar.get().game).toBeNull();
    expect(select("Juego").value).toBe("");
  });

  it("drops a folder that was deleted but keeps 'no folder'", () => {
    choose(select("Carpeta"), "f1");
    bar.setData({ clips, folders: [folder("f2", "Otra")] });
    expect(bar.get().folder).toBeNull();

    choose(select("Carpeta"), NONE);
    bar.setData({ clips, folders: [folder("f2", "Otra")] });
    expect(bar.get().folder).toBe(NONE);
  });

  it("does not wipe what was typed", () => {
    type("ace");
    bar.setData({ clips, folders });
    expect(search().value).toBe("ace");
  });
});

describe("clips and screenshots", () => {
  const shots = [
    { ...clip("foto-1", null), name: "foto-1.png", kind: "screenshot" as const },
    { ...clip("foto-2", "Valorant"), name: "foto-2.png", kind: "screenshot" as const },
  ];
  const kindSelect = () => select("Tipo");

  it("offers no type choice while there are only clips", () => {
    expect(kindSelect().hidden).toBe(true);
  });

  it("offers it as soon as there is a screenshot", () => {
    bar.setData({ clips: [...clips, ...shots], folders });
    expect(kindSelect().hidden).toBe(false);
    expect(labels(kindSelect())).toEqual(["Clips y capturas", "Solo clips", "Solo capturas"]);
  });

  it("filters by type and counts it as a filter", () => {
    bar.setData({ clips: [...clips, ...shots], folders });
    choose(kindSelect(), "screenshot");
    expect(last().kind).toBe("screenshot");
    expect(clearButton().hidden).toBe(false);
    choose(kindSelect(), "all");
    expect(last().kind).toBe("all");
    expect(clearButton().hidden).toBe(true);
  });

  it("drops the type filter when the last screenshot is gone", () => {
    bar.setData({ clips: [...clips, ...shots], folders });
    choose(kindSelect(), "screenshot");
    bar.setData({ clips, folders });
    expect(bar.get().kind).toBe("all");
    expect(kindSelect().hidden).toBe(true);
  });

  it("clearing the filters brings the screenshots back", () => {
    bar.setData({ clips: [...clips, ...shots], folders });
    choose(kindSelect(), "clip");
    clearButton().click();
    expect(last().kind).toBe("all");
    expect(kindSelect().value).toBe("all");
  });

  it("counts 'elementos' when the list mixes both", () => {
    bar.setData({ clips: [...clips, ...shots], folders });
    choose(kindSelect(), "screenshot");
    bar.setCount(2, 6);
    expect(q(".filter-status").textContent).toBe("2 de 6 elementos");
  });
});

describe("the count", () => {
  it("shows 'N de M clips' only while something is filtered", () => {
    bar.setCount(4, 4);
    expect(q(".filter-status").textContent).toBe("");
    type("ace");
    bar.setCount(1, 4);
    expect(q(".filter-status").textContent).toBe("1 de 4 clips");
    bar.reset();
    bar.setCount(4, 4);
    expect(q(".filter-status").textContent).toBe("");
  });
});

describe("keepFocus", () => {
  it("gives the focus back to the search box after the bar is moved", () => {
    const host = document.getElementById("host")!;
    search().focus();
    expect(document.activeElement).toBe(search());
    bar.keepFocus(() => host.replaceChildren(bar.el));
    expect(document.activeElement).toBe(search());
  });

  it("leaves the focus alone when it was elsewhere", () => {
    const other = document.createElement("button");
    document.body.append(other);
    other.focus();
    bar.keepFocus(() => document.getElementById("host")!.replaceChildren(bar.el));
    expect(document.activeElement).toBe(other);
  });

  it("just runs the change when nothing had the focus", () => {
    const change = vi.fn();
    bar.keepFocus(change);
    expect(change).toHaveBeenCalledTimes(1);
  });
});
