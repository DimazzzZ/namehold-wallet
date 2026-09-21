import { describe, it, expect } from "vitest";
import type { MouseEvent } from "react";
import { fromInteractiveChild } from "./rowClick";

/** A click whose target is the given markup's first element. */
function clickOn(html: string, pick = "*"): MouseEvent {
  const host = document.createElement("div");
  host.innerHTML = html;
  const target = pick === "*" ? host.firstElementChild : host.querySelector(pick);
  return { target } as unknown as MouseEvent;
}

describe("fromInteractiveChild", () => {
  it("claims the click for a control that handles it itself", () => {
    expect(fromInteractiveChild(clickOn("<button>Manage</button>"))).toBe(true);
    expect(fromInteractiveChild(clickOn('<input type="checkbox" />'))).toBe(true);
    expect(fromInteractiveChild(clickOn('<a href="#">link</a>'))).toBe(true);
    expect(fromInteractiveChild(clickOn("<select></select>"))).toBe(true);
    expect(fromInteractiveChild(clickOn('<div role="button">x</div>'))).toBe(true);
  });

  it("looks through the element the click actually landed on", () => {
    // A click on a button's inner text lands on the span, not the button —
    // which is exactly what happens with an icon or a wrapped label.
    expect(fromInteractiveChild(clickOn("<button><span>#100</span></button>", "span"))).toBe(true);
  });

  it("leaves an ordinary cell click to the row", () => {
    expect(fromInteractiveChild(clickOn("<td>plain text</td>"))).toBe(false);
    expect(fromInteractiveChild(clickOn("<span>label</span>"))).toBe(false);
  });

  it("treats a target-less event as the row's own", () => {
    expect(fromInteractiveChild({ target: null } as unknown as MouseEvent)).toBe(false);
  });
});
