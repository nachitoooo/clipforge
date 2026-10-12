import * as api from "../api";
import type { ClipInfo } from "../lib/clips";
import { copiedMessage } from "../lib/share";
import { attempt, toast } from "./toast";

/**
 * Put the clip's file on the clipboard and say so; when the clip is bigger than
 * what a chat or a mail takes, say that too (it is a warning, not a failure:
 * the file is copied all the same).
 */
export async function copyClip(clip: ClipInfo): Promise<void> {
  if (!(await attempt(() => api.copyClip(clip.name)))) return;
  const { text, warn } = copiedMessage(clip.sizeBytes);
  toast(text, "info", warn ? 7000 : 3200);
}
