import * as api from "../api";
import { clipCountLabel, clipTitle, groupByDay, type ClipInfo } from "../lib/clips";
import { applyFilter, isFiltering, layoutFor, normalizeText } from "../lib/filter";
import {
  clipsIn,
  deleteBreakdown,
  folderColor,
  folderIconId,
  looseClips,
  sortFolders,
  validateFolderName,
  type FolderInfo,
} from "../lib/folders";
import { formatBytes } from "../lib/format";
import { CLIP_DRAG_TYPE, actionButton, createClipCards } from "./clipcard";
import { h, svg } from "./dom";
import { createFilterBar, type FilterBar } from "./filterbar";
import { openFolderDialog } from "./folderdialog";
import { folderIcons, icons } from "./icons";
import { choiceDialog, confirmDialog } from "./modal";
import { attempt, toast } from "./toast";

export interface Library {
  /** Reload clips and folders from the backend. */
  refresh(): Promise<void>;
  /** The Biblioteca was just opened: show fresh data. */
  onShow(): void;
  /** Highlight a freshly saved clip (call before `refresh`). */
  markFresh(name: string): void;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "3 clips · 120 MB", or just "Sin clips". */
function folderMeta(folder: FolderInfo): string {
  return folder.clipCount === 0
    ? clipCountLabel(0)
    : `${clipCountLabel(folder.clipCount)} · ${formatBytes(folder.sizeBytes)}`;
}

/**
 * The Biblioteca: the user's folders and, under them, the clips that are in no
 * folder yet. Opening a folder shows its clips. A clip can be in several
 * folders; dragging one onto a folder adds it there.
 */
export function createLibrary(root: HTMLElement): Library {
  let clips: ClipInfo[] = [];
  let folders: FolderInfo[] = [];
  let loaded = false;
  let openId: string | null = null;
  let requestSeq = 0;
  let renamingFolder = false;
  const fresh = new Set<string>();

  const cards = createClipCards({
    folders: () => folders,
    allClips: () => clips,
    refresh: () => refresh(),
    isFresh: (name) => fresh.has(name),
    extraActions: (clip) =>
      openId === null
        ? []
        : [
            {
              label: "Sacar de esta carpeta",
              icon: icons.folderMinus,
              onClick: () => void takeOut(clip),
            },
          ],
  });

  const folderById = (id: string) => folders.find((f) => f.id === id);

  // One filter bar at the root (it can also pick a folder) and one inside a
  // folder (where the folder is already chosen). Each keeps its own filter.
  const rootBar = createFilterBar({ onChange: () => renderBody(), folderSelector: true });
  const folderBar = createFilterBar({ onChange: () => renderBody() });
  /** What is on screen, so a filter change only has to redo the list. */
  let current: { bar: FilterBar | null; body: () => HTMLElement } | null = null;
  let bodyEl: HTMLElement | null = null;

  // ---- folder actions ---------------------------------------------------

  async function newFolder(): Promise<void> {
    const draft = await openFolderDialog({ existing: folders });
    if (!draft) return;
    if (await attempt(() => api.createFolder(draft))) {
      toast(`Carpeta «${draft.name}» creada`);
      await refresh();
    }
  }

  async function editFolder(folder: FolderInfo): Promise<void> {
    const draft = await openFolderDialog({ folder, existing: folders });
    if (!draft) return;
    if (await attempt(() => api.updateFolder(folder.id, draft))) {
      toast("Carpeta actualizada");
      await refresh();
    }
  }

  async function deleteFolder(folder: FolderInfo): Promise<void> {
    const { total, exclusive, shared } = deleteBreakdown(folder, clips);
    let deleteClips = false;
    if (total === 0) {
      const ok = await confirmDialog({
        title: "¿Borrar la carpeta?",
        message: `«${folder.name}» está vacía y se va a borrar.`,
        confirmLabel: "Borrar",
        danger: true,
      });
      if (!ok) return;
    } else if (exclusive === 0) {
      const ok = await confirmDialog({
        title: "¿Borrar la carpeta?",
        message: `«${folder.name}» tiene ${plural(total, "clip", "clips")}, y ${total === 1 ? "está" : "están"} también en otras carpetas. Los clips no se borran.`,
        confirmLabel: "Borrar carpeta",
        danger: true,
      });
      if (!ok) return;
    } else {
      const sharedNote =
        shared > 0
          ? ` ${plural(shared, "clip está", "clips están")} también en otras carpetas y se conserva${shared === 1 ? "" : "n"} igual.`
          : "";
      const choice = await choiceDialog({
        title: `¿Borrar la carpeta «${folder.name}»?`,
        message: `Tiene ${plural(total, "clip", "clips")}. Podés conservarlos (siguen en Recientes) o borrar también ${plural(exclusive, "clip que no está", "clips que no están")} en ninguna otra carpeta.${sharedNote}`,
        choices: [
          { value: "keep", label: "Conservar los clips" },
          {
            value: "all",
            label: `Borrar carpeta y ${plural(exclusive, "clip", "clips")}`,
            danger: true,
          },
        ],
      });
      if (choice === null) return;
      deleteClips = choice === "all";
    }

    let deleted = 0;
    const ok = await attempt(async () => {
      deleted = await api.deleteFolder(folder.id, deleteClips);
    });
    if (ok) {
      toast(deleted > 0 ? `Carpeta borrada junto con ${plural(deleted, "clip", "clips")}` : "Carpeta borrada");
      if (openId === folder.id) openId = null;
    }
    await refresh();
  }

  async function fileClip(folder: FolderInfo, name: string): Promise<void> {
    let added = 0;
    const ok = await attempt(async () => {
      added = await api.addToFolder(folder.id, [name]);
    });
    if (!ok) return;
    toast(
      added > 0
        ? `«${clipTitle(name)}» agregado a «${folder.name}»`
        : `«${clipTitle(name)}» ya estaba en «${folder.name}»`,
    );
    await refresh();
  }

  async function takeOut(clip: ClipInfo): Promise<void> {
    if (openId === null) return;
    const folder = folderById(openId);
    if (await attempt(() => api.removeFromFolder(openId!, [clip.name]))) {
      toast(`«${clipTitle(clip.name)}» salió de «${folder?.name ?? "la carpeta"}»`);
      await refresh();
    }
  }

  function startFolderRename(titleEl: HTMLElement, folder: FolderInfo): void {
    if (renamingFolder) return;
    renamingFolder = true;
    const input = h("input", {
      class: "rename-input",
      value: folder.name,
      "aria-label": "Nuevo nombre de la carpeta",
      spellcheck: "false",
    });
    let finished = false;

    const finish = async (commit: boolean) => {
      if (finished) return;
      finished = true;
      if (commit && input.value.trim() !== folder.name) {
        const check = validateFolderName(input.value, folders, folder.id);
        if (!check.ok) {
          toast(check.error, "error");
        } else {
          await attempt(() =>
            api.updateFolder(folder.id, { name: check.name, color: folder.color, icon: folder.icon }),
          );
        }
      }
      renamingFolder = false;
      await refresh();
    };

    input.addEventListener("click", (e) => e.stopPropagation());
    input.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") void finish(true);
      if (e.key === "Escape") void finish(false);
    });
    input.addEventListener("blur", () => void finish(true));
    titleEl.replaceChildren(input);
    input.focus();
    input.select();
  }

  // ---- folder cards -----------------------------------------------------

  function folderCard(folder: FolderInfo): HTMLElement {
    const title = h("div", { class: "card-title", title: folder.name }, folder.name);
    const el = h(
      "article",
      {
        class: "card folder-card",
        tabindex: "0",
        role: "button",
        "data-folder": folder.id,
        "aria-label": `Carpeta ${folder.name}`,
        style: `--folder-color:${folderColor(folder.color)}`,
      },
      h(
        "div",
        { class: "folder-cover" },
        svg(folderIcons[folderIconId(folder.icon)], "icon folder-cover-icon"),
        h(
          "div",
          { class: "card-actions" },
          ...[
            { label: "Renombrar", icon: icons.rename, onClick: () => startFolderRename(title, folder) },
            { label: "Editar carpeta", icon: icons.palette, onClick: () => void editFolder(folder) },
            { label: "Borrar carpeta", icon: icons.trash, onClick: () => void deleteFolder(folder), danger: true },
          ].map(actionButton),
        ),
      ),
      h("div", { class: "card-body" }, title, h("div", { class: "card-meta" }, folderMeta(folder))),
    );

    const open = () => {
      openId = folder.id;
      folderBar.reset(); // each visit to a folder starts unfiltered
      render();
    };
    el.addEventListener("click", open);
    el.addEventListener("keydown", (e) => {
      if (e.target === el && (e.key === "Enter" || e.key === " ")) {
        e.preventDefault();
        open();
      }
    });

    // Dropping a clip files it here (it also stays wherever it was).
    const carriesClip = (e: DragEvent) =>
      Array.from(e.dataTransfer?.types ?? []).includes(CLIP_DRAG_TYPE);
    el.addEventListener("dragover", (e) => {
      if (!carriesClip(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
      el.classList.add("drop-target");
    });
    el.addEventListener("dragleave", (e) => {
      const into = e.relatedTarget;
      if (!(into instanceof Node) || !el.contains(into)) el.classList.remove("drop-target");
    });
    el.addEventListener("drop", (e) => {
      if (!carriesClip(e)) return;
      e.preventDefault();
      el.classList.remove("drop-target");
      const name = e.dataTransfer?.getData(CLIP_DRAG_TYPE);
      if (name) void fileClip(folder, name);
    });
    return el;
  }

  // ---- views ------------------------------------------------------------

  const grid = (clips: ClipInfo[], draggable = false) =>
    h("div", { class: "grid" }, ...clips.map((clip) => cards.card(clip, { draggable })));

  /** Clips under their days, or in one list when the order isn't by date. */
  function clipSections(shown: ClipInfo[], sort: Parameters<typeof layoutFor>[0]): HTMLElement[] {
    if (layoutFor(sort) === "flat") return [h("section", { class: "group" }, grid(shown))];
    return groupByDay(shown, Date.now()).map((group) =>
      h("section", { class: "group" }, h("h2", { class: "group-title" }, group.label), grid(group.clips)),
    );
  }

  function noMatches(bar: FilterBar): HTMLElement {
    return h(
      "div",
      { class: "empty" },
      svg(icons.search, "icon empty-icon"),
      h("h2", {}, "Ningún clip coincide"),
      h("p", { class: "muted" }, "Probá con otras palabras o sacá algún filtro."),
      h(
        "button",
        {
          class: "btn",
          onclick: () => {
            bar.reset();
            renderBody();
          },
        },
        "Limpiar filtros",
      ),
    );
  }

  function rootParts() {
    const header = h(
      "header",
      { class: "view-header" },
      h(
        "div",
        {},
        h("h1", {}, "Biblioteca"),
        h(
          "p",
          { class: "muted" },
          loaded
            ? `${plural(folders.length, "carpeta", "carpetas")} · ${clipCountLabel(clips.length)}`
            : "Cargando…",
        ),
      ),
      h(
        "div",
        { class: "header-actions" },
        h(
          "button",
          { class: "btn btn-primary", onclick: () => void newFolder() },
          svg(icons.folderPlus),
          "Nueva carpeta",
        ),
      ),
    );

    const body = (): HTMLElement => {
      const body = h("div", { class: "view-body" });
      if (!loaded) return body;
      const filter = rootBar.get();
      rootBar.setCount(applyFilter(clips, folders, filter, Date.now()).length, clips.length);

      // Searching or filtering turns the Biblioteca into a list of results over
      // every clip, folders or not, plus the folders whose name matches.
      if (isFiltering(filter)) {
        const results = applyFilter(clips, folders, filter, Date.now());
        const words = normalizeText(filter.query).split(/\s+/).filter(Boolean);
        const onlySearching = filter.game === null && filter.folder === null && filter.range === "all";
        const matching =
          onlySearching && words.length > 0
            ? sortFolders(folders).filter((f) => words.every((w) => normalizeText(f.name).includes(w)))
            : [];
        if (matching.length > 0) {
          body.append(
            h(
              "section",
              { class: "group", "data-section": "folders" },
              h("h2", { class: "group-title" }, "Carpetas"),
              h("div", { class: "grid" }, ...matching.map(folderCard)),
            ),
          );
        }
        if (results.length > 0) {
          body.append(
            h(
              "section",
              { class: "group", "data-section": "results" },
              h("h2", { class: "group-title" }, `Clips · ${results.length}`),
              grid(results),
            ),
          );
        } else if (matching.length === 0) {
          body.append(noMatches(rootBar));
        }
        return body;
      }

      const folderSection = h("section", { class: "group", "data-section": "folders" });
      folderSection.append(h("h2", { class: "group-title" }, "Carpetas"));
      if (folders.length === 0) {
        folderSection.append(
          h(
            "div",
            { class: "empty-inline" },
            svg(icons.folderPlus, "icon empty-icon"),
            h(
              "p",
              { class: "muted" },
              "Todavía no armaste carpetas. Hacé una por juego, por tanda de partidas o para tus mejores jugadas, y metele los clips que quieras: un clip puede estar en varias a la vez.",
            ),
            h(
              "button",
              { class: "btn btn-primary", onclick: () => void newFolder() },
              svg(icons.folderPlus),
              "Crear la primera carpeta",
            ),
          ),
        );
      } else {
        folderSection.append(h("div", { class: "grid" }, ...sortFolders(folders).map(folderCard)));
      }
      body.append(folderSection);

      const loose = applyFilter(looseClips(clips), folders, filter, Date.now()); // sorted
      const looseSection = h("section", { class: "group", "data-section": "loose" });
      looseSection.append(h("h2", { class: "group-title" }, `Sin carpeta · ${loose.length}`));
      if (loose.length > 0) {
        if (folders.length > 0) {
          looseSection.append(
            h(
              "p",
              { class: "muted hint" },
              "Arrastrá un clip sobre una carpeta para agregarlo, o usá el botón de carpetas de cada clip.",
            ),
          );
        }
        looseSection.append(grid(loose, true));
      } else {
        looseSection.append(
          h(
            "p",
            { class: "muted" },
            clips.length === 0
              ? "Todavía no hay clips. Cuando guardes uno aparece acá."
              : "Todos tus clips están en alguna carpeta.",
          ),
        );
      }
      body.append(looseSection);
      return body;
    };

    rootBar.setData({ clips, folders });
    return { header, bar: clips.length > 0 ? rootBar : null, body };
  }

  function folderParts(folder: FolderInfo) {
    const inside = clipsIn(clips, folder.id);
    const header = h(
      "header",
      { class: "view-header", style: `--folder-color:${folderColor(folder.color)}` },
      h(
        "div",
        {},
        h(
          "nav",
          { class: "breadcrumb", "aria-label": "Ubicación" },
          h(
            "button",
            {
              class: "crumb",
              onclick: () => {
                openId = null;
                render();
              },
            },
            svg(icons.back),
            "Biblioteca",
          ),
        ),
        h(
          "h1",
          { class: "folder-heading" },
          svg(folderIcons[folderIconId(folder.icon)], "icon folder-heading-icon"),
          folder.name,
        ),
        h("p", { class: "muted" }, folderMeta(folder)),
      ),
      h(
        "div",
        { class: "header-actions" },
        h(
          "button",
          { class: "btn", onclick: () => void editFolder(folder) },
          svg(icons.palette),
          "Editar",
        ),
        h(
          "button",
          { class: "btn", onclick: () => void deleteFolder(folder) },
          svg(icons.trash),
          "Borrar",
        ),
      ),
    );

    const body = (): HTMLElement => {
      const body = h("div", { class: "view-body" });
      if (inside.length === 0) {
        body.append(
          h(
            "div",
            { class: "empty" },
            svg(folderIcons[folderIconId(folder.icon)], "icon empty-icon"),
            h("h2", {}, "Esta carpeta está vacía"),
            h(
              "p",
              { class: "muted" },
              "Agregale clips con el botón de carpetas que aparece al pasar el mouse sobre un clip (en Recientes o en «Sin carpeta»), o arrastrándolos sobre la carpeta desde la Biblioteca.",
            ),
          ),
        );
        return body;
      }
      const filter = folderBar.get();
      const shown = applyFilter(inside, folders, filter, Date.now());
      folderBar.setCount(shown.length, inside.length);
      if (shown.length === 0) body.append(noMatches(folderBar));
      else body.append(...clipSections(shown, filter.sort));
      return body;
    };

    folderBar.setData({ clips: inside, folders });
    return { header, bar: inside.length > 0 ? folderBar : null, body };
  }

  function render(): void {
    const folder = openId === null ? undefined : folderById(openId);
    if (openId !== null && !folder) openId = null;
    cards.prune(clips.map((c) => c.name));
    const parts = folder ? folderParts(folder) : rootParts();
    current = parts;
    bodyEl = parts.body();
    // Redoing the view must not take the focus out of the search box.
    const keeper = parts.bar ?? rootBar;
    keeper.keepFocus(() =>
      root.replaceChildren(...(parts.bar ? [parts.header, parts.bar.el, bodyEl!] : [parts.header, bodyEl!])),
    );
  }

  /** The filter changed: only the list under the bar is redone. */
  function renderBody(): void {
    if (!current || !bodyEl) return;
    const next = current.body();
    bodyEl.replaceWith(next);
    bodyEl = next;
  }

  async function refresh(): Promise<void> {
    // Don't tear down the input the user is typing in; finishing the rename
    // refreshes anyway.
    if (renamingFolder || cards.isRenaming()) return;
    const seq = ++requestSeq;
    try {
      const [nextClips, nextFolders] = await Promise.all([api.listClips(), api.listFolders()]);
      if (seq !== requestSeq) return; // a newer refresh superseded this one
      clips = nextClips;
      folders = nextFolders;
      loaded = true;
    } catch (e) {
      if (seq !== requestSeq) return;
      toast(String(e), "error");
      loaded = true;
    }
    render();
    window.setTimeout(() => fresh.clear(), 3000);
  }

  window.addEventListener("focus", () => {
    if (!root.hidden) void refresh();
  });

  render();
  return {
    refresh,
    onShow: () => void refresh(),
    markFresh: (name) => fresh.add(name),
  };
}
