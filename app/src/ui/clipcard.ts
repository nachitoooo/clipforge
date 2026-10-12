import * as api from "../api";
import { validateClipName } from "../lib/clipname";
import { clipTitle, type ClipInfo } from "../lib/clips";
import { gameKey, gameOptions } from "../lib/filter";
import { folderColor, foldersOf, type FolderInfo } from "../lib/folders";
import { formatBytes, formatDuration, formatRelativeTime } from "../lib/format";
import { h, svg } from "./dom";
import { openEditor } from "./editor";
import { openFolderPicker } from "./folderpicker";
import { openGameDialog } from "./gamedialog";
import { icons } from "./icons";
import { confirmDialog } from "./modal";
import { openPlayer } from "./player";
import { copyClip } from "./share";
import { attempt, toast } from "./toast";

/** The drag payload type: the file name of the clip being dragged. */
export const CLIP_DRAG_TYPE = "application/x-clipforge-clip";

const PREVIEW_FRAME = 0.1; // seconds; forces the webview to paint a thumbnail
const MAX_CHIPS = 2;

export interface ClipAction {
  label: string;
  icon: string;
  onClick: () => void;
  danger?: boolean;
}

/** A round icon button for a card's hover actions; clicking it never opens the card. */
export function actionButton(action: ClipAction): HTMLElement {
  return h(
    "button",
    {
      class: `icon-btn ${action.danger ? "icon-btn-danger" : ""}`,
      title: action.label,
      "aria-label": action.label,
      onclick: (e: Event) => {
        e.stopPropagation();
        action.onClick();
      },
    },
    svg(action.icon),
  );
}

export interface ClipCardsOptions {
  /** The folders that exist now, for the labels under each clip. */
  folders: () => FolderInfo[];
  /** Every clip in the library, for the games the game dialog suggests. */
  allClips: () => readonly ClipInfo[];
  /** Reload the view after an action changed something. */
  refresh: () => Promise<void>;
  isFresh?: (name: string) => boolean;
  /** Buttons to add after the usual ones (for instance "take out of this folder"). */
  extraActions?: (clip: ClipInfo) => ClipAction[];
}

export interface ClipCards {
  /**
   * The card of a clip. A card is built once and handed out again for as long
   * as nothing it shows has changed, so filtering or re-rendering a view
   * doesn't reload every thumbnail.
   */
  card(clip: ClipInfo, options?: { draggable?: boolean }): HTMLElement;
  /** Forget the cards of clips that are no longer in `names`. */
  prune(names: Iterable<string>): void;
  /** A rename box is open: the view must not rebuild its cards under it. */
  isRenaming(): boolean;
}

/**
 * The clip card shared by "Recientes" and the Biblioteca: thumbnail that plays
 * on hover, actions (edit, rename, folders, reveal, delete), folder labels.
 * One factory per view — the lazy-loading observer and the rename guard are
 * per view.
 */
