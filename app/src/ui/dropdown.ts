import { h, svg } from "./dom";
import { icons } from "./icons";

export interface DropdownOption {
  value: string;
  label: string;
}

export interface DropdownConfig {
  /** Names the control for screen readers: «Juego», «Fecha». */
  label: string;
  value?: string;
  options?: readonly DropdownOption[];
  /** The person picked a different option. Not called for `setValue`. */
  onChange(value: string): void;
}

export interface Dropdown {
  /** Put this in the page; it holds the trigger and the list. */
  el: HTMLElement;
  trigger: HTMLButtonElement;
  setOptions(options: readonly DropdownOption[]): void;
  /** Show `value` as chosen, without telling `onChange`. */
  setValue(value: string): void;
  value(): string;
  close(): void;
}

/** How long the letters typed to jump to an option are remembered. */
const TYPEAHEAD_MS = 700;
const MENU_MAX_HEIGHT = 320;
const GAP = 6;
const EDGE = 8;

/** Only one list is open at a time. */
let openDropdown: Dropdown | null = null;
let counter = 0;

/**
 * A drop-down list that replaces the browser's `<select>`: a trigger showing the
 * choice and a floating list with a tick on the chosen option. Arrow keys, Home,
 * End, Enter, Escape and typing a few letters all work; it opens upwards when
 * there is no room below and closes when you click elsewhere or scroll the page.
 */
