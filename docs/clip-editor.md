# Clip editor

Open a saved clip, cut it, and leave audio tracks out of the copy. The original
is never touched: saving writes `<name>-recorte.mp4` next to it.

## Using it

* **Open:** the scissors on a clip card in *Recientes*, or "Recortar" in the
  player.
* **Cut:** drag the orange handles, use the arrow keys on a focused handle
  (one frame; with Shift one second), press `I` / `O` to mark the start and
  end at the playhead, or type times in the boxes (`12.5`, `0:12.5`).
* **Play:** Space plays the selection (looping unless turned off with `L`);
  ← / → step a frame, Home / End jump to the ends of the selection.
* **Audio lanes:** when the clip has sound, each audio track appears under the
  video lane with its waveform and a switch. A track that is switched off is
  left out of the copy. A silent clip shows no audio lanes.
* **Save:** *Guardar recorte* (or ⌘/Ctrl+S). Closing with an unsaved cut asks
  first.

The preview plays the clip's first audio track only (that is what the webview
does with several tracks); the copy keeps every track left switched on.

## How a cut works (no ffmpeg, no re-encode)

`core/src/clip/trim.rs` reads the clip with the `mp4` crate and writes the copy
with the same muxer that writes clips from the replay buffer
(`clip/mp4mux.rs`, `write_mp4_cut`).

* **Video:** an H.264 stream can only start on a keyframe, and ours are two
  seconds apart. The copy starts at the keyframe *before* the requested start;
  those extra frames stay in the file as decode-only lead-in and an edit list
  (`elst`) on the video track makes playback begin at the requested frame. The
  start is frame accurate; the price is up to one GOP of extra data. The
  container durations (`mvhd`, video `tkhd`) are rewritten to the played
  length. The end can be any frame (no B-frames, so no reordering).
* **Audio:** AAC packets are copied as they are. Each track gets its own edit
  list from the muxer's `plan_audio`, which absorbs the encoder delay and the
  decoder warm-up and aligns the track to the new start. Timestamps handed to
  the muxer are *playback* times (the source's edit list already absorbed the
  encoder delay), so a copy can be cut again and again.
* **Waveform:** `audio_peaks` decodes a track with `symphonia` (pure Rust,
  AAC-LC only) and lays 50 peaks per second on the played timeline.
* **Limits:** only clips our own muxer writes are understood — H.264 without
  B-frames, AAC-LC mono/stereo, not fragmented. Anything else is refused with a
  clear error instead of being cut wrongly.

## Where things live

| Piece | File |
|---|---|
| probe / cut / waveform | `core/src/clip/trim.rs` |
| muxing a cut, video edit list | `core/src/clip/mp4mux.rs`, `core/src/clip/edit_list.rs` |
| Tauri commands `describe_clip`, `clip_waveform`, `trim_clip` | `app/src-tauri/src/lib.rs`, `app/src-tauri/src/editor.rs` |
| state and time math (pure) | `app/src/lib/editor.ts`, `timeline.ts`, `waveform.ts` |
| the view | `app/src/ui/editor.ts`, `filmstrip.ts` |

## Tests

* `core/tests/clip_trim.rs` — synthetic clips whose frames and packets carry
  their own index: which frame plays first, keyframe start, edit lists, bytes
  copied untouched, audio alignment (also after a second cut), dropping tracks,
  error cases; plus a real ffmpeg-made clip (`core/tests/fixtures/tone.mp4`,
  4 s, silence then a 440 Hz tone) for the waveform and a foreign encoder.
* `core/tests/macos_trim.rs` — the result as AVFoundation (QuickTime, Safari,
  the Tauri webview on macOS) plays it: frames decoded, first frame at 0, tones
  where the cut says; and a clip from the real pipeline (real AAC, a voice
  track that starts late).
* `app/src-tauri/src/editor.rs` — request validation, naming of copies, JSON.
* `app/src/lib/*.test.ts`, `app/src/ui/editor.test.ts` — the model and the view
  in a simulated DOM.

By hand, after changing the cut: `ffprobe -show_entries stream=start_time,duration`
on a copy, and `ffmpeg -i copy.mp4 -af silencedetect -f null -` to see where
the sound starts.
