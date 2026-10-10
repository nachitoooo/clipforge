import { h, svg } from "./dom";
import { icons } from "./icons";

/** A not-yet-built section: says honestly what's coming instead of faking it. */
export function renderPlaceholder(
  root: HTMLElement,
  options: { title: string; icon: string; description: string },
): void {
  root.replaceChildren(
    h("header", { class: "view-header" }, h("div", {}, h("h1", {}, options.title))),
    h(
      "div",
      { class: "view-body" },
      h(
        "div",
        { class: "empty" },
        svg(options.icon, "icon empty-icon"),
        h("h2", {}, "Próximamente"),
        h("p", { class: "muted" }, options.description),
      ),
    ),
  );
}

export const libraryPlaceholder = {
  title: "Biblioteca",
  icon: icons.library,
  description:
    "Acá vas a poder organizar tus clips en carpetas. Por ahora todos tus clips están en «Recientes».",
};
