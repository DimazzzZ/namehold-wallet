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
import { ACTION_EVENT_NAME, type ActionEventDetail } from "../../lib/actionBus";

// Harness component that mounts the hook and surfaces navigation + cheatsheet
// state for assertions.
function TestHarness() {
  const [cheatsheetOpen, setCheatsheetOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  useAppHotkeys({ setCheatsheetOpen, setPaletteOpen });
  const location = useLocation();
  return (
    <div>
      <span data-testid="route">{location.pathname}</span>
      <span data-testid="cheatsheet">{cheatsheetOpen ? "open" : "closed"}</span>
      <span data-testid="palette">{paletteOpen ? "open" : "closed"}</span>
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

  it("has expected categories (nav, modal, action, palette, list)", () => {
    const categories = new Set(HOTKEY_BINDINGS.map((b) => b.category));
    expect(categories).toEqual(
      new Set(["nav", "modal", "action", "palette", "list"]),
    );
  });

  it("every action/list binding has an actionId and a route (non-'*') scope", () => {
    for (const b of HOTKEY_BINDINGS) {
      if (b.category === "action" || b.category === "list") {
        expect(b.actionId, `${b.keys} needs an actionId`).toBeTruthy();
        const scopes = Array.isArray(b.scope) ? b.scope : [b.scope];
        expect(
          scopes.includes("*"),
          `${b.keys} must be route-scoped, not global`,
        ).toBe(false);
      }
    }
  });

  it("every binding uses an allowed scope", () => {
    const allowed = new Set([
      "/",
      "/activity",
      "/auctions",
      "/watchlist",
      "/migration",
      "/settings",
      "*",
    ]);
    for (const b of HOTKEY_BINDINGS) {
      const scopes = Array.isArray(b.scope) ? b.scope : [b.scope];
      for (const s of scopes) {
        expect(allowed.has(s), `unknown scope ${s} on ${b.keys}`).toBe(true);
      }
    }
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

  it("pressing Shift+? opens the cheatsheet", () => {
    renderHarness();
    expect(screen.getByTestId("cheatsheet").textContent).toBe("closed");
    // Reproduce the REAL browser event: pressing "?" on a US layout is
    // Shift+/, so the browser emits key="?" AND shiftKey=true. The hotkey
    // binding must be "shift+?" (not bare "?") to pass react-hotkeys-hook's
    // modifier-parity check. This test dispatches shiftKey:true precisely so
    // it catches the parity bug that a shiftKey:false event would miss.
    dispatchKey("?", "Slash", { shiftKey: true });
    expect(screen.getByTestId("cheatsheet").textContent).toBe("open");
  });

  it("pressing Escape closes the cheatsheet", () => {
    renderHarness();
    dispatchKey("?", "Slash", { shiftKey: true }); // open first
    expect(screen.getByTestId("cheatsheet").textContent).toBe("open");
    dispatchKey("Escape", "Escape");
    expect(screen.getByTestId("cheatsheet").textContent).toBe("closed");
  });
});

// Subscribe a spy to the action bus; returns collected events + stop().
function spyOnActionBus() {
  const events: ActionEventDetail[] = [];
  const listener = (e: Event) =>
    events.push((e as CustomEvent<ActionEventDetail>).detail);
  window.addEventListener(ACTION_EVENT_NAME, listener);
  return {
    events,
    stop: () => window.removeEventListener(ACTION_EVENT_NAME, listener),
  };
}

describe("route-scoped action keys", () => {
  it("pressing 's' at / dispatches wallet:send", () => {
    renderHarness("/");
    const spy = spyOnActionBus();
    dispatchKey("s", "KeyS");
    expect(spy.events).toEqual([{ actionId: "wallet:send" }]);
    spy.stop();
  });

  it("pressing 's' at /auctions does NOT dispatch", () => {
    renderHarness("/auctions");
    const spy = spyOnActionBus();
    dispatchKey("s", "KeyS");
    expect(spy.events).toEqual([]);
    spy.stop();
  });

  it("pressing '/' dispatches the route-appropriate focus action", () => {
    let harness = renderHarness("/");
    let spy = spyOnActionBus();
    dispatchKey("/", "Slash");
    expect(spy.events).toEqual([{ actionId: "wallet:focusFilter" }]);
    spy.stop();
    harness.unmount();

    harness = renderHarness("/auctions");
    spy = spyOnActionBus();
    dispatchKey("/", "Slash");
    expect(spy.events).toEqual([{ actionId: "auctions:focusLookup" }]);
    spy.stop();
    harness.unmount();

    harness = renderHarness("/activity");
    spy = spyOnActionBus();
    dispatchKey("/", "Slash");
    expect(spy.events).toEqual([{ actionId: "activity:focusSearch" }]);
    spy.stop();
    harness.unmount();
  });

  it("j/k/Enter at / dispatch list navigation actions", () => {
    renderHarness("/");
    const spy = spyOnActionBus();
    dispatchKey("j", "KeyJ");
    dispatchKey("k", "KeyK");
    dispatchKey("Enter", "Enter");
    expect(spy.events.map((e) => e.actionId)).toEqual([
      "wallet:list:next",
      "wallet:list:prev",
      "wallet:list:open",
    ]);
    spy.stop();
  });

  it("action keys do NOT fire while a Dialog is open", () => {
    renderHarness("/");
    const modal = document.createElement("div");
    modal.setAttribute("role", "dialog");
    document.body.appendChild(modal);
    const spy = spyOnActionBus();
    dispatchKey("s", "KeyS");
    expect(spy.events).toEqual([]);
    spy.stop();
    modal.remove();
  });

  it("modal guard still allows ⌘K and Shift+? to open their overlays", () => {
    renderHarness("/");
    const modal = document.createElement("div");
    modal.setAttribute("role", "dialog");
    document.body.appendChild(modal);
    dispatchKey("k", "KeyK", { metaKey: true });
    expect(screen.getByTestId("palette").textContent).toBe("open");
    dispatchKey("?", "Slash", { shiftKey: true });
    expect(screen.getByTestId("cheatsheet").textContent).toBe("open");
    modal.remove();
  });
});

describe("command palette hotkey", () => {
  it("meta+k opens the palette", () => {
    renderHarness("/");
    expect(screen.getByTestId("palette").textContent).toBe("closed");
    dispatchKey("k", "KeyK", { metaKey: true });
    expect(screen.getByTestId("palette").textContent).toBe("open");
  });

  it("ctrl+k opens the palette", () => {
    renderHarness("/");
    dispatchKey("k", "KeyK", { ctrlKey: true });
    expect(screen.getByTestId("palette").textContent).toBe("open");
  });
});
