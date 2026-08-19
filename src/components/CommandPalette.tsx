import { useEffect, useMemo, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { PRIMARY_ROUTES } from "../lib/navigation";
import { HOTKEY_BINDINGS, type RouteScope } from "../lib/hotkeys";
import { dispatchAction, type ActionId } from "../lib/actionBus";
import { useWriteCapability } from "../queries/wallet";
import { cn } from "../lib/utils";

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
}

interface PaletteCommand {
  id: string;
  label: string;
  shortcut?: string;
  category: string;
  run: () => void;
}

/**
 * Case-insensitive subsequence match — the query's characters must appear in
 * order within the text, but not necessarily contiguously. Keeps "fuzzy"
 * search dependency-free.
 */
export function fuzzyMatch(query: string, text: string): boolean {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  let qi = 0;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) qi++;
  }
  return qi === q.length;
}

/**
 * Build the command list from the SAME registries the rest of the app uses:
 * PRIMARY_ROUTES for navigation and HOTKEY_BINDINGS (action category) for
 * view actions. Actions that aren't available on the current route, or that
 * require write capability on a read-only wallet, are omitted.
 */
export function buildCommands(
  navigate: (to: string) => void,
  pathname: string,
  canWrite: boolean,
): PaletteCommand[] {
  const commands: PaletteCommand[] = [];

  PRIMARY_ROUTES.forEach((route, i) => {
    commands.push({
      id: `nav:${route.key}`,
      label: `Go to ${route.label}`,
      shortcut: String(i + 1),
      category: "Navigation",
      run: () => navigate(route.to),
    });
  });

  for (const b of HOTKEY_BINDINGS) {
    if (b.category !== "action" || !b.actionId) continue;
    const scopes = Array.isArray(b.scope) ? b.scope : [b.scope];
    const onRoute =
      scopes.includes("*") || scopes.includes(pathname as RouteScope);
    if (!onRoute) continue;
    if (b.requiresWrite && !canWrite) continue;
    const actionId = b.actionId;
    commands.push({
      id: actionId,
      label: b.description,
      shortcut: b.label,
      category: "Actions",
      run: () => dispatchAction(actionId as ActionId),
    });
  }

  return commands;
}

/**
 * Command palette overlay (⌘K / Ctrl+K). Searchable list of navigation targets
 * and view actions, keyboard-driven. Renders above the shared Dialog (z-60 vs
 * z-50) and tags itself with `data-modal-open` so the global action hotkeys
 * stay suppressed while it's open.
 */
export function CommandPalette({ open, onClose }: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const { data: writeCap } = useWriteCapability();
  const canWrite = writeCap?.canWrite ?? false;
  const inputRef = useRef<HTMLInputElement>(null);

  const commands = useMemo(
    () => buildCommands(navigate, pathname, canWrite),
    [navigate, pathname, canWrite],
  );

  const filtered = useMemo(
    () => (query ? commands.filter((c) => fuzzyMatch(query, c.label)) : commands),
    [commands, query],
  );

  // Reset selection whenever the filtered set changes.
  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  // Reset query + selection each time the palette opens; focus the input.
  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIndex(0);
      inputRef.current?.focus();
    }
  }, [open]);

  // Capture-phase Escape handler: closing the palette must NOT also close a
  // Dialog mounted underneath (Dialog listens for Escape on document). We
  // intercept in the capture phase and stop propagation so only the palette
  // reacts.
  useEffect(() => {
    if (!open) return;
    const onKeyDownCapture = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDownCapture, true);
    return () => document.removeEventListener("keydown", onKeyDownCapture, true);
  }, [open, onClose]);

  if (!open) return null;

  const runSelected = () => {
    const cmd = filtered[selectedIndex];
    if (cmd) {
      cmd.run();
      onClose();
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      runSelected();
    }
    // Escape handled by the capture-phase listener above.
  };

  return (
    <div
      className="fixed inset-0 z-[60] flex items-start justify-center pt-[20vh]"
      data-modal-open="true"
      data-testid="command-palette"
    >
      <div className="fixed inset-0 bg-black/30" onClick={onClose} />
      <div
        className="relative bg-white rounded-lg shadow-2xl w-full max-w-md mx-4"
        onKeyDown={onKeyDown}
      >
        <input
          ref={inputRef}
          autoFocus
          className="w-full px-4 py-3 border-b text-sm outline-none rounded-t-lg"
          placeholder="Type a command…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          data-testid="command-palette-input"
        />
        <ul className="max-h-64 overflow-auto py-1" role="listbox">
          {filtered.map((cmd, i) => (
            <li
              key={cmd.id}
              role="option"
              aria-selected={i === selectedIndex}
              className={cn(
                "px-4 py-2 text-sm cursor-pointer flex items-center justify-between gap-4",
                i === selectedIndex && "bg-blue-50",
              )}
              onMouseEnter={() => setSelectedIndex(i)}
              onClick={() => {
                cmd.run();
                onClose();
              }}
            >
              <span className="flex flex-col">
                <span className="text-gray-800">{cmd.label}</span>
                <span className="text-[10px] uppercase tracking-wide text-gray-400">
                  {cmd.category}
                </span>
              </span>
              {cmd.shortcut && (
                <kbd className="text-xs text-gray-400 font-mono">{cmd.shortcut}</kbd>
              )}
            </li>
          ))}
          {filtered.length === 0 && (
            <li className="px-4 py-2 text-sm text-gray-400">No results</li>
          )}
        </ul>
      </div>
    </div>
  );
}
