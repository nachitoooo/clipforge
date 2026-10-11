import * as api from "../api";
import { clipTitle, type ClipInfo } from "../lib/clips";
import { gameKey, validateGameName } from "../lib/filter";
import { h } from "./dom";
import { openModal } from "./modal";
import { attempt, toast } from "./toast";

export interface GameDialogOptions {
  clip: ClipInfo;
  /** The games already in the library, to suggest. */
  games: readonly string[];
  /** How many clips are of this game (for "change it in all of them"). */
  clipsOfGame: (game: string) => number;
  /** Something changed: reload. */
  onSaved: () => void;
}

let dialogs = 0;

/**
 * "Juego del clip": say which game a clip is from. The OS-reported name can be
 * ugly ("VALORANT-Win64-Shipping"), so the change can apply to every clip that
 * had the same game.
 */
export function openGameDialog(options: GameDialogOptions): void {
  const { clip } = options;
  const listId = `game-suggestions-${++dialogs}`;

  const input = h("input", {
    class: "text-input",
    value: clip.game ?? "",
    placeholder: "Por ejemplo: Valorant, Fortnite, Minecraft",
    "aria-label": "Juego",
    list: listId,
    spellcheck: "false",
  });
  const suggestions = h(
    "datalist",
    { id: listId },
    ...options.games.map((game) => h("option", { value: game })),
  );
  const error = h("p", { class: "field-error", role: "alert" });

  const others = clip.game === null ? 0 : options.clipsOfGame(clip.game);
  const allBox = h("input", { type: "checkbox", "aria-label": "Cambiar en todos los clips del juego" });
  const allRow =
    clip.game !== null && others > 1
      ? h(
          "label",
          { class: "check-row" },
          allBox,
          h("span", {}, `Cambiar en todos los clips de «${clip.game}» (${others})`),
        )
      : null;

  const save = h("button", { class: "btn btn-primary", onclick: () => void submit() }, "Guardar");
  const remove =
    clip.game === null
      ? null
      : h("button", { class: "btn", title: "Dejar el clip sin juego", onclick: () => void clearGame() }, "Quitar el juego");

  const dialog = h(
    "div",
    { class: "dialog game-dialog", role: "dialog", "aria-modal": "true", "aria-label": "Juego del clip" },
    h("h2", {}, "Juego del clip"),
    h("p", { class: "picker-clip", title: clip.name }, clipTitle(clip.name)),
    h("label", { class: "field" }, h("span", { class: "field-label" }, "Juego"), input, suggestions, error),
    allRow,
    h("div", { class: "dialog-actions" }, remove, h("button", { class: "btn", onclick: () => close() }, "Cancelar"), save),
  );
  const close = openModal(dialog);
  input.focus();
  input.select();

  // Typing the game it already has changes nothing worth saving.
  const unchanged = () => clip.game !== null && gameKey(input.value) === gameKey(clip.game) && input.value.trim() === clip.game;

  async function submit() {
    const check = validateGameName(input.value);
    if (!check.ok) {
      error.textContent = check.error;
      return;
    }
    if (unchanged()) {
      close();
      return;
    }
    let changed = 0;
    const ok = await attempt(async () => {
      changed = await api.setClipGame(clip.name, check.name, allBox.checked);
    });
    if (!ok) return;
    toast(changed > 1 ? `Juego cambiado en ${changed} clips` : "Juego actualizado");
    close();
    options.onSaved();
  }

  async function clearGame() {
    if (await attempt(() => api.setClipGame(clip.name, null, false))) {
      toast("El clip quedó sin juego");
      close();
      options.onSaved();
    }
  }

  input.addEventListener("input", () => {
    error.textContent = "";
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      void submit();
    }
  });
}
