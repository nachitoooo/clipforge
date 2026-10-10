import { describe, expect, it } from "vitest";
import { EditorModel, trackLabel, type EditorClip } from "./editor";

const clip = (audioTracks = 2, duration = 30, fps = 60): EditorClip => ({
  name: "clip.mp4",
  duration,
  width: 1920,
  height: 1080,
  fps,
  frames: Math.round(duration * fps),
  audio: Array.from({ length: audioTracks }, (_, index) => ({
    index,
    channels: 2,
    sampleRate: 48000,
    duration,
  })),
});

describe("EditorModel selection", () => {
  it("starts with the whole clip selected and every track included", () => {
    const m = new EditorModel(clip());
    expect(m.range).toEqual({ start: 0, end: 30 });
    expect(m.tracks).toEqual([
      { index: 0, included: true },
      { index: 1, included: true },
    ]);
    expect(m.isDirty()).toBe(false);
  });

  it("moves the handles to frame boundaries", () => {
    const m = new EditorModel(clip());
    m.setStart(10.004);
    m.setEnd(20.5);
    expect(m.range.start).toBeCloseTo(10, 5);
    expect(m.range.end).toBe(20.5);
    expect(m.length).toBeCloseTo(10.5, 5);
  });

  it("keeps the handles from crossing", () => {
    const m = new EditorModel(clip());
    m.setStart(10);
    m.setEnd(20);
    m.setStart(25);
    expect(m.range.start).toBeCloseTo(20 - m.minLength, 3);
    m.setEnd(1);
    expect(m.range.end).toBeCloseTo(m.range.start + m.minLength, 3);
  });

  it("marks in and out at the playhead", () => {
    const m = new EditorModel(clip());
    m.seek(8);
    m.markIn();
    m.seek(14.5);
    m.markOut();
    expect(m.range).toEqual({ start: 8, end: 14.5 });
  });

  it("slides the selection and resets", () => {
    const m = new EditorModel(clip());
    m.setStart(5);
    m.setEnd(10);
    m.shift(3);
    expect(m.range).toEqual({ start: 8, end: 13 });
    m.setIncluded(1, false);
    m.reset();
    expect(m.range).toEqual({ start: 0, end: 30 });
    expect(m.isIncluded(1)).toBe(true);
    expect(m.isDirty()).toBe(false);
  });
});

describe("EditorModel playhead", () => {
  it("stays inside the clip", () => {
    const m = new EditorModel(clip());
    m.seek(-4);
    expect(m.playhead).toBe(0);
    m.seek(99);
    expect(m.playhead).toBe(30);
  });

  it("steps by frames from a frame boundary", () => {
    const m = new EditorModel(clip(1, 30, 60));
    m.seek(1);
    m.stepFrames(1);
    expect(m.playhead).toBeCloseTo(1 + 1 / 60, 6);
    m.stepFrames(-2);
    expect(m.playhead).toBeCloseTo(1 - 1 / 60, 6);
    m.seek(0);
    m.stepFrames(-1);
    expect(m.playhead).toBe(0);
  });

  it("falls back to 30 fps when the clip does not say", () => {
    const m = new EditorModel(clip(0, 10, 0));
    expect(m.fps).toBe(30);
  });
});

describe("EditorModel playback", () => {
  const model = () => {
    const m = new EditorModel(clip());
    m.setStart(10);
    m.setEnd(20);
    return m;
  };

  it("keeps playing inside the selection", () => {
    expect(model().playbackStep(15, false)).toEqual({ kind: "continue" });
    expect(model().playbackStep(10, true)).toEqual({ kind: "continue" });
  });

  it("jumps into the selection when started before it", () => {
    expect(model().playbackStep(3, false)).toEqual({ kind: "seek", to: 10 });
  });

  it("stops at the end, or wraps around when looping", () => {
    expect(model().playbackStep(20, false)).toEqual({ kind: "stop", at: 20 });
    expect(model().playbackStep(20.4, true)).toEqual({ kind: "seek", to: 10 });
  });

  it("plays from the playhead when it is inside, otherwise from the start", () => {
    const m = model();
    m.seek(14);
    expect(m.playFrom()).toBe(14);
    m.seek(2);
    expect(m.playFrom()).toBe(10);
    m.seek(19.99); // basically at the end: start over
    expect(m.playFrom()).toBe(10);
    m.seek(20);
    expect(m.playFrom()).toBe(10);
  });
});

describe("EditorModel saving", () => {
  it("is dirty once the selection or the tracks change", () => {
    const a = new EditorModel(clip());
    a.setStart(1);
    expect(a.isDirty()).toBe(true);

    const b = new EditorModel(clip());
    b.setEnd(29);
    expect(b.isDirty()).toBe(true);

    const c = new EditorModel(clip());
    c.setIncluded(1, false);
    expect(c.isDirty()).toBe(true);
    c.setIncluded(1, true);
    expect(c.isDirty()).toBe(false);
  });

  it("asks for the selection and only the tracks that are included", () => {
    const m = new EditorModel(clip());
    m.setStart(5);
    m.setEnd(12.5);
    m.setIncluded(0, false);
    expect(m.request("clip.mp4")).toEqual({
      name: "clip.mp4",
      start: 5,
      end: 12.5,
      keepAudio: [1],
    });
  });

  it("can ask for a silent copy", () => {
    const m = new EditorModel(clip());
    m.setIncluded(0, false);
    m.setIncluded(1, false);
    expect(m.request("x.mp4").keepAudio).toEqual([]);
  });

  it("ignores unknown tracks", () => {
    const m = new EditorModel(clip(1));
    m.setIncluded(7, false);
    expect(m.isIncluded(7)).toBe(false);
    expect(m.isDirty()).toBe(false);
  });

  it("works for a silent clip", () => {
    const m = new EditorModel(clip(0));
    expect(m.tracks).toEqual([]);
    m.setStart(2);
    expect(m.request("a.mp4").keepAudio).toEqual([]);
  });
});

describe("trackLabel", () => {
  it("names the lanes by what they usually hold", () => {
    expect(trackLabel(0, 1)).toBe("Audio");
    expect(trackLabel(0, 2)).toBe("Juego");
    expect(trackLabel(1, 2)).toBe("Micrófono");
    expect(trackLabel(2, 3)).toBe("Pista 3");
  });
});
