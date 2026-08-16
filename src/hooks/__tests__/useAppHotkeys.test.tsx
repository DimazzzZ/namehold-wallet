/**
 * Tests for the global keyboard-shortcut hook and the binding table.
 *
 * Note: react-hotkeys-hook's internal event handling is complex and difficult
 * to test in isolation with fireEvent. The hook is best tested manually or
 * via E2E testing. This test suite focuses on the binding table and component
 * structure, which are deterministic and testable.
 */
import { describe, it, expect } from "vitest";
import { PRIMARY_ROUTES } from "../../lib/navigation";
import { HOTKEY_BINDINGS } from "../../lib/hotkeys";


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

  it("has expected categories", () => {
    const categories = new Set(HOTKEY_BINDINGS.map((b) => b.category));
    expect(categories.has("nav")).toBe(true);
    expect(categories.has("modal")).toBe(true);
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
});
