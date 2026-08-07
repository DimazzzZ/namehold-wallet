import type {
  AppRouteKey,
  MigrationSectionKey,
  WorkspaceTab,
} from "../types";

export interface PrimaryRoute {
  key: AppRouteKey;
  to: string;
  label: string;
  description: string;
}

/**
 * Wallet-first primary navigation.
 *
 * Order intentionally leads with the wallet (the default landing screen),
 * followed by the urgent Namebase migration flow, then secondary tools.
 */
export const PRIMARY_ROUTES: PrimaryRoute[] = [
  { key: "wallet", to: "/", label: "Wallet", description: "Balance, send, receive, and history" },
  { key: "activity", to: "/activity", label: "Activity", description: "Full transaction and name-action history" },
  { key: "auctions", to: "/auctions", label: "Auctions", description: "Acquire new Handshake TLDs" },
  { key: "watchlist", to: "/watchlist", label: "Watchlist", description: "Track names you don't own" },
  { key: "migration", to: "/migration", label: "Move from Namebase", description: "Guided transfer of your domains from Namebase" },
  { key: "settings", to: "/settings", label: "Settings", description: "Configuration and safety" },
];


export const MIGRATION_TABS: WorkspaceTab<MigrationSectionKey>[] = [
  { key: "namebase", label: "Namebase", description: "Connect and transfer source" },
];
