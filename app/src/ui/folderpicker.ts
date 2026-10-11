import * as api from "../api";
import { clipTitle, clipCountLabel, type ClipInfo } from "../lib/clips";
import {
  DEFAULT_FOLDER_COLOR,
  DEFAULT_FOLDER_ICON,
  folderColor,
  folderIconId,
  sortFolders,
  validateFolderName,
  type FolderInfo,
} from "../lib/folders";
import { h, svg } from "./dom";
import { folderIcons, icons } from "./icons";
import { openModal } from "./modal";
import { attempt, toast } from "./toast";

/**
 * "Carpetas del clip": tick the folders a clip should be in (it may be in
 * several), or make a new one on the spot. `onSaved` runs after a change.
 */
export function openFolderPicker(clip: ClipInfo, onSaved: () => void): void {
  let folders: FolderInfo[] = [];
  let original = new Set<string>();
  const selected = new Set<string>();

  const list = h("div", { class: "picker-list" }, h("p", { class: "muted" }, "Cargando carpetas…"));
  const input = h("input", {
    class: "text-input",
    placeholder: "Nueva carpeta…",
    "aria-label": "Nombre de la nueva carpeta",
    spellcheck: "false",
  });
  const error = h("p", { class: "field-error", role: "alert" });
  const createButton = h(
    "button",
    { class: "btn", title: "Crear la carpeta y agregar este clip", onclick: () => void create() },
    svg(icons.plus),
    "Crear",
  );
  const saveButton = h("button", { class: "btn btn-primary", onclick: () => void save() }, "Guardar");
  saveButton.disabled = true;

  const dialog = h(
    "div",
    {
      class: "dialog folder-picker",
      role: "dialog",
      "aria-modal": "true",
      "aria-label": `Carpetas de ${clipTitle(clip.name)}`,
    },
    h("h2", {}, "Carpetas del clip"),
    h("p", { class: "picker-clip", title: clip.name }, clipTitle(clip.name)),
    list,
    h("div", { class: "picker-new" }, input, createButton),
    error,
    h(
      "div",
      { class: "dialog-actions" },
      h("button", { class: "btn", onclick: () => close() }, "Cancelar"),
      saveButton,
    ),
  );
  const close = openModal(dialog);

  const changed = () =>
    selected.size !== original.size || [...selected].some((id) => !original.has(id));

  function renderList() {
    saveButton.disabled = !changed();
    if (folders.length === 0) {
      list.replaceChildren(
        h("p", { class: "muted" }, "Todavía no tenés carpetas. Creá la primera acá abajo."),
      );
      return;
    }
    list.replaceChildren(
      ...sortFolders(folders).map((folder) => {
        const box = h("input", { type: "checkbox", "data-folder": folder.id, "aria-label": folder.name });
        box.checked = selected.has(folder.id);
        box.addEventListener("change", () => {
          if (box.checked) selected.add(folder.id);
          else selected.delete(folder.id);
          saveButton.disabled = !changed();
        });
        return h(
          "label",
          { class: "picker-row", style: `--folder-color:${folderColor(folder.color)}` },
          box,
          h("span", { class: "picker-icon" }, svg(folderIcons[folderIconId(folder.icon)])),
          h("span", { class: "picker-name" }, folder.name),
          h("span", { class: "muted picker-count" }, clipCountLabel(folder.clipCount)),
        );
      }),
    );
  }

  async function create() {
    const check = validateFolderName(input.value, folders);
    if (!check.ok) {
      error.textContent = check.error;
      return;
    }
    error.textContent = "";
    let made: FolderInfo | undefined;
    const ok = await attempt(async () => {
      made = await api.createFolder({
        name: check.name,
        color: DEFAULT_FOLDER_COLOR,
        icon: DEFAULT_FOLDER_ICON,
      });
    });
    if (!ok || !made) return;
    folders.push(made);
    selected.add(made.id);
    input.value = "";
    renderList();
  }

  async function save() {
    if (!changed()) return;
    const ok = await attempt(() => api.setClipFolders(clip.name, [...selected]));
    if (!ok) return;
    toast("Carpetas actualizadas");
    close();
    onSaved();
  }

  input.addEventListener("keydown", (e) => {
    error.textContent = "";
    if (e.key === "Enter") {
      e.preventDefault();
      void create();
    }
  });

  api
    .listFolders()
    .then((found) => {
      folders = found;
      const known = new Set(found.map((f) => f.id));
      original = new Set(clip.folders.filter((id) => known.has(id)));
      for (const id of original) selected.add(id);
      renderList();
    })
    .catch((e) => {
      toast(String(e), "error");
      close();
    });
}
