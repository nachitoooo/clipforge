/** Mirrors `recorder::BufferStatus` in `src-tauri/src/recorder.rs`. */
export interface BufferStatus {
  running: boolean;
  /** Why the buffer isn't running, when the last start attempt failed. */
  error: string | null;
}

/** Text for the status pill; `null` means the status hasn't loaded yet. */
export function bufferLabel(status: BufferStatus | null): string {
  if (status === null) return "Buffer…";
  if (status.running) return "Grabando buffer";
  return status.error ? "Sin captura" : "Buffer detenido";
}
