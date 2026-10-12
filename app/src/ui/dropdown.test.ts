// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createDropdown, type Dropdown, type DropdownOption } from "./dropdown";

const OPTIONS: DropdownOption[] = [
  { value: "", label: "Todos los juegos" },
  { value: "valorant", label: "Valorant (2)" },
  { value: "fortnite", label: "Fortnite (1)" },
  { value: "__none__", label: "Sin juego (1)" },
];

let changes: string[];
let dd: Dropdown;

function make(value = "") {
  document.body.innerHTML = '<div id="host"></div>';
  changes = [];
  dd = createDropdown({ label: "Juego", value, options: OPTIONS, onChange: (v) => changes.push(v) });
  document.getElementById("host")!.append(dd.el);
}

beforeEach(() => make());
afterEach(() => {
  dd.close();
  vi.useRealTimers();
});

const menu = () => dd.el.querySelector<HTMLElement>(".dropdown-menu")!;
const items = () => [...dd.el.querySelectorAll<HTMLElement>(".dropdown-item")];
const labels = () => items().map((i) => i.querySelector(".dropdown-item-label")!.textContent);
const isOpen = () => !menu().hidden;
const activeLabel = () => items().find((i) => i.classList.contains("is-active"))?.textContent;
const key = (k: string, target: HTMLElement = menu(), extra: KeyboardEventInit = {}) => {
  const event = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...extra });
  target.dispatchEvent(event);
  return event;
};

describe("what it shows", () => {
  it("is a button naming the control and showing the choice, with the list closed", () => {
    expect(dd.trigger.getAttribute("aria-label")).toBe("Juego");
    expect(dd.trigger.textContent).toBe("Todos los juegos");
    expect(dd.trigger.getAttribute("aria-haspopup")).toBe("listbox");
    expect(dd.trigger.getAttribute("aria-expanded")).toBe("false");
    expect(isOpen()).toBe(false);
  });

  it("has an option per choice, in a listbox, with the chosen one marked", () => {
    expect(menu().getAttribute("role")).toBe("listbox");
    expect(labels()).toEqual(["Todos los juegos", "Valorant (2)", "Fortnite (1)", "Sin juego (1)"]);
    expect(items().every((i) => i.getAttribute("role") === "option")).toBe(true);
    expect(items().map((i) => i.getAttribute("aria-selected"))).toEqual(["true", "false", "false", "false"]);
  });

  it("starts on the value it was given", () => {
    make("fortnite");
    expect(dd.trigger.textContent).toBe("Fortnite (1)");
    expect(dd.value()).toBe("fortnite");
    expect(dd.el.dataset.value).toBe("fortnite");
  });

  it("shows the first option when the value isn't one of them", () => {
    dd.setValue("gone");
    expect(dd.trigger.textContent).toBe("Todos los juegos");
  });

  it("is empty, and won't open, when there is nothing to choose", () => {
    dd.setOptions([]);
    expect(dd.trigger.textContent).toBe("");
    dd.trigger.click();
    expect(isOpen()).toBe(false);
  });
});

describe("opening and closing", () => {
  it("opens on a click and says so", () => {
    dd.trigger.click();
    expect(isOpen()).toBe(true);
    expect(dd.trigger.getAttribute("aria-expanded")).toBe("true");
    expect(dd.el.classList.contains("is-open")).toBe(true);
  });

  it("puts the focus in the list and the cursor on the chosen option", () => {
    make("fortnite");
    dd.trigger.click();
    expect(document.activeElement).toBe(menu());
    expect(activeLabel()).toBe("Fortnite (1)");
    expect(menu().getAttribute("aria-activedescendant")).toBe(items()[2].id);
  });

  it("a second click on the trigger closes it and keeps the focus there", () => {
    dd.trigger.click();
    dd.trigger.click();
    expect(isOpen()).toBe(false);
    expect(document.activeElement).toBe(dd.trigger);
  });

  it("opens with the arrow keys from the trigger", () => {
    const down = key("ArrowDown", dd.trigger);
    expect(down.defaultPrevented).toBe(true);
    expect(isOpen()).toBe(true);
    dd.close();
    key("ArrowUp", dd.trigger);
    expect(isOpen()).toBe(true);
  });

  it("closes when the pointer goes down anywhere else, not inside", () => {
    dd.trigger.click();
    menu().dispatchEvent(new Event("pointerdown", { bubbles: true }));
    expect(isOpen()).toBe(true);
    document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    expect(isOpen()).toBe(false);
  });

  it("closes when the page scrolls, but not when the list does", () => {
    dd.trigger.click();
    menu().dispatchEvent(new Event("scroll"));
    expect(isOpen()).toBe(true);
    document.body.dispatchEvent(new Event("scroll"));
    expect(isOpen()).toBe(false);
  });

  it("closes when the window is resized", () => {
    dd.trigger.click();
    window.dispatchEvent(new Event("resize"));
    expect(isOpen()).toBe(false);
  });

  it("only one list is open at a time", () => {
    const other = createDropdown({ label: "Fecha", options: OPTIONS, onChange: () => {} });
    document.getElementById("host")!.append(other.el);
    dd.trigger.click();
    other.trigger.click();
    expect(isOpen()).toBe(false);
    expect(other.el.querySelector<HTMLElement>(".dropdown-menu")!.hidden).toBe(false);
    other.close();
  });

  it("stops listening for outside clicks once closed", () => {
    const remove = vi.spyOn(document, "removeEventListener");
    dd.trigger.click();
    dd.close();
    expect(remove).toHaveBeenCalledWith("pointerdown", expect.any(Function), true);
    remove.mockRestore();
  });
});