export function createDropdown(config: DropdownConfig): Dropdown {
  const id = `dropdown-${++counter}`;
  let options = [...(config.options ?? [])];
  let value = config.value ?? options[0]?.value ?? "";
  let active = -1;
  let isOpen = false;
  let typed = "";
  let typedTimer: ReturnType<typeof setTimeout> | undefined;

  const valueText = h("span", { class: "dropdown-value", id: `${id}-value` });
  const trigger = h(
    "button",
    {
      type: "button",
      class: "dropdown-trigger",
      "aria-haspopup": "listbox",
      "aria-expanded": "false",
      "aria-controls": `${id}-list`,
      "aria-describedby": `${id}-value`,
      "aria-label": config.label,
    },
    valueText,
    svg(icons.chevron, "icon dropdown-chevron"),
  );
  const menu = h("div", {
    class: "dropdown-menu",
    id: `${id}-list`,
    role: "listbox",
    tabindex: "-1",
    "aria-label": config.label,
    hidden: true,
  });
  const el = h("div", { class: "dropdown", "data-label": config.label }, trigger, menu);

  const items = () => [...menu.querySelectorAll<HTMLElement>(".dropdown-item")];

  function sync() {
    const chosen = options.find((o) => o.value === value) ?? options[0];
    valueText.textContent = chosen?.label ?? "";
    el.dataset.value = value;
    items().forEach((item, i) => {
      const selected = options[i]?.value === value;
      item.classList.toggle("is-selected", selected);
      item.setAttribute("aria-selected", String(selected));
    });
  }

  function renderItems() {
    menu.replaceChildren(
      ...options.map((option, i) =>
        h(
          "div",
          {
            class: "dropdown-item",
            id: `${id}-option-${i}`,
            role: "option",
            "data-value": option.value,
            "aria-selected": "false",
          },
          h("span", { class: "dropdown-item-label" }, option.label),
          svg(icons.check, "icon dropdown-check"),
        ),
      ),
    );
    sync();
    if (isOpen) markActive(Math.min(active, options.length - 1));
  }

  function markActive(index: number) {
    active = index;
    items().forEach((item, i) => item.classList.toggle("is-active", i === index));
    const current = items()[index];
    if (current) {
      menu.setAttribute("aria-activedescendant", current.id);
      current.scrollIntoView?.({ block: "nearest" });
    } else {
      menu.removeAttribute("aria-activedescendant");
    }
  }

  /** Under the trigger, or over it when there is more room above. */
  function place() {
    const rect = trigger.getBoundingClientRect();
    const below = window.innerHeight - rect.bottom - EDGE - GAP;
    const above = rect.top - EDGE - GAP;
    const wanted = Math.min(menu.scrollHeight || MENU_MAX_HEIGHT, MENU_MAX_HEIGHT);
    const up = below < wanted && above > below;
    menu.style.minWidth = `${rect.width}px`;
    menu.style.maxHeight = `${Math.max(120, Math.min(MENU_MAX_HEIGHT, up ? above : below))}px`;
    const width = menu.offsetWidth;
    const left = Math.max(EDGE, Math.min(rect.left, window.innerWidth - width - EDGE));
    menu.style.left = `${left}px`;
    if (up) {
      menu.style.top = "auto";
      menu.style.bottom = `${window.innerHeight - rect.top + GAP}px`;
    } else {
      menu.style.bottom = "auto";
      menu.style.top = `${rect.bottom + GAP}px`;
    }
  }

  const onOutside = (event: Event) => {
    if (!el.contains(event.target as Node)) api.close();
  };
  const onScroll = (event: Event) => {
    if (!menu.contains(event.target as Node)) api.close();
  };
  const onResize = () => api.close();

  function open() {
    if (isOpen || options.length === 0) return;
    openDropdown?.close();
    openDropdown = api;
    isOpen = true;
    menu.hidden = false;
    el.classList.add("is-open");
    trigger.setAttribute("aria-expanded", "true");
    place();
    markActive(Math.max(0, options.findIndex((o) => o.value === value)));
    menu.focus({ preventScroll: true });
    document.addEventListener("pointerdown", onOutside, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
  }

  function close(refocus = false) {
    if (!isOpen) return;
    isOpen = false;
    menu.hidden = true;
    el.classList.remove("is-open");
    trigger.setAttribute("aria-expanded", "false");
    document.removeEventListener("pointerdown", onOutside, true);
    window.removeEventListener("scroll", onScroll, true);
    window.removeEventListener("resize", onResize);
    clearTimeout(typedTimer);
    typed = "";
    if (openDropdown === api) openDropdown = null;
    if (refocus) trigger.focus();
  }

  function choose(index: number) {
    const option = options[index];
    if (option && option.value !== value) {
      value = option.value;
      sync();
      config.onChange(value);
    }
    close(true);
  }

  function typeahead(key: string) {
    clearTimeout(typedTimer);
    typed += key.toLowerCase();
    typedTimer = setTimeout(() => (typed = ""), TYPEAHEAD_MS);
    const from = typed.length === 1 ? active + 1 : active;
    const order = [...options.keys()].map((_, i) => (from + i) % options.length);
    const hit = order.find((i) => options[i].label.toLowerCase().startsWith(typed));
    if (hit !== undefined) markActive(hit);
  }

  trigger.addEventListener("click", () => (isOpen ? close(true) : open()));
  trigger.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      open();
    }
  });

  menu.addEventListener("keydown", (e) => {
    const last = options.length - 1;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        markActive(Math.min(last, active + 1));
        break;
      case "ArrowUp":
        e.preventDefault();
        markActive(Math.max(0, active - 1));
        break;
      case "Home":
        e.preventDefault();
        markActive(0);
        break;
      case "End":
        e.preventDefault();
        markActive(last);
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        choose(active);
        break;
      case "Escape":
        // Only this list closes: a player or editor behind it stays open.
        e.preventDefault();
        e.stopPropagation();
        close(true);
        break;
      case "Tab":
        close();
        break;
      default:
        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) typeahead(e.key);
    }
  });
  menu.addEventListener("pointermove", (e) => {
    const item = (e.target as HTMLElement).closest<HTMLElement>(".dropdown-item");
    if (item) markActive(items().indexOf(item));
  });
  menu.addEventListener("click", (e) => {
    const item = (e.target as HTMLElement).closest<HTMLElement>(".dropdown-item");
    if (item) choose(items().indexOf(item));
  });

  const api: Dropdown = {
    el,
    trigger,
    setOptions(next) {
      options = [...next];
      renderItems();
      if (isOpen && options.length === 0) close();
    },
    setValue(next) {
      value = next;
      sync();
    },
    value: () => value,
    close: () => close(false),
  };

  renderItems();
  return api;
}
