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
export type RouteScope = "/" | "/names" | "/market" | "/activity" | "/settings" | "*";

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
  { keys: "2", label: "2", description: "Go to Names", category: "nav", scope: "*" },
  { keys: "3", label: "3", description: "Go to Market", category: "nav", scope: "*" },
  { keys: "4", label: "4", description: "Go to Activity", category: "nav", scope: "*" },
  { keys: "5", label: "5", description: "Go to Settings", category: "nav", scope: "*" },

  // --- Dialogs (global) ---
  {
    keys: "shift+?",
    label: "Shift + ?",
    description: "Open keyboard shortcuts",
    category: "modal",
    scope: "*",
  },
  {
    keys: "escape",
    label: "Esc",
    description: "Close dialog / modal",
    category: "modal",
    scope: "*",
  },

  // --- Command palette (global) ---
  {
    keys: "meta+k,ctrl+k",
    label: "⌘K / Ctrl+K",
    description: "Open command palette",
    category: "palette",
    scope: "*",
  },

  // --- Action shortcuts (route-scoped) ---
  {
    keys: "s",
    label: "S",
    description: "Open Send",
    category: "action",
    scope: "/",
    actionId: "wallet:send",
    requiresWrite: true,
  },
  {
    keys: "r",
    label: "R",
    description: "Sync / Refresh",
    category: "action",
    scope: "/",
    actionId: "wallet:sync",
  },
  {
    keys: "u",
    label: "U",
    description: "Unlock / Lock wallet",
    category: "action",
    scope: "/",
    actionId: "wallet:toggleLock",
  },
  {
    keys: "q",
    label: "Q",
    description: "Open Receive",
    category: "action",
    scope: "/",
    actionId: "wallet:openReceive",
  },

  {
    keys: "/",
    label: "/",
    description: "Find a name",
    category: "action",
    scope: "/names",
    actionId: "names:focusLookup",
  },
  {
    keys: "b",
    label: "B",
    description: "Open batch bid",
    category: "action",
    scope: "/names",
    actionId: "names:batchBid",
    requiresWrite: true,
  },

  {
    keys: "/",
    label: "/",
    description: "Focus search",
    category: "action",
    scope: "/activity",
    actionId: "activity:focusSearch",
  },

  {
    keys: "/",
    label: "/",
    description: "Focus name search",
    category: "action",
    scope: "/market",
    actionId: "market:focusSearch",
  },
  {
    keys: "i",
    label: "I",
    description: "Import a listing",
    category: "action",
    scope: "/market",
    actionId: "market:import",
  },

  // --- List navigation (Names table) ---
  {
    keys: "j,down",
    label: "J / ↓",
    description: "Next name in list",
    category: "list",
    scope: "/names",
    actionId: "names:list:next",
  },
  {
    keys: "k,up",
    label: "K / ↑",
    description: "Previous name in list",
    category: "list",
    scope: "/names",
    actionId: "names:list:prev",
  },
  {
    keys: "enter",
    label: "Enter",
    description: "Open selected name",
    category: "list",
    scope: "/names",
    actionId: "names:list:open",
  },

  // --- List navigation (Market listings) ---
  {
    keys: "j,down",
    label: "J / ↓",
    description: "Next listing",
    category: "list",
    scope: "/market",
    actionId: "market:list:next",
  },
  {
    keys: "k,up",
    label: "K / ↑",
    description: "Previous listing",
    category: "list",
    scope: "/market",
    actionId: "market:list:prev",
  },
  {
    keys: "enter",
    label: "Enter",
    description: "Run the listing panel's button",
    category: "list",
    scope: "/market",
    actionId: "market:primary",
  },
];
