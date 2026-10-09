type Child = Node | string | null | false | undefined;
type Attrs = Record<string, string | boolean | EventListener | undefined>;

/**
 * Tiny element factory. Text children always go through `textContent`, never
 * `innerHTML`, so clip names (user data) can't inject markup.
 */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === undefined || value === false) continue;
    if (key.startsWith("on") && typeof value === "function") {
      el.addEventListener(key.slice(2).toLowerCase(), value);
    } else if (key === "class") {
      el.className = String(value);
    } else if (value === true) {
      el.setAttribute(key, "");
    } else {
      el.setAttribute(key, String(value));
    }
  }
  for (const child of children) {
    if (child === null || child === false || child === undefined) continue;
    el.append(child);
  }
  return el;
}

/** Static, trusted SVG markup only (see `icons.ts`). */
export function svg(markup: string, className = "icon"): SVGSVGElement {
  const wrapper = document.createElement("div");
  wrapper.innerHTML = markup.trim();
  const el = wrapper.firstElementChild as SVGSVGElement;
  el.setAttribute("class", className);
  el.setAttribute("aria-hidden", "true");
  return el;
}
