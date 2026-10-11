import * as api from "../api";
import { bufferLabel, type BufferStatus } from "../lib/buffer";
import { clipCountLabel, groupByDay, type ClipInfo } from "../lib/clips";
import { applyFilter, layoutFor } from "../lib/filter";
import type { FolderInfo } from "../lib/folders";
import { displayAccelerator } from "../lib/hotkey";
import { DEFAULT_SAVE_SHORTCUT } from "../lib/settings";
import { createClipCards } from "./clipcard";
import { h, svg } from "./dom";
import { createFilterBar } from "./filterbar";
import { icons } from "./icons";
import { toast } from "./toast";

export interface Gallery {
  /** Reload the clip list from disk. */
  refresh(): Promise<void>;
  /** Highlight a freshly saved clip (call before `refresh`). */
  markFresh(name: string): void;
  setBufferStatus(status: BufferStatus): void;
  /** The global shortcut that saves a clip changed: show the new one. */
  setSaveShortcut(accelerator: string): void;
}

export function createGallery(root: HTMLElement): Gallery {
  let clips: ClipInfo[] = [];
  let folders: FolderInfo[] = [];
  let loaded = false;
  let buffer: BufferStatus | null = null;
  let requestSeq = 0;
  let headerEl: HTMLElement | null = null;
  let saveShortcut = DEFAULT_SAVE_SHORTCUT;
  const fresh = new Set<string>();

  const hotkeyLabel = () =>
    displayAccelerator(saveShortcut, navigator.userAgent.includes("Mac"));

  const cards = createClipCards({
    folders: () => folders,
    allClips: () => clips,
    refresh: () => refresh(),
    isFresh: (name) => fresh.has(name),
  });
  const bar = createFilterBar({ onChange: () => renderBody(), folderSelector: true });
  let bodyEl: HTMLElement | null = null;

  // ---- header -----------------------------------------------------------

  function bufferPill(): HTMLElement {
    const running = buffer?.running ?? false;
    return h(
      "button",
      {
        class: `pill ${running ? "pill-live" : ""} ${buffer?.error ? "pill-error" : ""}`,
        title: running ? "Detener el buffer de replay" : "Iniciar el buffer de replay",
        onclick: () => void api.toggleBuffer(),
      },
      h("span", { class: "dot" }),
      bufferLabel(buffer),
    );
  }

  /** Explains why capture isn't running, with a way to try again. */
  function bufferBanner(error: string): HTMLElement {
    return h(
      "div",
      { class: "banner", role: "alert" },
      h(
        "div",
        {},
        h("strong", {}, "No se pudo iniciar la captura. "),
        h("span", {}, error),
      ),
      h("button", { class: "btn", onclick: () => void api.toggleBuffer() }, "Reintentar"),
    );
  }

  function header(): HTMLElement {
    return h(
      "header",
      { class: "view-header" },
      h(
        "div",
        {},
        h("h1", {}, "Clips recientes"),
        h("p", { class: "muted" }, loaded ? clipCountLabel(clips.length) : "Cargando…"),
      ),
      h(
        "div",
        { class: "header-actions" },
        bufferPill(),
        h(
          "button",
          { class: "btn", title: "Abrir la carpeta de clips", onclick: () => void api.openClipsFolder() },
          svg(icons.folder),
          "Carpeta",
        ),
        h(
          "button",
          {
            class: "btn btn-primary",
            title: "Guardar los últimos 30 s",
            onclick: () => void api.saveClip(),
          },
          svg(icons.record),
          "Guardar clip",
          h("kbd", {}, hotkeyLabel()),
        ),
      ),
      buffer?.error ? bufferBanner(buffer.error) : null,
    );
  }

  // ---- render / refresh -------------------------------------------------

  const visibleClips = () => applyFilter(clips, folders, bar.get(), Date.now());

  function emptyState(): HTMLElement {
    return h(
      "div",
      { class: "empty" },
      svg(icons.forge, "icon empty-icon"),
      h("h2", {}, "Todavía no hay clips"),
      h(
        "p",
        { class: "muted" },
        "Jugá tranquilo: ClipForge guarda los últimos segundos en segundo plano. Cuando pase algo bueno, apretá ",
        h("kbd", {}, hotkeyLabel()),
        " y el clip aparece acá.",
      ),
    );
  }

  function noMatches(): HTMLElement {
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

  function buildBody(): HTMLElement {
    const body = h("div", { class: "view-body" });
    if (loaded && clips.length === 0) {
      body.append(emptyState());
      return body;
    }
    const shown = visibleClips();
    bar.setCount(shown.length, clips.length);
    if (loaded && shown.length === 0) {
      body.append(noMatches());
    } else if (layoutFor(bar.get().sort) === "by-day") {
      for (const group of groupByDay(shown, Date.now())) {
        body.append(
          h(
            "section",
            { class: "group" },
            h("h2", { class: "group-title" }, group.label),
            h("div", { class: "grid" }, ...group.clips.map((clip) => cards.card(clip))),
          ),
        );
      }
    } else {
      body.append(
        h("section", { class: "group" }, h("div", { class: "grid" }, ...shown.map((clip) => cards.card(clip)))),
      );
    }
    return body;
  }

  /** The filter changed: only the list under the bar is redone. */
  function renderBody(): void {
    if (!bodyEl) return;
    const next = buildBody();
    bodyEl.replaceWith(next);
    bodyEl = next;
  }

  function render(): void {
    cards.prune(clips.map((c) => c.name));
    bar.setData({ clips, folders });
    bodyEl = buildBody();
    headerEl = header();
    const showBar = loaded && clips.length > 0;
    // Redoing the view must not take the focus out of the search box.
    bar.keepFocus(() =>
      root.replaceChildren(...(showBar ? [headerEl!, bar.el, bodyEl!] : [headerEl!, bodyEl!])),
    );
  }

  async function refresh(): Promise<void> {
    // Don't tear down the input the user is typing in; finishing the rename
    // refreshes anyway.
    if (cards.isRenaming()) return;
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

  window.addEventListener("focus", () => void refresh());

  render();
  return {
    refresh,
    markFresh: (name) => fresh.add(name),
    setSaveShortcut(accelerator) {
      if (accelerator === saveShortcut) return;
      saveShortcut = accelerator;
      if (loaded && clips.length === 0) {
        render(); // the empty state mentions it too
      } else if (headerEl) {
        const next = header();
        headerEl.replaceWith(next);
        headerEl = next;
      }
    },
    setBufferStatus(status) {
      buffer = status;
      // Only the header changes; re-rendering the grid would reload every thumbnail.
      if (headerEl) {
        const next = header();
        headerEl.replaceWith(next);
        headerEl = next;
      }
    },
  };
}
