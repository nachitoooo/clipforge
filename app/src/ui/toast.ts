import { h } from "./dom";

export type ToastKind = "info" | "error";

export function toast(message: string, kind: ToastKind = "info", ms = 3200): void {
  const host = document.getElementById("toasts");
  if (!host) return;
  const el = h("div", { class: `toast toast-${kind}`, role: "status" }, message);
  host.append(el);
  setTimeout(() => {
    el.classList.add("toast-out");
    el.addEventListener("transitionend", () => el.remove(), { once: true });
    setTimeout(() => el.remove(), 400); // fallback if no transition fires
  }, ms);
}

/** Run an action that may fail; a failure becomes an error toast. True if it worked. */
export async function attempt(action: () => Promise<unknown>): Promise<boolean> {
  try {
    await action();
    return true;
  } catch (e) {
    toast(String(e), "error");
    return false;
  }
}
