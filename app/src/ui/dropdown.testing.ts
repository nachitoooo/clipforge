// Helpers for the tests of the views that hold a dropdown (the filter bar and
// whoever shows one). Not part of the app: nothing imports this but tests.

/** The element of the dropdown named `label` (the wrapper that is shown or hidden). */
export const dropdownOf = (root: ParentNode, label: string) =>
  root.querySelector<HTMLElement>(`.dropdown[data-label="${label}"]`);

/** The texts of the options it offers. */
export const dropdownLabels = (dropdown: HTMLElement) =>
  [...dropdown.querySelectorAll(".dropdown-item-label")].map((el) => el.textContent);

/** The value it is on. */
export const dropdownValue = (dropdown: HTMLElement) => dropdown.dataset.value;

/** What a person does: open the list and click an option. */
export function chooseOption(dropdown: HTMLElement, value: string) {
  dropdown.querySelector<HTMLElement>(".dropdown-trigger")!.click();
  const option = dropdown.querySelector<HTMLElement>(`.dropdown-item[data-value="${value}"]`);
  if (!option) throw new Error(`no option "${value}" in ${dropdown.dataset.label}`);
  option.click();
}
