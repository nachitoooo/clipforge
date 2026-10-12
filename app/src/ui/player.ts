import { clipSrc } from "../api";
import { clipTitle, type ClipInfo } from "../lib/clips";
import { formatBytes } from "../lib/format";
import { h, svg } from "./dom";
import { icons } from "./icons";
import { openEditor } from "./editor";
import { openModal } from "./modal";
import { copyClip } from "./share";

/** Full-size playback of one clip. */
export function openPlayer(clip: ClipInfo): void {
  const video = h("video", { class: "player-video", src: clipSrc(clip), controls: true, autoplay: true });
  const closeButton = h(
    "button",
    { class: "icon-btn", title: "Cerrar", "aria-label": "Cerrar", onclick: () => close() },
    svg(icons.close),
  );
  const panel = h(
    "div",
    { class: "player", role: "dialog", "aria-modal": "true", "aria-label": clipTitle(clip.name) },
    h(
      "div",
      { class: "player-bar" },
      h(
        "div",
        { class: "player-title" },
        h("strong", {}, clipTitle(clip.name)),
        h("span", { class: "muted" }, formatBytes(clip.sizeBytes)),
      ),
      h(
        "div",
        { class: "player-actions" },
        h(
          "button",
          {
            class: "btn",
            title: "Copiar el clip para pegarlo en un chat o un mail",
            onclick: () => void copyClip(clip),
          },
          svg(icons.copy),
          "Copiar",
        ),
        h(
          "button",
          {
            class: "btn",
            title: "Recortar y editar este clip",
            onclick: () => {
              close();
              openEditor(clip);
            },
          },
          svg(icons.cut),
          "Recortar",
        ),
        closeButton,
      ),
    ),
    video,
  );
  const close = openModal(panel, () => {
    video.pause();
    video.removeAttribute("src");
    video.load(); // release the file handle
  });
}
