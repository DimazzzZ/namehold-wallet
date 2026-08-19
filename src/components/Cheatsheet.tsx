import { useLocation } from "react-router-dom";
import { Dialog } from "./ui/Dialog";
import { HOTKEY_BINDINGS, type HotkeyCategory, type RouteScope } from "../lib/hotkeys";

interface CheatsheetProps {
  open: boolean;
  onClose: () => void;
}

const CATEGORY_LABELS: Record<HotkeyCategory, string> = {
  nav: "Navigation",
  modal: "Dialogs",
  action: "Actions (current view)",
  palette: "Command Palette",
  list: "List Navigation",
};

const CATEGORY_ORDER: HotkeyCategory[] = ["nav", "action", "list", "palette", "modal"];

/**
 * Keyboard-shortcut cheatsheet. Rendered from the shared HOTKEY_BINDINGS table
 * so it can never drift from what useAppHotkeys actually registers.
 */
export function Cheatsheet({ open, onClose }: CheatsheetProps) {
  const { pathname } = useLocation();

  const groups = CATEGORY_ORDER.map((category) => ({
    category,
    bindings: HOTKEY_BINDINGS.filter((b) => {
      if (b.category !== category) return false;
      // For route-scoped categories, only show bindings that apply to the
      // current route — so the cheatsheet always reads "keys that work here".
      if (category === "action" || category === "list") {
        const scopes = Array.isArray(b.scope) ? b.scope : [b.scope];
        return scopes.includes("*") || scopes.includes(pathname as RouteScope);
      }
      return true;
    }),
  })).filter((g) => g.bindings.length > 0);

  return (
    <Dialog open={open} onClose={onClose} title="Keyboard shortcuts">
      <div className="space-y-4" data-testid="cheatsheet">
        {groups.map((group) => (
          <div key={group.category}>
            <div className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-1">
              {CATEGORY_LABELS[group.category]}
            </div>
            <table className="w-full text-sm">
              <tbody>
                {group.bindings.map((b) => (
                  <tr key={b.keys}>
                    <td className="py-1 pr-4 text-gray-700">{b.description}</td>
                    <td className="py-1 text-right">
                      <kbd className="px-2 py-0.5 rounded border border-gray-300 bg-gray-50 text-xs font-mono text-gray-800">
                        {b.label}
                      </kbd>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ))}
      </div>
    </Dialog>
  );
}
