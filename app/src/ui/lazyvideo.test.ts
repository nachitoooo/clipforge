// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  KEEP_MARGIN,
  LOAD_MARGIN,
  createLazyVideos,
  distanceFrom,
  loadVideo,
  unloadVideo,
  verdictFor,
  type LazyVideos,
} from "./lazyvideo";

const SCROLLER = { top: 100, bottom: 700 }; // a view body 600 px tall

describe("how far a card is from the scroller", () => {
  it("a card that shows, even in part, is at 0", () => {
    expect(distanceFrom(SCROLLER, { top: 200, bottom: 380 })).toBe(0);
    expect(distanceFrom(SCROLLER, { top: 20, bottom: 120 })).toBe(0); // a corner shows at the top
    expect(distanceFrom(SCROLLER, { top: 650, bottom: 830 })).toBe(0); // and at the bottom
  });

  it("is the gap to the nearest edge, above or below", () => {
    expect(distanceFrom(SCROLLER, { top: -300, bottom: -100 })).toBe(200); // 200 px above
    expect(distanceFrom(SCROLLER, { top: 1000, bottom: 1180 })).toBe(300); // 300 px below
  });
});

describe("what to do with a card at a distance", () => {
  it("loads what is on screen and what is near", () => {
    expect(verdictFor(0)).toBe("load");
    expect(verdictFor(LOAD_MARGIN)).toBe("load");
  });

  it("leaves alone what is between the two margins, so an edge card does not flip", () => {
    expect(LOAD_MARGIN).toBeLessThan(KEEP_MARGIN);
    expect(verdictFor(LOAD_MARGIN + 1)).toBe("keep");
    expect(verdictFor(KEEP_MARGIN)).toBe("keep");
  });

  it("unloads what was left far behind", () => {
    expect(verdictFor(KEEP_MARGIN + 1)).toBe("unload");
    expect(verdictFor(50_000)).toBe("unload");
  });
});

function thumb(src = "asset://clip.mp4"): HTMLVideoElement {
  const video = document.createElement("video");
  video.dataset.src = src;
  video.load = vi.fn();
  return video;
}

describe("loading and unloading one thumbnail", () => {
  it("loading gives the video its file, once", () => {
    const video = thumb();
    expect(video.hasAttribute("src")).toBe(false);
    loadVideo(video);
    expect(video.getAttribute("src")).toBe("asset://clip.mp4");
    video.src = "asset://changed.mp4";
    loadVideo(video);
    expect(video.getAttribute("src")).toBe("asset://changed.mp4");
  });

  it("a video with nowhere to load from stays empty", () => {
    const video = document.createElement("video");
    loadVideo(video);
    expect(video.hasAttribute("src")).toBe(false);
  });

  it("unloading takes the file away, tells the engine and keeps the way back", () => {
    const video = thumb();
    loadVideo(video);
    unloadVideo(video);
    expect(video.hasAttribute("src")).toBe(false);
    expect(video.load).toHaveBeenCalledTimes(1);
    expect(video.dataset.src).toBe("asset://clip.mp4");
    loadVideo(video);
    expect(video.getAttribute("src")).toBe("asset://clip.mp4");
  });

  it("unloading a video that was never loaded does nothing", () => {
    const video = thumb();
    unloadVideo(video);
    expect(video.load).not.toHaveBeenCalled();
  });
});

/** Gives an element the box the test says it has (happy-dom does no layout). */
function place(el: Element, box: { top: number; bottom: number; width?: number }): void {
  const width = box.width ?? 320;
  const height = box.bottom - box.top;
  el.getBoundingClientRect = () =>
    ({ top: box.top, bottom: box.bottom, left: 0, right: width, width, height, x: 0, y: box.top }) as DOMRect;
}

