import * as api from "../api";
import { bufferLabel, type BufferStatus } from "../lib/buffer";
import { validateClipName } from "../lib/clipname";
import { clipTitle, groupByDay, type ClipInfo } from "../lib/clips";
import { displayAccelerator } from "../lib/hotkey";
import { DEFAULT_SAVE_SHORTCUT } from "../lib/settings";
import { formatBytes, formatDuration, formatRelativeTime } from "../lib/format";
import { confirmDialog } from "./modal";
import { h, svg } from "./dom";
import { icons } from "./icons";
import { openEditor } from "./editor";
import { openPlayer } from "./player";
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

const PREVIEW_FRAME = 0.1; // seconds; forces the webview to paint a thumbnail

export function createGallery(root: HTMLElement): Gallery {
  let clips: ClipInfo[] = [];
  let loaded = false;
  let buffer: BufferStatus | null = null;
  let requestSeq = 0;
  let renaming = false;
  let headerEl: HTMLElement | null = null;
  let saveShortcut = DEFAULT_SAVE_SHORTCUT;
  const fresh = new Set<string>();

  const hotkeyLabel = () =>
    displayAccelerator(saveShortcut, navigator.userAgent.includes("Mac"));

  const lazyVideos = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const video = entry.target as HTMLVideoElement;
        lazyVideos.unobserve(video);
        video.src = video.dataset.src ?? "";
      }
    },
    { root: null, rootMargin: "400px" },
  );

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
        h("p", { class: "muted" }, loaded ? countLabel(clips.length) : "Cargando…"),
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

  // ---- cards ------------------------------------------------------------

  function card(clip: ClipInfo): HTMLElement {
    const duration = h("span", { class: "badge" }, "–:––");
    const video = h("video", {
      class: "thumb-video",
      preload: "metadata",
      "data-src": api.clipSrc(clip),
    });
    video.muted = true;
    video.loop = true;
    video.playsInline = true;
    video.addEventListener("loadedmetadata", () => {
      duration.textContent = formatDuration(video.duration);
      video.currentTime = Math.min(PREVIEW_FRAME, video.duration / 2);
    });
    lazyVideos.observe(video);

    const title = h("div", { class: "card-title", title: clip.name }, clipTitle(clip.name));
    const el = h(
      "article",
      { class: `card ${fresh.has(clip.name) ? "card-fresh" : ""}`, tabindex: "0", role: "button" },
      h(
        "div",
        { class: "thumb" },
        video,
        h("div", { class: "thumb-play" }, svg(icons.play)),
        duration,
        h(
          "div",
          { class: "card-actions" },
          actionButton("Recortar y editar", icons.cut, () => openEditor(clip)),
          actionButton("Renombrar", icons.rename, () => startRename(title, clip)),
          actionButton("Mostrar en carpeta", icons.folder, () => void run(() => api.revealClip(clip.name))),
          actionButton("Borrar", icons.trash, () => void remove(clip), true),
        ),
      ),
      h(
        "div",
        { class: "card-body" },
        title,
        h(
          "div",
          { class: "card-meta" },
          `${formatRelativeTime(clip.modifiedMs, Date.now())} · ${formatBytes(clip.sizeBytes)}`,
        ),
      ),
    );

    el.addEventListener("mouseenter", () => void video.play().catch(() => {}));
    el.addEventListener("mouseleave", () => {
      video.pause();
      video.currentTime = Math.min(PREVIEW_FRAME, video.duration / 2 || PREVIEW_FRAME);
    });
    el.addEventListener("click", () => openPlayer(clip));
    el.addEventListener("keydown", (e) => {
      if (e.target === el && (e.key === "Enter" || e.key === " ")) {
        e.preventDefault();
        openPlayer(clip);
      }
    });
    return el;
  }

  function actionButton(
    label: string,
    icon: string,
    onClick: () => void,
    danger = false,
  ): HTMLElement {
    return h(
      "button",
      {
        class: `icon-btn ${danger ? "icon-btn-danger" : ""}`,
        title: label,
        "aria-label": label,
        onclick: (e: Event) => {
          e.stopPropagation();
          onClick();
        },
      },
      svg(icon),
    );
  }

  // ---- actions ----------------------------------------------------------

  async function run(action: () => Promise<unknown>): Promise<boolean> {
    try {
      await action();
      return true;
    } catch (e) {
      toast(String(e), "error");
      return false;
    }
  }

  async function remove(clip: ClipInfo): Promise<void> {
    const ok = await confirmDialog({
      title: "¿Borrar este clip?",
      message: `«${clipTitle(clip.name)}» se va a borrar de tu disco. Esta acción no se puede deshacer.`,
      confirmLabel: "Borrar",
      danger: true,
    });
    if (!ok) return;
    if (await run(() => api.deleteClip(clip.name))) {
      toast("Clip borrado");
      await refresh();
    }
  }

  function startRename(titleEl: HTMLElement, clip: ClipInfo): void {
    if (renaming) return;
    renaming = true;
    const input = h("input", {
      class: "rename-input",
      value: clipTitle(clip.name),
      "aria-label": "Nuevo nombre del clip",
      spellcheck: "false",
    });
    let finished = false;

    const finish = async (commit: boolean) => {
      if (finished) return;
      finished = true;
      if (commit && input.value.trim() !== clipTitle(clip.name)) {
        const check = validateClipName(input.value);
        if (!check.ok) {
          toast(check.error, "error");
        } else {
          await run(() => api.renameClip(clip.name, check.stem));
        }
      }
      renaming = false;
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

  // ---- render / refresh -------------------------------------------------

  function render(): void {
    const body = h("div", { class: "view-body" });
    if (loaded && clips.length === 0) {
      body.append(
        h(
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
        ),
      );
    } else {
      for (const group of groupByDay(clips, Date.now())) {
        body.append(
          h(
            "section",
            { class: "group" },
            h("h2", { class: "group-title" }, group.label),
            h("div", { class: "grid" }, ...group.clips.map(card)),
          ),
        );
      }
    }
    headerEl = header();
    root.replaceChildren(headerEl, body);
  }

  async function refresh(): Promise<void> {
    // Don't tear down the input the user is typing in; finishing the rename
    // refreshes anyway.
    if (renaming) return;
    const seq = ++requestSeq;
    try {
      const next = await api.listClips();
      if (seq !== requestSeq) return; // a newer refresh superseded this one
      clips = next;
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

function countLabel(n: number): string {
  if (n === 0) return "Sin clips";
  return n === 1 ? "1 clip" : `${n} clips`;
}
