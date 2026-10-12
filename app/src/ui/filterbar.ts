import type { ClipInfo } from "../lib/clips";
import {
  KIND_OPTIONS,
  NONE,
  RANGE_OPTIONS,
  SORT_OPTIONS,
  emptyFilter,
  gameOptions,
  isFiltering,
  type ClipFilter,
  type DateRange,
  type KindFilter,
  type SortKey,
} from "../lib/filter";
import { sortFolders, type FolderInfo } from "../lib/folders";
import { h, svg } from "./dom";
import { icons } from "./icons";

export interface FilterBarOptions {
  /** The person changed something. */
  onChange(filter: ClipFilter): void;
  /** Offer the folder selector (leave it out inside a folder). */
  folderSelector?: boolean;
}

export interface FilterBar {
  el: HTMLElement;
  get(): ClipFilter;
  /**
   * The clips and folders to offer. A game or folder that is selected but no
   * longer exists is dropped from the filter (read it again with `get`).
   */
  setData(data: { clips: readonly ClipInfo[]; folders: readonly FolderInfo[] }): void;
  /** Shown in "N de M clips" while something is filtered. */
  setCount(shown: number, total: number): void;
  /** Back to showing everything (the order is kept). */
  reset(): void;
  /** Run a DOM change that moves the bar without losing the focus inside it. */
  keepFocus(change: () => void): void;
}

function option(value: string, label: string): HTMLOptionElement {
  const el = h("option", { value }, label);
  return el;
}

export function createFilterBar(options: FilterBarOptions): FilterBar {
  const state: ClipFilter = emptyFilter();

  const search = h("input", {
    type: "search",
    class: "text-input filter-search",
    placeholder: "Buscar clips…",
    title: "Busca por nombre, juego o carpeta",
    "aria-label": "Buscar clips",
    spellcheck: "false",
  });
  const gameSelect = h("select", { class: "select select-sm", "aria-label": "Juego" });
  const folderSelect = h("select", { class: "select select-sm", "aria-label": "Carpeta" });
  const kindSelect = h("select", { class: "select select-sm", "aria-label": "Tipo" });
  const rangeSelect = h("select", { class: "select select-sm", "aria-label": "Fecha" });
  const sortSelect = h("select", { class: "select select-sm", "aria-label": "Ordenar por" });
  const status = h("span", { class: "filter-status muted" });
  const clear = h(
    "button",
    { class: "btn btn-small", title: "Mostrar todos los clips", onclick: () => reset(true) },
    svg(icons.close),
    "Limpiar",
  );

  for (const kind of KIND_OPTIONS) kindSelect.append(option(kind.id, kind.label));
  for (const range of RANGE_OPTIONS) rangeSelect.append(option(range.id, range.label));
  for (const sort of SORT_OPTIONS) sortSelect.append(option(sort.id, sort.label));

  const el = h(
    "div",
    { class: "filterbar", role: "search" },
    search,
    gameSelect,
    options.folderSelector ? folderSelect : null,
    kindSelect,
    rangeSelect,
    sortSelect,
    status,
    clear,
  );

  const changed = () => {
    clear.hidden = !isFiltering(state);
    options.onChange({ ...state });
  };

  search.addEventListener("input", () => {
    state.query = search.value;
    changed();
  });
  search.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && search.value !== "") {
      e.stopPropagation();
      search.value = "";
      state.query = "";
      changed();
    }
  });
  gameSelect.addEventListener("change", () => {
    state.game = gameSelect.value === "" ? null : gameSelect.value;
    changed();
  });
  folderSelect.addEventListener("change", () => {
    state.folder = folderSelect.value === "" ? null : folderSelect.value;
    changed();
  });
  kindSelect.addEventListener("change", () => {
    state.kind = kindSelect.value as KindFilter;
    changed();
  });
  rangeSelect.addEventListener("change", () => {
    state.range = rangeSelect.value as DateRange;
    changed();
  });
  sortSelect.addEventListener("change", () => {
    state.sort = sortSelect.value as SortKey;
    changed();
  });

  function syncControls() {
    search.value = state.query;
    gameSelect.value = state.game ?? "";
    folderSelect.value = state.folder ?? "";
    kindSelect.value = state.kind;
    rangeSelect.value = state.range;
    sortSelect.value = state.sort;
    clear.hidden = !isFiltering(state);
  }

  function reset(notify: boolean) {
    const sort = state.sort;
    Object.assign(state, emptyFilter(), { sort });
    syncControls();
    if (notify) changed();
  }

  // What the list is made of decides the words ("clips" or "elementos").
  let mixed = false;

  clear.hidden = true;
  syncControls();

  return {
    el,
    get: () => ({ ...state }),

    setData({ clips, folders }) {
      // Nothing to choose between until there is a screenshot.
      mixed = clips.some((c) => c.kind === "screenshot");
      kindSelect.hidden = !mixed;
      if (!mixed) state.kind = "all";

      const { games, withoutGame } = gameOptions(clips);
      gameSelect.replaceChildren(option("", "Todos los juegos"));
      for (const game of games) gameSelect.append(option(game.key, `${game.name} (${game.count})`));
      if (games.length > 0 && withoutGame > 0) {
        gameSelect.append(option(NONE, `Sin juego (${withoutGame})`));
      }
      // Nothing to choose between when no clip has a game.
      gameSelect.hidden = games.length === 0;
      const gameStillThere =
        state.game === null || [...games.map((g) => g.key), NONE].includes(state.game);
      if (!gameStillThere) state.game = null;

      const loose = clips.filter((c) => c.folders.length === 0).length;
      folderSelect.replaceChildren(option("", "Todas las carpetas"));
      for (const folder of sortFolders(folders)) {
        folderSelect.append(option(folder.id, `${folder.name} (${folder.clipCount})`));
      }
      if (folders.length > 0) folderSelect.append(option(NONE, `Sin carpeta (${loose})`));
      folderSelect.hidden = folders.length === 0;
      const folderStillThere =
        state.folder === null || state.folder === NONE || folders.some((f) => f.id === state.folder);
      if (!folderStillThere) state.folder = null;

      syncControls();
    },

    setCount(shown, total) {
      status.textContent = isFiltering(state) ? `${shown} de ${total} ${mixed ? "elementos" : "clips"}` : "";
      clear.hidden = !isFiltering(state);
    },

    reset: () => reset(false),

    keepFocus(change) {
      const focused = document.activeElement;
      const inside = focused instanceof HTMLElement && el.contains(focused) ? focused : null;
      change();
      if (inside && document.activeElement !== inside && el.contains(inside)) inside.focus();
    },
  };
}
