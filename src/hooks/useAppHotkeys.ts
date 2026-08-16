import { useHotkeys } from "react-hotkeys-hook";
import { useNavigate } from "react-router-dom";
import { PRIMARY_ROUTES } from "../lib/navigation";

interface UseAppHotkeysOptions {
  setCheatsheetOpen: (open: boolean) => void;
}

/**
 * Registers global keyboard shortcuts for the app. Mount once in Layout.
 *
 * - 1..6: navigate to the nth primary route.
 * - ?: open the keyboard-shortcuts cheatsheet.
 * - Esc: close the cheatsheet (Dialog handles its own Esc independently).
 *
 * All hotkeys are suppressed when focus is inside an input, textarea, or
 * contenteditable element (react-hotkeys-hook default behavior).
 */
export function useAppHotkeys({ setCheatsheetOpen }: UseAppHotkeysOptions) {
  const navigate = useNavigate();

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
}
