import {
  DEFAULT_FOLDER_COLOR,
  FOLDER_COLORS,
  FOLDER_ICONS,
  folderColor,
  folderIconId,
  validateFolderName,
  type FolderDraft,
  type FolderInfo,
} from "../lib/folders";
import { h, svg } from "./dom";
import { folderIcons } from "./icons";
import { openModal } from "./modal";

/**
 * Create a folder, or edit one (`folder` given): name, color and icon.
 * Resolves with the checked values, or null if the user backs out. The caller
 * does the saving.
 */
export function openFolderDialog(options: {
  folder?: FolderInfo;
  existing: readonly FolderInfo[];
}): Promise<FolderDraft | null> {
  const { folder, existing } = options;
  return new Promise((resolve) => {
    let color = FOLDER_COLORS.some((c) => c.id === folder?.color) ? folder!.color : DEFAULT_FOLDER_COLOR;
    let icon = folderIconId(folder?.icon ?? "");
    let touched = false;
    let answered = false;

    const answer = (value: FolderDraft | null) => {
      if (answered) return;
      answered = true;
      close();
      resolve(value);
    };

    const title = folder ? "Editar carpeta" : "Nueva carpeta";
    const input = h("input", {
      class: "text-input",
      value: folder?.name ?? "",
      placeholder: "Por ejemplo: Valorant, Ranked, Mis mejores jugadas",
      "aria-label": "Nombre de la carpeta",
      spellcheck: "false",
    });
    const error = h("p", { class: "field-error", role: "alert" });
    const previewIcon = h("span", { class: "folder-preview-icon" });
    const previewName = h("span", { class: "folder-preview-name" });
    const preview = h("div", { class: "folder-preview" }, previewIcon, previewName);
    const submit = h("button", { class: "btn btn-primary", onclick: () => save() }, folder ? "Guardar" : "Crear carpeta");

    const swatches = FOLDER_COLORS.map((c) =>
      h("button", {
        class: "swatch",
        role: "radio",
        type: "button",
        title: c.label,
        "aria-label": c.label,
        "data-color": c.id,
        style: `background:${c.hex}`,
        onclick: () => {
          color = c.id;
          update();
        },
      }),
    );
    const iconChoices = FOLDER_ICONS.map((i) =>
      h(
        "button",
        {
          class: "icon-choice",
          role: "radio",
          type: "button",
          title: i.label,
          "aria-label": i.label,
          "data-icon": i.id,
          onclick: () => {
            icon = i.id;
            update();
          },
        },
        svg(folderIcons[i.id]),
      ),
    );

    function check() {
      return validateFolderName(input.value, existing, folder?.id);
    }

    function update() {
      const result = check();
      preview.style.setProperty("--folder-color", folderColor(color));
      previewIcon.replaceChildren(svg(folderIcons[icon]));
      previewName.textContent = input.value.trim() || "Nombre de la carpeta";
      previewName.classList.toggle("muted", input.value.trim() === "");
      for (const el of swatches) el.setAttribute("aria-checked", String(el.dataset.color === color));
      for (const el of iconChoices) el.setAttribute("aria-checked", String(el.dataset.icon === icon));
      error.textContent = touched && !result.ok ? result.error : "";
      submit.disabled = !result.ok;
    }

    function save() {
      const result = check();
      if (!result.ok) {
        touched = true;
        update();
        return;
      }
      answer({ name: result.name, color, icon });
    }

    input.addEventListener("input", () => {
      touched = true;
      update();
    });
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        save();
      }
    });

    const dialog = h(
      "div",
      { class: "dialog folder-dialog", role: "dialog", "aria-modal": "true", "aria-label": title },
      h("h2", {}, title),
      preview,
      h("label", { class: "field" }, h("span", { class: "field-label" }, "Nombre"), input, error),
      h(
        "div",
        { class: "field" },
        h("span", { class: "field-label" }, "Color"),
        h("div", { class: "swatches", role: "radiogroup", "aria-label": "Color" }, ...swatches),
      ),
      h(
        "div",
        { class: "field" },
        h("span", { class: "field-label" }, "Ícono"),
        h("div", { class: "icon-choices", role: "radiogroup", "aria-label": "Ícono" }, ...iconChoices),
      ),
      h(
        "div",
        { class: "dialog-actions" },
        h("button", { class: "btn", onclick: () => answer(null) }, "Cancelar"),
        submit,
      ),
    );
    const close = openModal(dialog, () => answer(null));
    update();
    input.focus();
    input.select();
  });
}
