/**
 * Loads the clip thumbnails (paused `<video>` elements) as they come near the
 * screen and lets go of the ones that were left far behind.
 *
 * A loaded video keeps its decoder and a decoded frame in the web view's memory,
 * a few MB each (the first dozen cost ~44 MB in the real window), and a library of
 * a hundred clips scrolled once would hold all of them for as long as the window
 * is open. Here only the cards around the
 * scroller are loaded: they start loading a little before they are seen, and are
 * unloaded only once they are much further away, so a card at the edge does not
 * flip back and forth while scrolling.
 *
 * It looks at positions itself instead of using an IntersectionObserver: the
 * views build a new scroller on every render and keep their cards (and the
 * thumbnails already loaded) from one render to the next, and an observer's
 * margins only reach beyond the one scroller it was given.
 */

/** A card this close (px) to the scroller's edge is loaded. */
export const LOAD_MARGIN = 400;
/** A card further than this (px) from the scroller's edge is unloaded. */
export const KEEP_MARGIN = 1600;

/** The scroller of a view; cards outside any of them are measured against the window. */
const SCROLLER = ".view-body";

export type Verdict = "load" | "keep" | "unload";

interface Box {
  top: number;
  bottom: number;
}

/** How far (px) `card` is from the visible part of the scroller: 0 when any of it shows. */
export function distanceFrom(scroller: Box, card: Box): number {
  const above = scroller.top - card.bottom;
  const below = card.top - scroller.bottom;
  return Math.max(0, above, below);
}

/** Near enough: load. Far enough: unload. In between, leave it as it is. */
export function verdictFor(distance: number): Verdict {
  if (distance <= LOAD_MARGIN) return "load";
  if (distance > KEEP_MARGIN) return "unload";
  return "keep";
}

export interface LazyVideos {
  observe(video: HTMLVideoElement): void;
  unobserve(video: HTMLVideoElement): void;
  /** Look at every card now (it happens by itself on scroll, resize and when cards move). */
  check(): void;
  /** Stop listening. */
  dispose(): void;
}

/** Makes the video fetch its file (the address waits in `data-src`). */
export function loadVideo(video: HTMLVideoElement): void {
  if (video.hasAttribute("src")) return;
  const src = video.dataset.src;
  if (src) video.src = src;
}

/** Drops the file, the decoder and the frame; `data-src` keeps the way back. */
export function unloadVideo(video: HTMLVideoElement): void {
  if (!video.hasAttribute("src")) return;
  video.pause();
  video.removeAttribute("src");
  video.load?.(); // tells the engine to let go of the resource
}

export function createLazyVideos(): LazyVideos {
  const videos = new Set<HTMLVideoElement>();
  let frame = 0;

  function check(): void {
    frame = 0;
    const window_ = { top: 0, bottom: window.innerHeight };
    for (const video of videos) {
      const card = video.getBoundingClientRect();
      // No box at all: a view that is not showing. Leave it as it is.
      if (card.width === 0 && card.height === 0) continue;
      const scroller = video.closest(SCROLLER)?.getBoundingClientRect() ?? window_;
      const verdict = verdictFor(distanceFrom(scroller, card));
      if (verdict === "load") loadVideo(video);
      else if (verdict === "unload") unloadVideo(video);
    }
  }

  /** Once per frame, however many things happened. */
  function schedule(): void {
    if (frame === 0) frame = requestAnimationFrame(check);
  }

  // `scroll` does not bubble, but it can be caught on the way down.
  document.addEventListener("scroll", schedule, { capture: true, passive: true });
  window.addEventListener("resize", schedule);
  // Cards are moved into a fresh scroller on every render, and views are shown
  // and hidden with the `hidden` attribute.
  const moves = new MutationObserver(schedule);
  moves.observe(document.body, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["hidden"],
  });

  return {
    observe(video) {
      videos.add(video);
      schedule();
    },
    unobserve(video) {
      videos.delete(video);
    },
    check,
    dispose() {
      document.removeEventListener("scroll", schedule, { capture: true });
      window.removeEventListener("resize", schedule);
      moves.disconnect();
      if (frame !== 0) cancelAnimationFrame(frame);
      frame = 0;
    },
  };
}
