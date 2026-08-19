/**
 * Global keyboard shortcut bindings for the app.
 * Single source of truth for both the useAppHotkeys hook and the Cheatsheet.
 */

import type { ActionId } from "./actionBus";

export type HotkeyCategory = "nav" | "modal" | "action" | "palette" | "list";

/**
 * Which route(s) a binding is active on. "*" means global (fires everywhere).
 * Route strings must match the `to` values in PRIMARY_ROUTES.
 */
export type RouteScope =
  | "/"
  | "/activity"
  | "/auctions"
  | "/watchlist"
  | "/migration"
  | "/settings"
  | "*";

export interface HotkeyBinding {
  keys: string; // react-hotkeys-hook format, e.g. "1", "shift+?"
  label: string; // human-readable key label, e.g. "1", "?"
  description: string; // what it does
  category: HotkeyCategory;
  /** Route(s) this binding is active on. "*" = global. */
  scope: RouteScope | RouteScope[];
  /**
   * Action dispatched on the action bus when this key fires. Present for
   * `action` and `list` categories; absent for nav/modal/palette which are
   * handled inline by the hook.
   */
  actionId?: ActionId;
  /** If true, the action needs write capability (guarded / annotated). */
  requiresWrite?: boolean;
}

export const HOTKEY_BINDINGS: HotkeyBinding[] = [
  // --- Navigation (global) ---
  { keys: "1", label: "1", description: "Go to Wallet", category: "nav", scope: "*" },
  { keys: "2", label: "2", description: "Go to Activity", category: "nav", scope: "*" },
  { keys: "3", label: "3", description: "Go to Auctions", category: "nav", scope: "*" },
  { keys: "4", label: "4", description: "Go to Watchlist", category: "nav", scope: "*" },
  { keys: "5", label: "5", description: "Go to Move from Namebase", category: "nav", scope: "*" },
  { keys: "6", label: "6", description: "Go to Settings", category: "nav", scope: "*" },

  // --- Dialogs (global) ---
  { keys: "shift+?", label: "Shift + ?", description: "Open keyboard shortcuts", category: "modal", scope: "*" },
  { keys: "escape", label: "Esc", description: "Close dialog / modal", category: "modal", scope: "*" },

  // --- Command palette (global) ---
  { keys: "meta+k,ctrl+k", label: "⌘K / Ctrl+K", description: "Open command palette", category: "palette", scope: "*" },

  // --- Action shortcuts (route-scoped) ---
  { keys: "s", label: "S", description: "Open Send", category: "action", scope: "/", actionId: "wallet:send", requiresWrite: true },
  { keys: "r", label: "R", description: "Sync / Refresh", category: "action", scope: "/", actionId: "wallet:sync" },
  { keys: "u", label: "U", description: "Unlock / Lock wallet", category: "action", scope: "/", actionId: "wallet:toggleLock" },
  { keys: "q", label: "Q", description: "Toggle receive QR", category: "action", scope: "/", actionId: "wallet:toggleQr" },
  { keys: "/", label: "/", description: "Focus name filter", category: "action", scope: "/", actionId: "wallet:focusFilter" },

  { keys: "/", label: "/", description: "Focus name lookup", category: "action", scope: "/auctions", actionId: "auctions:focusLookup" },
  { keys: "b", label: "B", description: "Open batch bid", category: "action", scope: "/auctions", actionId: "auctions:batchBid", requiresWrite: true },

  { keys: "a", label: "A", description: "Focus add name", category: "action", scope: "/watchlist", actionId: "watchlist:focusAdd" },
  { keys: "e", label: "E", description: "Export CSV", category: "action", scope: "/watchlist", actionId: "watchlist:exportCsv" },

  { keys: "/", label: "/", description: "Focus search", category: "action", scope: "/activity", actionId: "activity:focusSearch" },

  // --- List navigation (Wallet owned-names list) ---
  { keys: "j,down", label: "J / ↓", description: "Next name in list", category: "list", scope: "/", actionId: "wallet:list:next" },
  { keys: "k,up", label: "K / ↑", description: "Previous name in list", category: "list", scope: "/", actionId: "wallet:list:prev" },
  { keys: "enter", label: "Enter", description: "Open selected name", category: "list", scope: "/", actionId: "wallet:list:open" },
];
