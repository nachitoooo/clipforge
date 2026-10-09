/**
 * Client-side mirror of `library::sanitize_stem` (src-tauri/src/library.rs).
 * Rust stays authoritative; this only gives instant feedback while typing.
 */
export const MAX_CLIP_NAME_LENGTH = 120;

const FORBIDDEN = /[\/\\:*?"<>|]/;
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

export type ClipNameCheck =
  | { ok: true; stem: string }
  | { ok: false; error: string };

export function validateClipName(input: string): ClipNameCheck {
  const trimmed = input.trim();
  const stem = trimmed.replace(/\.mp4$/i, "").trim();

  if (stem === "") return { ok: false, error: "El nombre no puede estar vacío" };
  if ([...stem].length > MAX_CLIP_NAME_LENGTH) {
    return {
      ok: false,
      error: `El nombre no puede superar ${MAX_CLIP_NAME_LENGTH} caracteres`,
    };
  }
  if (stem.startsWith(".")) {
    return { ok: false, error: "El nombre no puede empezar con un punto" };
  }
  if (FORBIDDEN.test(stem) || CONTROL.test(stem)) {
    return { ok: false, error: "El nombre contiene caracteres no permitidos" };
  }
  return { ok: true, stem };
}
