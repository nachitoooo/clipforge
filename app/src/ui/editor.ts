import * as api from "../api";
import { clipTitle, type ClipInfo } from "../lib/clips";
import { EditorModel, trackLabel, type EditorClip } from "../lib/editor";
import { formatBytes } from "../lib/format";
import {
  formatTimecode,
  fractionToTime,
  parseTimecode,
  rulerTicks,
  timeToPercent,
} from "../lib/timeline";
import { columns, waveformPath, type Waveform } from "../lib/waveform";
import { h, svg } from "./dom";
import { buildFilmstrip } from "./filmstrip";
import { icons } from "./icons";
import { confirmDialog } from "./modal";
import { switchControl } from "./switch";
import { toast } from "./toast";

export interface EditorOptions {
  /** The editor went away (saved, discarded or closed). */
  onClosed?: () => void;
}

/** Columns the waveform is drawn with; the SVG stretches them to the lane. */
const WAVE_COLUMNS = 600;
const FILMSTRIP_TILE_PX = 100;

type WaveState = { kind: "loading" } | { kind: "ready"; wave: Waveform } | { kind: "failed" };

const isTypingTarget = (target: EventTarget | null) =>
  target instanceof HTMLInputElement ||
  target instanceof HTMLSelectElement ||
  target instanceof HTMLTextAreaElement;

/**
 * The clip editor: a full-window view with the video, a timeline with handles
 * to cut the clip, and — when the clip has sound — one lane per audio track
 * under the video lane. Cutting never touches the original: saving writes a
 * copy next to it.
 */
export function openEditor(clip: ClipInfo, options: EditorOptions = {}): () => void {
  const previouslyFocused = document.activeElement as HTMLElement | null;
  const root = h("div", {
    class: "editor",
    role: "dialog",
    "aria-modal": "true",
    "aria-label": `Editar ${clipTitle(clip.name)}`,
    tabindex: "-1",
  });
  let closed = false;
  let teardown: (() => void) | null = null;

  function close(): void {
    if (closed) return;
    closed = true;
    teardown?.();
    root.remove();
    previouslyFocused?.focus?.();
    options.onClosed?.();
  }

  root.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !e.defaultPrevented) {
      e.preventDefault();
      void requestClose();
    }
  });

  /** Close, asking first when there is a cut nobody saved. */
  let requestClose: () => Promise<void> = async () => close();

  root.append(
    h(
      "header",
      { class: "editor-bar" },
      h(
        "button",
        { class: "btn", onclick: () => void requestClose() },
        svg(icons.back),
        "Volver",
      ),
      h("strong", { class: "editor-name" }, clipTitle(clip.name)),
    ),
    h("div", { class: "editor-loading muted" }, "Abriendo el clip…"),
  );
  document.body.append(root);
  root.focus();

  api
    .describeClip(clip.name)
    .then((info) => {
      if (closed) return;
      const view = buildView(clip, info, root, close);
      requestClose = view.requestClose;
      teardown = view.teardown;
    })
    .catch((e) => {
      if (closed) return;
      root.querySelector(".editor-loading")?.replaceChildren(
        h("p", {}, `No se pudo abrir el clip para editarlo: ${String(e)}`),
      );
    });

  return close;
}

interface View {
  requestClose(): Promise<void>;
  teardown(): void;
}