describe("choosing", () => {
  it("a click on an option picks it, tells the owner and closes", () => {
    dd.trigger.click();
    items()[1].click();
    expect(changes).toEqual(["valorant"]);
    expect(dd.trigger.textContent).toBe("Valorant (2)");
    expect(dd.value()).toBe("valorant");
    expect(isOpen()).toBe(false);
    expect(document.activeElement).toBe(dd.trigger);
  });

  it("marks the new choice", () => {
    dd.trigger.click();
    items()[2].click();
    expect(items().map((i) => i.getAttribute("aria-selected"))).toEqual(["false", "false", "true", "false"]);
    expect(items()[2].classList.contains("is-selected")).toBe(true);
  });

  it("picking what is already chosen changes nothing and tells no one", () => {
    dd.trigger.click();
    items()[0].click();
    expect(changes).toEqual([]);
    expect(isOpen()).toBe(false);
  });

  it("clicking the label inside an option works too", () => {
    dd.trigger.click();
    items()[3].querySelector<HTMLElement>(".dropdown-item-label")!.click();
    expect(changes).toEqual(["__none__"]);
  });

  it("setValue shows a choice without telling the owner", () => {
    dd.setValue("fortnite");
    expect(changes).toEqual([]);
    expect(dd.trigger.textContent).toBe("Fortnite (1)");
    expect(items()[2].getAttribute("aria-selected")).toBe("true");
  });

  it("new options keep the choice when it is still there, and show the first when it is gone", () => {
    dd.setValue("fortnite");
    dd.setOptions(OPTIONS.slice(1));
    expect(dd.trigger.textContent).toBe("Fortnite (1)");
    dd.setOptions([OPTIONS[0], OPTIONS[1]]);
    expect(dd.trigger.textContent).toBe("Todos los juegos");
  });
});

describe("the keyboard", () => {
  beforeEach(() => dd.trigger.click());

  it("the arrows move the cursor and stop at the ends", () => {
    key("ArrowDown");
    expect(activeLabel()).toBe("Valorant (2)");
    key("ArrowDown");
    key("ArrowDown");
    key("ArrowDown");
    expect(activeLabel()).toBe("Sin juego (1)");
    key("ArrowUp");
    expect(activeLabel()).toBe("Fortnite (1)");
    for (let i = 0; i < 6; i++) key("ArrowUp");
    expect(activeLabel()).toBe("Todos los juegos");
  });

  it("Home and End jump to the first and last option", () => {
    key("End");
    expect(activeLabel()).toBe("Sin juego (1)");
    key("Home");
    expect(activeLabel()).toBe("Todos los juegos");
  });

  it("Enter and Space choose the one under the cursor", () => {
    key("ArrowDown");
    key("Enter");
    expect(changes).toEqual(["valorant"]);
    expect(isOpen()).toBe(false);

    dd.trigger.click();
    key("ArrowDown");
    key(" ");
    expect(changes).toEqual(["valorant", "fortnite"]);
  });

  it("Escape closes only the list: nobody behind it hears it", () => {
    const heard = vi.fn();
    document.body.addEventListener("keydown", heard);
    const event = key("Escape");
    expect(isOpen()).toBe(false);
    expect(event.defaultPrevented).toBe(true);
    expect(heard).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(dd.trigger);
    expect(changes).toEqual([]);
    document.body.removeEventListener("keydown", heard);
  });

  it("Tab closes it and lets the focus move on", () => {
    const event = key("Tab");
    expect(isOpen()).toBe(false);
    expect(event.defaultPrevented).toBe(false);
  });

  it("the pointer moves the cursor too", () => {
    items()[2].dispatchEvent(new Event("pointermove", { bubbles: true }));
    expect(activeLabel()).toBe("Fortnite (1)");
  });

  it("typing a letter jumps to the option that starts with it", () => {
    key("f");
    expect(activeLabel()).toBe("Fortnite (1)");
  });

  it("letters typed together are one word: 'fs' matches nothing and stays put", () => {
    key("f");
    key("s");
    expect(activeLabel()).toBe("Fortnite (1)");
  });

  it("typing a longer word keeps narrowing, and the memory fades", () => {
    vi.useFakeTimers();
    key("s");
    key("i");
    expect(activeLabel()).toBe("Sin juego (1)");
    vi.advanceTimersByTime(800);
    key("v");
    expect(activeLabel()).toBe("Valorant (2)");
  });

  it("ignores shortcuts while typing", () => {
    key("f", menu(), { ctrlKey: true });
    expect(activeLabel()).toBe("Todos los juegos");
  });
});
