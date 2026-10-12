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
import { createDropdown, type DropdownOption } from "./dropdown";
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

const option = (value: string, label: string): DropdownOption => ({ value, label });

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
  // `""` is "all" for the game and the folder; NONE is "the ones without".
  const gameSelect = createDropdown({
    label: "Juego",
    onChange(value) {
      state.game = value === "" ? null : value;
      changed();
    },
  });
  const folderSelect = createDropdown({
    label: "Carpeta",
    onChange(value) {
      state.folder = value === "" ? null : value;
      changed();
    },
  });
  const kindSelect = createDropdown({
    label: "Tipo",
    options: KIND_OPTIONS.map((kind) => option(kind.id, kind.label)),
    onChange(value) {
      state.kind = value as KindFilter;
      changed();
    },
  });
  const rangeSelect = createDropdown({
    label: "Fecha",
    options: RANGE_OPTIONS.map((range) => option(range.id, range.label)),
    onChange(value) {
      state.range = value as DateRange;
      changed();
    },
  });
  const sortSelect = createDropdown({
    label: "Ordenar por",
    options: SORT_OPTIONS.map((sort) => option(sort.id, sort.label)),
    onChange(value) {
      state.sort = value as SortKey;
      changed();
    },
  });
  const status = h("span", { class: "filter-status muted" });
  const clear = h(
    "button",
    { class: "btn btn-small", title: "Mostrar todos los clips", onclick: () => reset(true) },
    svg(icons.close),
    "Limpiar",
  );

  const el = h(
    "div",
    { class: "filterbar", role: "search" },
    h("div", { class: "search-field" }, svg(icons.search), search),
    gameSelect.el,
    options.folderSelector ? folderSelect.el : null,
    kindSelect.el,
    rangeSelect.el,
    sortSelect.el,
    status,
    clear,
  );

  const changed = () => {
    clear.hidden = !isFiltering(state);
    markActive();
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
  /** A selector that narrows the list is drawn differently from one left on "all". */
  function markActive() {
    gameSelect.el.classList.toggle("is-set", state.game !== null);
    folderSelect.el.classList.toggle("is-set", state.folder !== null);
    kindSelect.el.classList.toggle("is-set", state.kind !== "all");
    rangeSelect.el.classList.toggle("is-set", state.range !== "all");
  }

  function syncControls() {
    search.value = state.query;
    gameSelect.setValue(state.game ?? "");
    folderSelect.setValue(state.folder ?? "");
    kindSelect.setValue(state.kind);
    rangeSelect.setValue(state.range);
    sortSelect.setValue(state.sort);
    clear.hidden = !isFiltering(state);
    markActive();
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
      kindSelect.el.hidden = !mixed;
      if (!mixed) {
        state.kind = "all";
        kindSelect.close();
      }

      const { games, withoutGame } = gameOptions(clips);
      const gameChoices = [
        option("", "Todos los juegos"),
        ...games.map((game) => option(game.key, `${game.name} (${game.count})`)),
      ];
      if (games.length > 0 && withoutGame > 0) gameChoices.push(option(NONE, `Sin juego (${withoutGame})`));
      gameSelect.setOptions(gameChoices);
      // Nothing to choose between when no clip has a game.
      gameSelect.el.hidden = games.length === 0;
      if (games.length === 0) gameSelect.close();
      const gameStillThere =
        state.game === null || [...games.map((g) => g.key), NONE].includes(state.game);
      if (!gameStillThere) state.game = null;

      const loose = clips.filter((c) => c.folders.length === 0).length;
      const folderChoices = [
        option("", "Todas las carpetas"),
        ...sortFolders(folders).map((folder) => option(folder.id, `${folder.name} (${folder.clipCount})`)),
      ];
      if (folders.length > 0) folderChoices.push(option(NONE, `Sin carpeta (${loose})`));
      folderSelect.setOptions(folderChoices);
      folderSelect.el.hidden = folders.length === 0;
      if (folders.length === 0) folderSelect.close();
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
