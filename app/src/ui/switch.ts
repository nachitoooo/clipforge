import { h } from "./dom";

/** An accessible on/off switch; `input` is the real checkbox behind it. */
export function switchControl(label: string, onChange: (checked: boolean) => void) {
  const input = h("input", {
    type: "checkbox",
    role: "switch",
    class: "switch-input",
    "aria-label": label,
  });
  input.addEventListener("change", () => onChange(input.checked));
  const el = h("label", { class: "switch" }, input, h("span", { class: "switch-track" }));
  return { el, input };
}