export function createClipCards(options: ClipCardsOptions): ClipCards {
  let renaming = false;
  const cache = new Map<string, { signature: string; el: HTMLElement; video: HTMLVideoElement }>();

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

  /** The folders a clip is in, as small colored labels (the first ones; the rest in a tooltip). */
  function chips(clip: ClipInfo): HTMLElement | null {
    const inside = foldersOf(clip, options.folders());
    if (inside.length === 0) return null;
    const shown = inside.slice(0, MAX_CHIPS).map((folder) =>
      h(
        "span",
        { class: "chip", title: folder.name, style: `--folder-color:${folderColor(folder.color)}` },
        h("span", { class: "chip-dot" }),
        h("span", { class: "chip-name" }, folder.name),
      ),
    );
    const rest = inside.slice(MAX_CHIPS);
    if (rest.length > 0) {
      shown.push(
        h(
          "span",
          { class: "chip chip-more", title: rest.map((f) => f.name).join(", ") },
          `+${rest.length}`,
        ),
      );
    }
    return h("div", { class: "card-chips" }, ...shown);
  }

  async function remove(clip: ClipInfo): Promise<void> {
    const ok = await confirmDialog({
      title: "¿Borrar este clip?",
      message: `«${clipTitle(clip.name)}» se va a borrar de tu disco. Esta acción no se puede deshacer.`,
      confirmLabel: "Borrar",
      danger: true,
    });
    if (!ok) return;
    if (await attempt(() => api.deleteClip(clip.name))) {
      toast("Clip borrado");
      await options.refresh();
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
      forget(clip.name); // the card holds the input: it has to be built again
      if (commit && input.value.trim() !== clipTitle(clip.name)) {
        const check = validateClipName(input.value);
        if (!check.ok) {
          toast(check.error, "error");
        } else {
          await attempt(() => api.renameClip(clip.name, check.stem));
        }
      }
      renaming = false;
      await options.refresh();
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

  function forget(name: string): void {
    const entry = cache.get(name);
    if (entry) lazyVideos.unobserve(entry.video);
    cache.delete(name);
  }

  /** Everything a card shows: when it is the same, the card is too. */
  function signature(clip: ClipInfo, draggable: boolean): string {
    return [
      clip.path,
      clip.sizeBytes,
      clip.modifiedMs,
      clip.game ?? "",
      foldersOf(clip, options.folders())
        .map((f) => `${f.id}:${f.name}:${f.color}`)
        .join(","),
      options.isFresh?.(clip.name) ? "fresh" : "",
      draggable ? "drag" : "",
      options.extraActions?.(clip).map((a) => a.label).join(",") ?? "",
      Math.floor(Date.now() / 60_000), // "hace 5 min" goes stale
    ].join("|");
  }

  function card(clip: ClipInfo, cardOptions: { draggable?: boolean } = {}): HTMLElement {
    const signature_ = signature(clip, cardOptions.draggable === true);
    const cached = cache.get(clip.name);
    if (cached?.signature === signature_) return cached.el;
    forget(clip.name);
    const built = build(clip, cardOptions);
    cache.set(clip.name, { signature: signature_, ...built });
    return built.el;
  }

  function build(
    clip: ClipInfo,
    cardOptions: { draggable?: boolean },
  ): { el: HTMLElement; video: HTMLVideoElement } {
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
    const actions: ClipAction[] = [
      { label: "Recortar y editar", icon: icons.cut, onClick: () => openEditor(clip) },
      { label: "Renombrar", icon: icons.rename, onClick: () => startRename(title, clip) },
      {
        label: "Carpetas…",
        icon: icons.folderPlus,
        onClick: () => openFolderPicker(clip, () => void options.refresh()),
      },
      ...(options.extraActions?.(clip) ?? []),
      {
        label: "Copiar el clip para pegarlo en un chat o un mail",
        icon: icons.copy,
        onClick: () => void copyClip(clip),
      },
      {
        label: "Mostrar el archivo en el disco",
        icon: icons.reveal,
        onClick: () => void attempt(() => api.revealClip(clip.name)),
      },
      { label: "Borrar", icon: icons.trash, onClick: () => void remove(clip), danger: true },
    ];

    const el = h(
      "article",
      {
        class: `card ${options.isFresh?.(clip.name) ? "card-fresh" : ""}`,
        tabindex: "0",
        role: "button",
        draggable: cardOptions.draggable ? "true" : undefined,
      },
      h(
        "div",
        { class: "thumb" },
        video,
        h("div", { class: "thumb-play" }, svg(icons.play)),
        duration,
        h("div", { class: "card-actions" }, ...actions.map(actionButton)),
      ),
      h(
        "div",
        { class: "card-body" },
        title,
        h(
          "div",
          { class: "card-meta" },
          h(
            "button",
            {
              class: `game-link ${clip.game === null ? "game-none" : ""}`,
              title: clip.game === null ? "Decir de qué juego es este clip" : "Cambiar el juego de este clip",
              onclick: (e: Event) => {
                e.stopPropagation();
                editGame(clip);
              },
            },
            clip.game ?? "Sin juego",
          ),
          h(
            "span",
            { class: "card-meta-rest" },
            ` · ${formatRelativeTime(clip.modifiedMs, Date.now())} · ${formatBytes(clip.sizeBytes)}`,
          ),
        ),
        chips(clip),
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
    if (cardOptions.draggable) {
      el.addEventListener("dragstart", (e) => {
        e.dataTransfer?.setData(CLIP_DRAG_TYPE, clip.name);
        if (e.dataTransfer) e.dataTransfer.effectAllowed = "copy";
        el.classList.add("dragging");
      });
      el.addEventListener("dragend", () => el.classList.remove("dragging"));
    }
    return { el, video };
  }

  function editGame(clip: ClipInfo): void {
    const clips = options.allClips();
    openGameDialog({
      clip,
      games: gameOptions(clips).games.map((g) => g.name),
      clipsOfGame: (game) =>
        clips.filter((c) => c.game !== null && gameKey(c.game) === gameKey(game)).length,
      onSaved: () => void options.refresh(),
    });
  }

  return {
    card,
    prune(names) {
      const keep = new Set(names);
      for (const name of [...cache.keys()]) if (!keep.has(name)) forget(name);
    },
    isRenaming: () => renaming,
  };
}
