import { captureAccelerator, displayAccelerator } from "../lib/hotkey";
import { h } from "./dom";

export interface ShortcutRecorderOptions {
  /** Which shortcut this is (`save-clip`, `mute-mic`): a stable hook for tests. */
  id: string;
  isMac: boolean;
  /** Tooltip of the button. */
  title: string;
  /** The accelerator to show on the button. */
  current(): string;
  /** Name of the other feature that already uses `accelerator`, if any. */
  conflict(accelerator: string): string | null;
  /** A new, free shortcut was recorded. */
  onChange(accelerator: string): void;
}

export interface ShortcutRecorder {
  /** Shows the shortcut; click it to record a new one. */
  button: HTMLButtonElement;
  /** Explains what to do, or why a key combination was refused. */
  note: HTMLElement;
  /** Refresh the button and the note from the current state. */
  render(): void;
  /** Leave recording mode (no-op when not recording). */
  stop(): void;
}

/** Only one shortcut is recorded at a time: starting one stops the other. */
let active: ShortcutRecorder | null = null;

/**
 * A button that, when clicked, listens for the next key combination and hands
 * it over as an accelerator. Esc cancels; combinations another feature uses
 * are refused with a message and recording continues.
 */
export function createShortcutRecorder(options: ShortcutRecorderOptions): ShortcutRecorder {
  let recording = false;
  let hint = "";

  const button = h("button", {
    class: "btn kbd-btn",
    "data-shortcut": options.id,
    title: options.title,
    onclick: () => (recording ? recorder.stop() : start()),
  });
  const note = h("div", {
    class: "row-hint shortcut-note",
    "data-shortcut-note": options.id,
  });

  function onKeyDown(e: KeyboardEvent): void {
    e.preventDefault();
    e.stopPropagation();
    const result = captureAccelerator(e, options.isMac);
    switch (result.kind) {
      case "waiting":
        return;
      case "cancel":
        recorder.stop();
        return;
      case "invalid":
        hint = result.reason;
        recorder.render();
        return;
      case "accelerator": {
        const owner = options.conflict(result.accelerator);
        if (owner) {
          hint = `Ese atajo ya lo usa «${owner}»`;
          recorder.render();
          return;
        }
        recorder.stop();
        options.onChange(result.accelerator);
      }
    }
  }

  function start(): void {
    if (active && active !== recorder) active.stop();
    active = recorder;
    recording = true;
    hint = "Presioná la combinación nueva (Esc para cancelar)";
    window.addEventListener("keydown", onKeyDown, true);
    button.addEventListener("blur", recorder.stop, { once: true });
    recorder.render();
  }

  const recorder: ShortcutRecorder = {
    button,
    note,
    render() {
      button.textContent = recording
        ? "Presioná la combinación…"
        : displayAccelerator(options.current(), options.isMac);
      button.classList.toggle("recording", recording);
      note.textContent = hint;
    },
    stop() {
      if (!recording) return;
      recording = false;
      hint = "";
      if (active === recorder) active = null;
      window.removeEventListener("keydown", onKeyDown, true);
      button.removeEventListener("blur", recorder.stop);
      recorder.render();
    },
  };
  return recorder;
}
