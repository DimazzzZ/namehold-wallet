import type { LucideIcon } from "lucide-react";
import { Globe, History, Settings, Store, Wallet } from "lucide-react";
import type { NavKey } from "../types";

export type { NavKey };

export interface PrimaryRoute {
  key: NavKey;
  to: string;
  label: string;
  description: string;
  /** A short tag beside the label, such as "New" for a recently added section. */
  badge?: string;
  icon: LucideIcon;
}

/** Wallet-first primary navigation; digits 1–5 follow this order. */
export const PRIMARY_ROUTES: PrimaryRoute[] = [
  {
    key: "wallet",
    to: "/",
    label: "Wallet",
    icon: Wallet,
    description: "Balance, send, receive, and what needs you",
  },
  {
    key: "names",
    to: "/names",
    label: "Names",
    icon: Globe,
    description: "Names you own, bid on or watch — and finding new ones",
  },
  {
    key: "market",
    to: "/market",
    label: "Market",
    icon: Store,
    description: "Buy and sell names through Shakedex",
    badge: "New",
  },
  {
    key: "activity",
    to: "/activity",
    label: "Activity",
    icon: History,
    description: "Every transaction and name action",
  },
  {
    key: "settings",
    to: "/settings",
    label: "Settings",
    icon: Settings,
    description: "Connections, node, notifications, import and safety",
  },
];

/** Old v1 URLs. Each keeps working forever as a redirect. */
export const ROUTE_ALIASES: { from: string; to: string }[] = [
  { from: "/auctions", to: "/names?filter=auction" },
  { from: "/watchlist", to: "/names?filter=watching" },
  { from: "/migration", to: "/settings?section=import" },
  // v1 StatusStrip pushed "/wallet", which only worked through the "*" fallback.
  { from: "/wallet", to: "/" },
];
