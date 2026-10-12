import { formatBytes } from "./format";

const MB = 1024 * 1024; // the same unit `formatBytes` shows

/** Somewhere a clip is pasted, and the biggest file it takes. */
export interface ShareLimit {
  id: string;
  name: string;
  bytes: number;
}

/**
 * What the usual places accept. Limits **change**: Discord's come from its
 * "File Attachments FAQ" as read on 2026-10-12; the 25 MB of a mail is Gmail's
 * usual attachment limit and was not re-checked. Look them up again before
 * trusting a number here.
 */
export const SHARE_LIMITS: readonly ShareLimit[] = [
  { id: "discord", name: "Discord sin Nitro", bytes: 20 * MB },
  { id: "mail", name: "un correo (Gmail)", bytes: 25 * MB },
  { id: "discord-basic", name: "Discord con Nitro Basic", bytes: 50 * MB },
];

/** The places a file of `sizeBytes` doesn't fit, smallest limit first. */
export function limitsExceeded(sizeBytes: number): ShareLimit[] {
  return SHARE_LIMITS.filter((limit) => sizeBytes > limit.bytes).sort((a, b) => a.bytes - b.bytes);
}

/** `a`, `a ni b`, `a, b ni c`. */
function nor(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} ni ${items[items.length - 1]}`;
}

/** What to tell the person once the clip is on the clipboard. */
export function copiedMessage(sizeBytes: number): { text: string; warn: boolean } {
  const exceeded = limitsExceeded(sizeBytes);
  if (exceeded.length === 0) {
    return { text: "Copiado: pegalo en el chat o en un mail", warn: false };
  }
  const places = exceeded.map((l) => `${l.name} (${Math.round(l.bytes / MB)} MB)`);
  return {
    text: `Copiado, pero pesa ${formatBytes(sizeBytes)} y no entra en ${nor(places)}`,
    warn: true,
  };
}
