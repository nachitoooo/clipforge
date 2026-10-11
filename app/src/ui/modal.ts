import { h } from "./dom";

/** Show `content` over a dimmed backdrop. Esc or a backdrop click closes it. */
export function openModal(content: HTMLElement, onClose?: () => void): () => void {
  const previouslyFocused = document.activeElement as HTMLElement | null;
  const backdrop = h("div", { class: "modal-backdrop" }, content);

  function close() {
    document.removeEventListener("keydown", onKey, true);
    backdrop.remove();
    previouslyFocused?.focus?.();
    onClose?.();
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    }
  }

  backdrop.addEventListener("mousedown", (e) => {
    if (e.target === backdrop) close();
  });
  document.addEventListener("keydown", onKey, true);
  document.body.append(backdrop);
  return close;
}

/** Resolves true when the user confirms, false on cancel/Esc/backdrop. */
export function confirmDialog(options: {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
}): Promise<boolean> {
  return new Promise((resolve) => {
    let answered = false;
    const answer = (value: boolean) => {
      if (answered) return;
      answered = true;
      close();
      resolve(value);
    };

    const confirmButton = h(
      "button",
      { class: options.danger ? "btn btn-danger" : "btn btn-primary", onclick: () => answer(true) },
      options.confirmLabel,
    );
    const dialog = h(
      "div",
      { class: "dialog", role: "alertdialog", "aria-modal": "true" },
      h("h2", {}, options.title),
      h("p", {}, options.message),
      h(
        "div",
        { class: "dialog-actions" },
        h("button", { class: "btn", onclick: () => answer(false) }, "Cancelar"),
        confirmButton,
      ),
    );
    const close = openModal(dialog, () => answer(false));
    confirmButton.focus();
  });
}

export interface Choice<T extends string> {
  value: T;
  label: string;
  danger?: boolean;
}

/**
 * A dialog with more than one way forward. Resolves with the chosen value, or
 * null on cancel/Esc/backdrop. The first choice is the one focused, so put the
 * safe one first.
 */
export function choiceDialog<T extends string>(options: {
  title: string;
  message: string;
  choices: Choice<T>[];
}): Promise<T | null> {
  return new Promise((resolve) => {
    let answered = false;
    const answer = (value: T | null) => {
      if (answered) return;
      answered = true;
      close();
      resolve(value);
    };

    const buttons = options.choices.map((choice) =>
      h(
        "button",
        {
          class: choice.danger ? "btn btn-danger" : "btn btn-primary",
          "data-choice": choice.value,
          onclick: () => answer(choice.value),
        },
        choice.label,
      ),
    );
    const dialog = h(
      "div",
      { class: "dialog dialog-wide", role: "alertdialog", "aria-modal": "true" },
      h("h2", {}, options.title),
      h("p", {}, options.message),
      h(
        "div",
        { class: "dialog-actions" },
        h("button", { class: "btn", onclick: () => answer(null) }, "Cancelar"),
        ...buttons,
      ),
    );
    const close = openModal(dialog, () => answer(null));
    buttons[0]?.focus();
  });
}
