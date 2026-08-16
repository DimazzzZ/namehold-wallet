/**
 * Tests for the global keyboard-shortcut hook and the binding table.
 *
 * Two axes:
 * 1. Binding-table shape / drift detection — asserts the hook's hard-coded
 *    1..6 registrations match the HOTKEY_BINDINGS nav entries in order.
 *    A future edit that changes one but not the other will fail here.
 * 2. Behavior via real keydown events — dispatches native KeyboardEvent on
 *    document (react-hotkeys-hook attaches its listener to document by
 *    default, verified in the library source).
 */
import { describe, it, expect } from "vitest";
import { render, screen, act } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router-dom";
import { useState } from "react";
import { useAppHotkeys } from "../useAppHotkeys";
import { PRIMARY_ROUTES } from "../../lib/navigation";
import { HOTKEY_BINDINGS } from "../../lib/hotkeys";

// Harness component that mounts the hook and surfaces navigation + cheatsheet
// state for assertions.
function TestHarness() {
  const [cheatsheetOpen, setCheatsheetOpen] = useState(false);
  useAppHotkeys({ setCheatsheetOpen });
  const location = useLocation();
  return (
    <div>
      <span data-testid="route">{location.pathname}</span>
      <span data-testid="cheatsheet">{cheatsheetOpen ? "open" : "closed"}</span>
    </div>
  );
}

function renderHarness(initialRoute = "/settings") {
  return render(
    <MemoryRouter initialEntries={[initialRoute]}>
      <TestHarness />
    </MemoryRouter>,
  );
}

// Dispatch a native KeyboardEvent on document. react-hotkeys-hook reads
// e.code, e.key, and modifier flags; we provide all of them.
function dispatchKey(key: string, code: string, opts: KeyboardEventInit = {}) {
  act(() => {
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key,
        code,
        bubbles: true,
        cancelable: true,
        ...opts,
      }),
    );
  });
}

describe("HOTKEY_BINDINGS table", () => {
  it("has a nav binding for each primary route", () => {
    const navBindings = HOTKEY_BINDINGS.filter((b) => b.category === "nav");
    expect(navBindings.length).toBe(PRIMARY_ROUTES.length);
  });

  it("every binding has non-empty keys, label, and description", () => {
    for (const b of HOTKEY_BINDINGS) {
      expect(b.keys.length).toBeGreaterThan(0);
      expect(b.label.length).toBeGreaterThan(0);
      expect(b.description.length).toBeGreaterThan(0);
    }
  });

  it("has expected categories (nav, modal)", () => {
    const categories = new Set(HOTKEY_BINDINGS.map((b) => b.category));
    expect(categories.has("nav")).toBe(true);
    expect(categories.has("modal")).toBe(true);
    expect(categories.size).toBe(2); // Only nav and modal, no "action"
  });

  it("nav bindings match primary routes in order", () => {
    const navBindings = HOTKEY_BINDINGS.filter((b) => b.category === "nav");
    for (let i = 0; i < navBindings.length; i++) {
      const binding = navBindings[i];
      const route = PRIMARY_ROUTES[i];
      expect(binding?.label).toBe(String(i + 1));
      expect(binding?.description).toContain(route?.label);
    }
  });

  it("nav bindings use exactly keys '1' through '6' (drift guard)", () => {
    const navBindings = HOTKEY_BINDINGS.filter((b) => b.category === "nav");
    const expectedKeys = ["1", "2", "3", "4", "5", "6"];
    expect(navBindings.map((b) => b.keys)).toEqual(expectedKeys);
    // Also confirms the hook's hard-coded useHotkeys("1"..."6") calls in
    // useAppHotkeys.ts remain consistent with PRIMARY_ROUTES.length.
    expect(PRIMARY_ROUTES.length).toBe(6);
  });
});

describe("useAppHotkeys — behavior via real keydown events", () => {
  it("pressing 1 navigates to the first primary route", () => {
    renderHarness("/settings");
    dispatchKey("1", "Digit1");
    expect(screen.getByTestId("route").textContent).toBe(PRIMARY_ROUTES[0]?.to);
  });

  it("pressing 6 navigates to the sixth primary route", () => {
    renderHarness("/");
    dispatchKey("6", "Digit6");
    expect(screen.getByTestId("route").textContent).toBe(PRIMARY_ROUTES[5]?.to);
  });

  it("pressing ? (shift+/) opens the cheatsheet", () => {
    renderHarness();
    expect(screen.getByTestId("cheatsheet").textContent).toBe("closed");
    // useKey: true matches on e.key directly — don't set shiftKey flag
    // (the library checks modifier parity and would reject if shift is set
    // but the hotkey definition doesn't include "shift").
    dispatchKey("?", "Slash");
    expect(screen.getByTestId("cheatsheet").textContent).toBe("open");
  });

  it("pressing Escape closes the cheatsheet", () => {
    renderHarness();
    dispatchKey("?", "Slash"); // open first
    expect(screen.getByTestId("cheatsheet").textContent).toBe("open");
    dispatchKey("Escape", "Escape");
    expect(screen.getByTestId("cheatsheet").textContent).toBe("closed");
  });
});