function buildView(clip: ClipInfo, info: EditorClip, root: HTMLElement, close: () => void): View {
  const model = new EditorModel(info);
  let loop = true;
  let saving = false;
  let dragging: "start" | "end" | "playhead" | null = null;
  let frame = 0;
  let released = false;
  const waves = new Map<number, WaveState>();

  // ---- the video ----------------------------------------------------------

  const video = h("video", {
    class: "editor-video",
    src: api.clipSrc(clip),
    preload: "auto",
    playsinline: "",
  });
  video.addEventListener("loadedmetadata", syncMute);

  function seekVideo(time: number): void {
    model.seek(time);
    try {
      video.currentTime = model.playhead;
    } catch {
      // Not ready yet: the playhead still moves.
    }
    paintPlayhead();
  }

  /** The preview plays the first audio track: silence it when it is left out. */
  function syncMute(): void {
    video.muted = info.audio.length > 0 && !model.isIncluded(info.audio[0].index);
  }

  function playing(): boolean {
    return !video.paused && !video.ended;
  }

  async function togglePlay(): Promise<void> {
    if (playing()) {
      video.pause();
      return;
    }
    seekVideo(model.playFrom());
    try {
      await video.play();
    } catch {
      // Autoplay rules or a decode error: stay paused.
    }
  }

  /** While playing: follow the clock, stay inside the selection. */
  function tick(): void {
    frame = 0;
    if (released) return;
    if (playing()) {
      model.seek(video.currentTime);
      const step = model.playbackStep(video.currentTime, loop);
      if (step.kind === "seek") {
        seekVideo(step.to);
      } else if (step.kind === "stop") {
        video.pause();
        seekVideo(step.at);
      }
      paintPlayhead();
      frame = requestAnimationFrame(tick);
    }
  }

  video.addEventListener("play", () => {
    paintPlayButton();
    if (!frame) frame = requestAnimationFrame(tick);
  });
  video.addEventListener("pause", paintPlayButton);
  video.addEventListener("ended", () => {
    paintPlayButton();
    // A selection that runs to the very end never reaches `tick`'s "past the
    // end" check: the video stops by itself, so wrap around here.
    if (loop && !released && !dragging && model.range.end >= model.duration - 0.05) {
      seekVideo(model.range.start);
      void video.play().catch(() => {});
    }
  });
  video.addEventListener("seeked", () => {
    if (!playing() && dragging === null) {
      model.seek(video.currentTime);
      paintPlayhead();
    }
  });

  // ---- header -------------------------------------------------------------

  const saveButton = h(
    "button",
    { class: "btn btn-primary", onclick: () => void save() },
    svg(icons.cut),
    "Guardar recorte",
  );
  const bar = h(
    "header",
    { class: "editor-bar" },
    h("button", { class: "btn", onclick: () => void requestClose() }, svg(icons.back), "Volver"),
    h(
      "div",
      { class: "editor-title" },
      h("strong", { class: "editor-name", title: clip.name }, clipTitle(clip.name)),
      h(
        "span",
        { class: "muted" },
        `${formatTimecode(info.duration, 1)} · ${info.width}×${info.height} · ${Math.round(info.fps)} fps · ${formatBytes(clip.sizeBytes)}`,
      ),
    ),
    h("div", { class: "editor-actions" }, saveButton),
  );

  // ---- transport ----------------------------------------------------------

  const playButton = h("button", {
    class: "btn btn-round",
    title: "Reproducir (espacio)",
    "aria-label": "Reproducir",
    onclick: () => void togglePlay(),
  });
  const readout = h("span", { class: "editor-time" });
  const loopButton = h(
    "button",
    {
      class: "btn btn-toggle",
      title: "Repetir la selección (L)",
      "aria-pressed": "true",
      onclick: () => {
        loop = !loop;
        paintLoop();
      },
    },
    svg(icons.loop),
    "Repetir",
  );
  const stepButton = (label: string, icon: string, frames: number) =>
    h(
      "button",
      {
        class: "icon-btn icon-btn-flat",
        title: label,
        "aria-label": label,
        onclick: () => stepFrames(frames),
      },
      svg(icon),
    );
  const transport = h(
    "div",
    { class: "editor-transport" },
    stepButton("Fotograma anterior (←)", icons.stepBack, -1),
    playButton,
    stepButton("Fotograma siguiente (→)", icons.stepForward, 1),
    readout,
    h("span", { class: "grow" }),
    loopButton,
  );

  function stepFrames(frames: number): void {
    video.pause();
    model.stepFrames(frames);
    seekVideo(model.playhead);
  }

  function paintPlayButton(): void {
    const isPlaying = playing();
    playButton.replaceChildren(svg(isPlaying ? icons.pause : icons.play));
    playButton.setAttribute("aria-label", isPlaying ? "Pausar" : "Reproducir");
    playButton.title = isPlaying ? "Pausar (espacio)" : "Reproducir (espacio)";
  }

  function paintLoop(): void {
    loopButton.setAttribute("aria-pressed", String(loop));
    loopButton.classList.toggle("on", loop);
  }

  // ---- timeline -----------------------------------------------------------

  const ruler = h("div", { class: "tl-ruler" });
  const videoTrack = h("div", { class: "tl-track tl-video-track" });
  const shadeStart = h("div", { class: "tl-shade tl-shade-start" });
  const shadeEnd = h("div", { class: "tl-shade tl-shade-end" });
  const handleStart = handle("start", "Inicio del recorte");
  const handleEnd = handle("end", "Final del recorte");
  const playhead = h("div", { class: "tl-playhead" }, h("div", { class: "tl-playhead-cap" }));
  const overlay = h(
    "div",
    { class: "tl-overlay" },
    shadeStart,
    shadeEnd,
    handleStart,
    handleEnd,
    playhead,
  );

  const lanes = h(
    "div",
    { class: "tl-rows" },
    h("div", { class: "tl-row tl-row-ruler" }, h("div", { class: "tl-label" }), ruler),
    h(
      "div",
      { class: "tl-row tl-row-video" },
      h("div", { class: "tl-label" }, svg(icons.film, "icon icon-sm"), "Video"),
      videoTrack,
    ),
  );

  const laneTracks = new Map<number, { track: HTMLElement; lane: HTMLElement; note: HTMLElement }>();
  for (const audio of info.audio) {
    const label = trackLabel(audio.index, info.audio.length);
    const toggle = switchControl(`Incluir en el recorte: ${label}`, (checked) => {
      model.setIncluded(audio.index, checked);
      if (audio.index === info.audio[0].index) syncMute();
      paintLaneState(audio.index);
      paintSave();
    });
    toggle.input.checked = true;
    const note = h("div", { class: "tl-note" });
    const track = h("div", { class: "tl-track tl-audio-track" }, note);
    const lane = h(
      "div",
      { class: "tl-row tl-row-audio", "data-track": String(audio.index) },
      h(
        "div",
        { class: "tl-label" },
        svg(audio.index === 1 && info.audio.length > 1 ? icons.mic : icons.speaker, "icon icon-sm"),
        h("span", { class: "tl-label-text" }, label),
        toggle.el,
      ),
      track,
    );
    laneTracks.set(audio.index, { track, lane, note });
    lanes.append(lane);
    waves.set(audio.index, { kind: "loading" });
    paintLaneState(audio.index);
  }

  const timeline = h("div", { class: "timeline" }, lanes, overlay);

  function handle(side: "start" | "end", label: string): HTMLElement {
    const el = h("div", {
      class: `tl-handle tl-handle-${side}`,
      role: "slider",
      tabindex: "0",
      "aria-label": label,
      "aria-orientation": "horizontal",
    });
    el.addEventListener("pointerdown", (e) => beginDrag(e, side));
    el.addEventListener("keydown", (e) => {
      const direction = e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
      if (direction === 0) return;
      e.preventDefault();
      e.stopPropagation();
      const delta = (e.shiftKey ? 1 : 1 / model.fps) * direction;
      if (side === "start") model.setStart(model.range.start + delta);
      else model.setEnd(model.range.end + delta);
      previewHandle(side);
      paintSelection();
    });
    return el;
  }

  function trackRect(): DOMRect {
    return overlay.getBoundingClientRect();
  }

  function timeAt(clientX: number): number {
    const rect = trackRect();
    return fractionToTime(rect.width > 0 ? (clientX - rect.left) / rect.width : 0, model.duration);
  }

  /** Show the frame a handle is pointing at. */
  function previewHandle(side: "start" | "end"): void {
    video.pause();
    seekVideo(side === "start" ? model.range.start : Math.max(model.range.end - 1 / model.fps, 0));
  }

  function beginDrag(e: PointerEvent, what: "start" | "end" | "playhead"): void {
    if (e.button > 0) return;
    e.preventDefault();
    e.stopPropagation();
    dragging = what;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture?.(e.pointerId);
    if (what === "playhead") video.pause();
    const move = (ev: PointerEvent) => {
      const time = timeAt(ev.clientX);
      if (what === "start") {
        model.setStart(time);
        previewHandle("start");
        paintSelection();
      } else if (what === "end") {
        model.setEnd(time);
        previewHandle("end");
        paintSelection();
      } else {
        seekVideo(time);
      }
    };
    const end = () => {
      dragging = null;
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", end);
      target.removeEventListener("pointercancel", end);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", end);
    target.addEventListener("pointercancel", end);
    // A handle keeps its place until dragged; the playhead jumps to the click.
    if (what === "playhead") move(e);
  }

  // Pressing anywhere else on the timeline moves the playhead there.
  overlay.addEventListener("pointerdown", (e) => beginDrag(e, "playhead"));

  // ---- footer -------------------------------------------------------------

  const startInput = h("input", {
    class: "tc-input",
    "aria-label": "Inicio del recorte (minutos:segundos)",
    spellcheck: "false",
  });
  const endInput = h("input", {
    class: "tc-input",
    "aria-label": "Final del recorte (minutos:segundos)",
    spellcheck: "false",
  });
  const lengthLabel = h("span", { class: "editor-length" });
  const fieldHandler = (input: HTMLInputElement, side: "start" | "end") => () => {
    const seconds = parseTimecode(input.value);
    if (seconds === null) {
      toast("Escribí un tiempo como 12.5 o 0:12.5", "error");
    } else if (side === "start") {
      model.setStart(seconds);
      previewHandle("start");
    } else {
      model.setEnd(seconds);
      previewHandle("end");
    }
    paintSelection();
  };
  startInput.addEventListener("change", fieldHandler(startInput, "start"));
  endInput.addEventListener("change", fieldHandler(endInput, "end"));

  const footer = h(
    "footer",
    { class: "editor-footer" },
    h("label", { class: "tc-field" }, "Inicio", startInput),
    h("label", { class: "tc-field" }, "Final", endInput),
    lengthLabel,
    h("span", { class: "grow" }),
    h(
      "button",
      {
        class: "btn",
        onclick: () => {
          model.reset();
          for (const audio of info.audio) {
            const input = laneTracks
              .get(audio.index)
              ?.lane.querySelector<HTMLInputElement>(".switch-input");
            if (input) input.checked = true;
            paintLaneState(audio.index);
          }
          syncMute();
          paintSelection();
        },
      },
      "Restablecer",
    ),
  );

  const hint = h(
    "p",
    { class: "editor-hint muted" },
    "Arrastrá los extremos naranjas para recortar. Espacio reproduce · I y O marcan inicio y final · ← → avanzan de a un fotograma.",
    info.audio.length > 1
      ? " La vista previa solo reproduce la pista de Juego; el recorte guarda las pistas que dejes activadas."
      : null,
  );

  root.replaceChildren(
    bar,
    h("div", { class: "editor-stage" }, video),
    transport,
    h("div", { class: "editor-timeline" }, timeline),
    hint,
    footer,
  );

  // ---- painting -----------------------------------------------------------

  function paintSelection(): void {
    const { start, end } = model.range;
    const a = timeToPercent(start, model.duration);
    const b = timeToPercent(end, model.duration);
    shadeStart.style.width = `${a}%`;
    shadeEnd.style.width = `${100 - b}%`;
    handleStart.style.left = `${a}%`;
    handleEnd.style.left = `${b}%`;
    handleStart.setAttribute("aria-valuenow", start.toFixed(2));
    handleStart.setAttribute("aria-valuetext", formatTimecode(start));
    handleEnd.setAttribute("aria-valuenow", end.toFixed(2));
    handleEnd.setAttribute("aria-valuetext", formatTimecode(end));
    for (const el of [handleStart, handleEnd]) {
      el.setAttribute("aria-valuemin", "0");
      el.setAttribute("aria-valuemax", model.duration.toFixed(2));
    }
    // Don't overwrite a box the person is typing in.
    if (document.activeElement !== startInput) startInput.value = formatTimecode(start);
    if (document.activeElement !== endInput) endInput.value = formatTimecode(end);
    lengthLabel.textContent = `Dura ${formatTimecode(model.length)}`;
    paintSave();
  }

  function paintPlayhead(): void {
    playhead.style.left = `${timeToPercent(model.playhead, model.duration)}%`;
    readout.textContent = `${formatTimecode(model.playhead)} / ${formatTimecode(model.duration)}`;
  }

  function paintSave(): void {
    const ready = model.isDirty() && !saving;
    saveButton.disabled = !ready;
    saveButton.title = saving
      ? "Guardando…"
      : model.isDirty()
        ? "Guardar el recorte como un clip nuevo (el original no se toca)"
        : "Mové los extremos o desactivá una pista para poder guardar";
    saveButton.lastChild!.textContent = saving ? "Guardando…" : "Guardar recorte";
  }

  function paintLaneState(index: number): void {
    const entry = laneTracks.get(index);
    if (!entry) return;
    const included = model.isIncluded(index);
    entry.lane.classList.toggle("tl-off", !included);
    const state = waves.get(index);
    if (!included) entry.note.textContent = "No se incluye en el recorte";
    else if (state?.kind === "loading") entry.note.textContent = "Dibujando la onda…";
    else if (state?.kind === "failed") entry.note.textContent = "No se pudo dibujar la onda";
    else entry.note.textContent = "";
  }

  function paintWave(index: number): void {
    const entry = laneTracks.get(index);
    const state = waves.get(index);
    if (!entry || state?.kind !== "ready") return;
    const heights = columns(state.wave, info.duration, WAVE_COLUMNS);
    const wave = svg(
      `<svg viewBox="0 0 ${heights.length} 100" preserveAspectRatio="none"><path d="${waveformPath(heights)}"/></svg>`,
      "tl-wave",
    );
    entry.track.querySelector(".tl-wave")?.remove();
    entry.track.prepend(wave);
    paintLaneState(index);
  }

  function paintRuler(): void {
    const width = trackRect().width || 800;
    ruler.replaceChildren(
      ...rulerTicks(model.duration, width).map((tick) =>
        h(
          "span",
          { class: "tl-tick", style: `left:${timeToPercent(tick.time, model.duration)}%` },
          tick.label,
        ),
      ),
    );
  }

  // ---- actions ------------------------------------------------------------

  async function save(): Promise<void> {
    if (saving || !model.isDirty()) return;
    saving = true;
    video.pause();
    paintSave();
    try {
      const result = await api.trimClip(model.request(clip.name));
      toast(`Recorte guardado: ${clipTitle(result.name)}`);
      saving = false;
      close();
    } catch (e) {
      saving = false;
      toast(String(e), "error");
      paintSave();
    }
  }

  async function requestClose(): Promise<void> {
    if (saving) return;
    if (model.isDirty()) {
      const discard = await confirmDialog({
        title: "¿Descartar el recorte?",
        message: "Moviste los extremos o quitaste una pista y todavía no lo guardaste.",
        confirmLabel: "Descartar",
        danger: true,
      });
      if (!discard) return;
    }
    close();
  }

  // ---- keyboard -----------------------------------------------------------

  function onKey(e: KeyboardEvent): void {
    if (e.defaultPrevented || e.altKey) return;
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      void save();
      return;
    }
    if (e.metaKey || e.ctrlKey || isTypingTarget(e.target)) return;
    const onButton = e.target instanceof HTMLButtonElement;
    switch (e.key) {
      case " ":
        if (onButton) return; // the button handles its own press
        e.preventDefault();
        void togglePlay();
        break;
      case "ArrowLeft":
      case "ArrowRight": {
        e.preventDefault();
        video.pause();
        const direction = e.key === "ArrowLeft" ? -1 : 1;
        if (e.shiftKey) seekVideo(model.playhead + direction);
        else stepFrames(direction);
        break;
      }
      case "i":
      case "I":
        model.markIn();
        paintSelection();
        break;
      case "o":
      case "O":
        model.markOut();
        paintSelection();
        break;
      case "l":
      case "L":
        loop = !loop;
        paintLoop();
        break;
      case "Home":
        e.preventDefault();
        seekVideo(model.range.start);
        break;
      case "End":
        e.preventDefault();
        seekVideo(model.range.end);
        break;
      default:
    }
  }
  root.addEventListener("keydown", onKey);

  const resizeObserver =
    typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => paintRuler());
  resizeObserver?.observe(overlay);

  // ---- start --------------------------------------------------------------

  syncMute();
  paintLoop();
  paintPlayButton();
  paintSelection();
  paintPlayhead();
  paintRuler();

  const tiles = Math.min(Math.max(Math.ceil((trackRect().width || 900) / FILMSTRIP_TILE_PX), 6), 24);
  void buildFilmstrip(api.clipSrc(clip), info.duration, videoTrack, tiles, () => released);
  for (const audio of info.audio) {
    api
      .clipWaveform(clip.name, audio.index)
      .then((wave) => {
        if (released) return;
        waves.set(audio.index, { kind: "ready", wave });
        paintWave(audio.index);
      })
      .catch(() => {
        if (released) return;
        waves.set(audio.index, { kind: "failed" });
        paintLaneState(audio.index);
      });
  }

  return {
    requestClose,
    teardown() {
      released = true;
      cancelAnimationFrame(frame);
      resizeObserver?.disconnect();
      video.pause();
      video.removeAttribute("src");
      video.load(); // release the file handle
    },
  };
}
