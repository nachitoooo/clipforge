import * as api from "./api";
import { h, svg } from "./ui/dom";
import { createGallery } from "./ui/gallery";
import { icons } from "./ui/icons";
import { libraryPlaceholder, renderPlaceholder } from "./ui/placeholder";
import { createSettings } from "./ui/settings";
import { toast } from "./ui/toast";

type ViewId = "recent" | "library" | "settings";

interface NavItem {
  id: ViewId;
  label: string;
  icon: string;
}

const topItems: NavItem[] = [
  { id: "recent", label: "Recientes", icon: icons.recent },
  { id: "library", label: "Biblioteca", icon: icons.library },
];
const bottomItems: NavItem[] = [{ id: "settings", label: "Configuración", icon: icons.settings }];

window.addEventListener("DOMContentLoaded", async () => {
  const sidebar = document.getElementById("sidebar")!;
  const content = document.getElementById("content")!;

  // One container per view, kept alive and toggled with `hidden`, so switching
  // views never reloads the gallery's thumbnails.
  const views: Record<ViewId, HTMLElement> = {
    recent: h("section", { class: "view" }),
    library: h("section", { class: "view", hidden: true }),
    settings: h("section", { class: "view", hidden: true }),
  };
  content.append(...Object.values(views));

  const gallery = createGallery(views.recent);
  renderPlaceholder(views.library, libraryPlaceholder);
  const settingsView = createSettings(views.settings);

  const buttons = new Map<ViewId, HTMLButtonElement>();
  function select(id: ViewId) {
    for (const [viewId, el] of Object.entries(views) as [ViewId, HTMLElement][]) {
      el.hidden = viewId !== id;
    }
    if (id === "settings") settingsView.onShow();
    else settingsView.onHide();
    for (const [navId, button] of buttons) {
      const active = navId === id;
      button.classList.toggle("active", active);
      if (active) button.setAttribute("aria-current", "page");
      else button.removeAttribute("aria-current");
    }
  }

  function navButton(item: NavItem): HTMLButtonElement {
    const button = h(
      "button",
      { class: "nav-btn", title: item.label, "aria-label": item.label, onclick: () => select(item.id) },
      svg(item.icon),
      h("span", { class: "nav-label" }, item.label),
    );
    buttons.set(item.id, button);
    return button;
  }

  sidebar.append(
    h("div", { class: "brand", title: "ClipForge" }, svg(icons.forge, "icon brand-icon")),
    h("div", { class: "nav-group" }, ...topItems.map(navButton)),
    h("div", { class: "nav-spacer" }),
    h("div", { class: "nav-group" }, ...bottomItems.map(navButton)),
  );
  select("recent");

  // Live updates from the Rust side (hotkey, tray and UI all end up here).
  // Listeners must be registered *before* asking for the current state: the
  // buffer starts on a background thread, and a change that lands between the
  // query and the registration would be lost, leaving the pill stuck.
  await Promise.all([
    api.onClipSaved((name) => {
      gallery.markFresh(name);
      void gallery.refresh();
    }),
    api.onBufferChanged((status) => gallery.setBufferStatus(status)),
    api.onSettingsChanged((settings) => gallery.setSaveShortcut(settings.shortcuts.saveClip)),
  ]);

  api
    .bufferStatus()
    .then((status) => gallery.setBufferStatus(status))
    .catch((e) => toast(String(e), "error"));
  api
    .getSettings()
    .then((settings) => gallery.setSaveShortcut(settings.shortcuts.saveClip))
    .catch((e) => toast(String(e), "error"));
  void gallery.refresh();
});