describe("following the scroll", () => {
  let lazy: LazyVideos;
  let body: HTMLElement;

  beforeEach(() => {
    document.body.innerHTML = '<section><div class="view-body"></div></section>';
    body = document.querySelector<HTMLElement>(".view-body")!;
    place(body, SCROLLER);
    // Frames the test runs by hand.
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    lazy = createLazyVideos();
  });

  afterEach(() => {
    lazy.dispose();
    vi.unstubAllGlobals();
  });

  /** A card inside the scroller, `top` px from the top of the page. */
  function card(top: number): HTMLVideoElement {
    const video = thumb(`asset://${top}.mp4`);
    place(video, { top, bottom: top + 180 });
    body.append(video);
    lazy.observe(video);
    return video;
  }

  it("loads the cards on screen and the ones just below", () => {
    const shown = card(150);
    const near = card(700 + LOAD_MARGIN - 10);
    const beyond = card(700 + LOAD_MARGIN + 50);
    lazy.check();
    expect(shown.hasAttribute("src")).toBe(true);
    expect(near.hasAttribute("src")).toBe(true);
    expect(beyond.hasAttribute("src")).toBe(false);
  });

  it("a card that scrolls away keeps its picture until it is far, then lets go of it", () => {
    const video = card(150);
    lazy.check();
    place(video, { top: -500, bottom: -320 }); // 420 px above: past the load margin
    lazy.check();
    expect(video.hasAttribute("src")).toBe(true);
    place(video, { top: -2500, bottom: -2320 }); // 2420 px above
    lazy.check();
    expect(video.hasAttribute("src")).toBe(false);
    expect(video.load).toHaveBeenCalled();
  });

  it("a card that comes back loads again", () => {
    const video = card(150);
    lazy.check();
    place(video, { top: -4000, bottom: -3820 });
    lazy.check();
    expect(video.hasAttribute("src")).toBe(false);
    place(video, { top: -300, bottom: -120 }); // 220 px above: near
    lazy.check();
    expect(video.getAttribute("src")).toBe("asset://150.mp4");
  });

  it("a card in a view that is not showing is left as it is", () => {
    const video = card(150);
    lazy.check();
    place(video, { top: 0, bottom: 0, width: 0 });
    lazy.check();
    expect(video.hasAttribute("src")).toBe(true);

    const waiting = card(150);
    place(waiting, { top: 0, bottom: 0, width: 0 });
    lazy.check();
    expect(waiting.hasAttribute("src")).toBe(false);
  });

  it("a card outside any scroller is measured against the window", () => {
    const loose = thumb("asset://loose.mp4");
    place(loose, { top: 10, bottom: 190 });
    document.body.append(loose);
    lazy.observe(loose);
    vi.stubGlobal("innerHeight", 800);
    lazy.check();
    expect(loose.hasAttribute("src")).toBe(true);
  });

  it("stops looking at a card that is gone", () => {
    const video = card(150);
    lazy.unobserve(video);
    lazy.check();
    expect(video.hasAttribute("src")).toBe(false);
  });

  it("asks for one look per frame, however many things happened", () => {
    const frames = vi.mocked(requestAnimationFrame);
    frames.mockClear();
    card(150);
    card(400);
    document.dispatchEvent(new Event("scroll"));
    window.dispatchEvent(new Event("resize"));
    expect(frames).toHaveBeenCalledTimes(1);
  });

  it("looks again when the page scrolls, is resized or a view is shown", async () => {
    const frames = vi.mocked(requestAnimationFrame);
    // Each frame ends when its callback runs.
    frames.mockImplementation((callback: FrameRequestCallback) => {
      queueMicrotask(() => callback(0));
      return 1;
    });
    const video = card(150);
    await Promise.resolve();
    place(video, { top: -4000, bottom: -3820 });
    body.dispatchEvent(new Event("scroll")); // does not bubble: it is caught on the way down
    await Promise.resolve();
    await Promise.resolve();
    expect(video.hasAttribute("src")).toBe(false);

    place(video, { top: 200, bottom: 380 });
    window.dispatchEvent(new Event("resize"));
    await Promise.resolve();
    await Promise.resolve();
    expect(video.hasAttribute("src")).toBe(true);

    // A view that was hidden when its cards were made, shown later.
    const hiddenView = document.createElement("section");
    hiddenView.hidden = true;
    const late = thumb("asset://late.mp4");
    place(late, { top: 0, bottom: 0, width: 0 });
    hiddenView.append(late);
    document.body.append(hiddenView);
    lazy.observe(late);
    await Promise.resolve();
    place(late, { top: 100, bottom: 280 });
    hiddenView.hidden = false;
    await new Promise((r) => setTimeout(r, 0));
    await Promise.resolve();
    expect(late.hasAttribute("src")).toBe(true);
  });
});
