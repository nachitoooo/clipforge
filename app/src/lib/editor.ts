// The clip editor's state, without any DOM: the selection, the playhead and
// which audio tracks go into the copy. The view in `ui/editor.ts` only draws it.

import {
  clampRange,
  isWhole,
  minSelection,
  shiftRange,
  snapToFrame,
  withEnd,
  withStart,
  type Range,
} from "./timeline";

/** What `describe_clip` returns. */
export interface EditorClip {
  name: string;
  duration: number;
  width: number;
  height: number;
  fps: number;
  frames: number;
  audio: EditorAudioTrack[];
}

export interface EditorAudioTrack {
  index: number;
  channels: number;
  sampleRate: number;
  duration: number;
}

/** What `trim_clip` takes. */
export interface TrimRequest {
  name: string;
  start: number;
  end: number;
  keepAudio: number[];
}

export interface TrimResult {
  name: string;
  duration: number;
  bytes: number;
}

export interface TrackState {
  index: number;
  included: boolean;
}

/** What the player should do next while the clip is playing. */
export type PlaybackStep = { kind: "continue" } | { kind: "seek"; to: number } | { kind: "stop"; at: number };

/** Name shown next to an audio lane. */
export function trackLabel(index: number, total: number): string {
  if (total <= 1) return "Audio";
  if (index === 0) return "Juego";
  if (index === 1) return "Micrófono";
  return `Pista ${index + 1}`;
}

export class EditorModel {
  readonly duration: number;
  readonly fps: number;
  readonly minLength: number;
  range: Range;
  playhead = 0;
  tracks: TrackState[];

  constructor(clip: Pick<EditorClip, "duration" | "fps" | "audio">) {
    this.duration = Math.max(clip.duration, 0);
    this.fps = clip.fps > 0 ? clip.fps : 30;
    this.minLength = minSelection(this.fps);
    this.range = { start: 0, end: this.duration };
    this.tracks = clip.audio.map((a) => ({ index: a.index, included: true }));
  }

  get length(): number {
    return this.range.end - this.range.start;
  }

  // ---- selection ----------------------------------------------------------

  /** Move the in-point to `time` (snapped to a frame). */
  setStart(time: number): void {
    this.range = withStart(this.range, snapToFrame(time, this.fps), this.duration, this.minLength);
  }

  /** Move the out-point to `time` (snapped to a frame). */
  setEnd(time: number): void {
    this.range = withEnd(this.range, snapToFrame(time, this.fps), this.duration, this.minLength);
  }

  /** The "I" key: start the selection at the playhead. */
  markIn(): void {
    this.setStart(this.playhead);
  }

  /** The "O" key: end the selection at the playhead. */
  markOut(): void {
    this.setEnd(this.playhead);
  }

  /** Slide the whole selection. */
  shift(delta: number): void {
    this.range = shiftRange(this.range, delta, this.duration);
  }

  reset(): void {
    this.range = { start: 0, end: this.duration };
    for (const track of this.tracks) track.included = true;
  }

  // ---- playhead -----------------------------------------------------------

  seek(time: number): void {
    this.playhead = Math.min(Math.max(time, 0), this.duration);
  }

  stepFrames(frames: number): void {
    this.seek(snapToFrame(this.playhead, this.fps) + frames / this.fps);
  }

  /**
   * Decide what to do with a playing clip at `time`: it only plays inside the
   * selection, and either wraps around or stops at the end.
   */
  playbackStep(time: number, loop: boolean): PlaybackStep {
    const { start, end } = this.range;
    if (time < start - 0.05) return { kind: "seek", to: start };
    if (time >= end) return loop ? { kind: "seek", to: start } : { kind: "stop", at: end };
    return { kind: "continue" };
  }

  /** Where "play" should begin: from the playhead if it is inside the selection. */
  playFrom(): number {
    const { start, end } = this.range;
    return this.playhead >= start && this.playhead < end - 0.02 ? this.playhead : start;
  }

  // ---- audio tracks -------------------------------------------------------

  setIncluded(index: number, included: boolean): void {
    const track = this.tracks.find((t) => t.index === index);
    if (track) track.included = included;
  }

  isIncluded(index: number): boolean {
    return this.tracks.find((t) => t.index === index)?.included ?? false;
  }

  // ---- saving -------------------------------------------------------------

  /** Whether saving would produce something different from the original. */
  isDirty(): boolean {
    return !isWhole(this.range, this.duration) || this.tracks.some((t) => !t.included);
  }

  request(name: string): TrimRequest {
    const { start, end } = clampRange(this.range, this.duration, this.minLength);
    return {
      name,
      start,
      end,
      keepAudio: this.tracks.filter((t) => t.included).map((t) => t.index),
    };
  }
}
