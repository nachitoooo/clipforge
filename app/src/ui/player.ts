import { clipSrc } from "../api";
import { clipTitle, type ClipInfo } from "../lib/clips";
import { formatBytes } from "../lib/format";
import { h, svg } from "./dom";
import { icons } from "./icons";
import { openModal } from "./modal";

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
      closeButton,
    ),
    video,
  );
  const close = openModal(panel, () => {
    video.pause();
    video.removeAttribute("src");
    video.load(); // release the file handle
  });
}
