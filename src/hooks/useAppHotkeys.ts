import { useHotkeys } from "react-hotkeys-hook";
import { useNavigate, useLocation } from "react-router-dom";
import { PRIMARY_ROUTES } from "../lib/navigation";
import { HOTKEY_BINDINGS, type RouteScope } from "../lib/hotkeys";
import { dispatchAction, type ActionId } from "../lib/actionBus";

interface UseAppHotkeysOptions {
  setCheatsheetOpen: (open: boolean) => void;
  setPaletteOpen: (open: boolean) => void;
}

/**
 * True when a modal/dialog is currently on screen. Action keys (s, r, j, …)
 * must not fire while a Dialog or the command palette is open — the overlay
 * owns the keyboard. The palette tags itself with `data-modal-open` and the
 * shared Dialog renders `role="dialog"`.
 */
function isModalOpen(): boolean {
  if (typeof document === "undefined") return false;
  return !!document.querySelector('[role="dialog"], [data-modal-open="true"]');
}

/**
 * Build, once, a map of physical key -> the route-scoped action bindings that
 * use it. Multiple routes can share a key (e.g. "/" focuses the search on
 * Wallet, Auctions, and Activity); we register that key with ONE useHotkeys
 * call and pick the right binding by pathname at fire time. Registering inside
 * a loop is fine here because the map is computed at module load and its size
 * never changes across renders — the hook order stays stable.
 */
type ScopedAction = { scopes: RouteScope[]; actionId: ActionId };

const ACTION_KEY_MAP: Array<{ keys: string; entries: ScopedAction[] }> = (() => {
  const byKey = new Map<string, ScopedAction[]>();
  for (const b of HOTKEY_BINDINGS) {
    if ((b.category !== "action" && b.category !== "list") || !b.actionId) continue;
    const scopes = Array.isArray(b.scope) ? b.scope : [b.scope];
    const list = byKey.get(b.keys) ?? [];
    list.push({ scopes, actionId: b.actionId });
    byKey.set(b.keys, list);
  }
  return [...byKey.entries()].map(([keys, entries]) => ({ keys, entries }));
})();

/**
 * Registers global keyboard shortcuts for the app. Mount once in Layout.
 *
 * - 1..6: navigate to the nth primary route.
 * - ?: open the keyboard-shortcuts cheatsheet.
 * - Esc: close the cheatsheet (Dialog handles its own Esc independently).
 * - ⌘K / Ctrl+K: open the command palette.
 * - Route-scoped action keys (s, r, u, q, /, b, a, e) and list-nav keys
 *   (j/k/↓/↑/Enter) dispatch typed actions on the action bus; the owning view
 *   subscribes and runs the handler. See src/lib/hotkeys.ts for the registry.
 *
 * All hotkeys are suppressed when focus is inside an input, textarea, or
 * contenteditable element (react-hotkeys-hook default behavior). Action/list
 * keys are additionally suppressed while a modal/palette is open.
 */
export function useAppHotkeys({
  setCheatsheetOpen,
  setPaletteOpen,
}: UseAppHotkeysOptions) {
  const navigate = useNavigate();
  const { pathname } = useLocation();

  // Bound guard: navigate only if the index is within PRIMARY_ROUTES.
  const goto = (i: number) => {
    const route = PRIMARY_ROUTES[i];
    if (route) navigate(route.to);
  };

  // Navigation: 1..N for each primary route.
  // These must stay in sync with the nav entries in HOTKEY_BINDINGS (src/lib/hotkeys.ts).
  // A test in useAppHotkeys.test.tsx asserts they match — if you add a route, add a binding too.
  useHotkeys("1", () => goto(0), { preventDefault: true });
  useHotkeys("2", () => goto(1), { preventDefault: true });
  useHotkeys("3", () => goto(2), { preventDefault: true });
  useHotkeys("4", () => goto(3), { preventDefault: true });
  useHotkeys("5", () => goto(4), { preventDefault: true });
  useHotkeys("6", () => goto(5), { preventDefault: true });

  // Cheatsheet: Shift+? key. The definition MUST include `shift+` because the
  // browser emits `shiftKey: true` when the user types "?" (Shift+/ on US
  // layouts), and react-hotkeys-hook enforces modifier parity — a bare "?"
  // definition has shift:false and would be rejected against a shiftKey:true
  // event, so the hotkey would never fire. `useKey: true` matches on
  // `event.key` (the "?" character) rather than `event.code` ("Slash"), so it
  // stays layout-robust across US and non-US keyboards.
  useHotkeys("shift+?", () => setCheatsheetOpen(true), {
    preventDefault: true,
    useKey: true,
  });

  // Esc: close cheatsheet (Dialog.tsx also handles its own Esc for modals).
  useHotkeys("escape", () => setCheatsheetOpen(false));

  // Command palette: ⌘K (macOS) / Ctrl+K (Windows/Linux). Allowed even when a
  // modal is open — the palette layers on top.
  useHotkeys("meta+k,ctrl+k", () => setPaletteOpen(true), {
    preventDefault: true,
  });

  // Route-scoped action + list keys. One useHotkeys per unique physical key
  // (stable count → obeys the Rules of Hooks); the callback resolves the
  // active route to the correct action. Suppressed while a modal is open and
  // (by react-hotkeys-hook default) while focus is in an input/textarea.
  for (const { keys, entries } of ACTION_KEY_MAP) {
    // "/" is Shift+7 on some layouts and code="Slash" everywhere; use
    // event.key matching (useKey) so react-hotkeys-hook resolves it by the
    // produced character, not the physical scancode.
    const useKey = keys.includes("/");
    // eslint-disable-next-line react-hooks/rules-of-hooks
    useHotkeys(
      keys,
      () => {
        if (isModalOpen()) return;
        const match = entries.find(
          (e) => e.scopes.includes("*") || e.scopes.includes(pathname as RouteScope),
        );
        if (match) dispatchAction(match.actionId);
      },
      { preventDefault: true, useKey },
    );
  }
}
