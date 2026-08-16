/**
 * Global keyboard shortcut bindings for the app.
 * Single source of truth for both the useAppHotkeys hook and the Cheatsheet.
 */

export interface HotkeyBinding {
  keys: string;           // react-hotkeys-hook format, e.g. "1", "shift+/"
  label: string;          // human-readable key label, e.g. "1", "?"
  description: string;    // what it does
  category: "nav" | "modal";
}

export const HOTKEY_BINDINGS: HotkeyBinding[] = [
  { keys: "1", label: "1", description: "Go to Wallet", category: "nav" },
  { keys: "2", label: "2", description: "Go to Activity", category: "nav" },
  { keys: "3", label: "3", description: "Go to Auctions", category: "nav" },
  { keys: "4", label: "4", description: "Go to Watchlist", category: "nav" },
  { keys: "5", label: "5", description: "Go to Move from Namebase", category: "nav" },
  { keys: "6", label: "6", description: "Go to Settings", category: "nav" },
  { keys: "shift+?", label: "?", description: "Open keyboard shortcuts", category: "modal" },
  { keys: "escape", label: "Esc", description: "Close dialog / modal", category: "modal" },
];
