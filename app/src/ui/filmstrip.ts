import { h } from "./dom";

const TILE_WIDTH = 96;
const SEEK_TIMEOUT_MS = 2500;

/**
 * Fill `host` with `count` thumbnails evenly spread over the clip, taken by
 * seeking a second, hidden <video>. Purely decorative: any failure just
 * leaves empty tiles, and `isCancelled` stops the work when the editor
 * closes.
 */
export async function buildFilmstrip(
  src: string,
  duration: number,
  host: HTMLElement,
  count: number,
  isCancelled: () => boolean,
): Promise<void> {
  const video = h("video", { preload: "auto", playsinline: "" });
  video.muted = true;
  const tiles: HTMLCanvasElement[] = [];
  for (let i = 0; i < count; i++) {
    const tile = h("canvas", { class: "tl-tile" });
    tiles.push(tile);
    host.append(tile);
  }

  const once = (event: string) =>
    new Promise<void>((resolve) => {
      const done = () => {
        clearTimeout(timer);
        video.removeEventListener(event, done);
        resolve();
      };
      const timer = setTimeout(done, SEEK_TIMEOUT_MS);
      video.addEventListener(event, done);
    });

  try {
    video.src = src;
    await once("loadeddata");
    const aspect = video.videoWidth > 0 ? video.videoHeight / video.videoWidth : 9 / 16;
    for (let i = 0; i < count && !isCancelled(); i++) {
      const seeked = once("seeked");
      video.currentTime = ((i + 0.5) / count) * duration;
      await seeked;
      const tile = tiles[i];
      tile.width = TILE_WIDTH;
      tile.height = Math.max(Math.round(TILE_WIDTH * aspect), 1);
      tile.getContext("2d")?.drawImage(video, 0, 0, tile.width, tile.height);
    }
  } catch {
    // Thumbnails are optional.
  } finally {
    video.removeAttribute("src");
    video.load();
  }
}
