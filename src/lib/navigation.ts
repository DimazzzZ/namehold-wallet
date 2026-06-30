import type {
  AppRouteKey,
  PortfolioSectionKey,
  MigrationSectionKey,
  WorkspaceTab,
} from "../types";

export interface PrimaryRoute {
  key: AppRouteKey;
  to: string;
  label: string;
  description: string;
  /** When true, only shown if the user enabled advanced mode in Settings. */
  advanced?: boolean;
}

/**
 * Wallet-first primary navigation.
 *
 * Order intentionally leads with the wallet (the default landing screen),
 * followed by the urgent Namebase migration flow, then secondary/advanced
 * tools. Items marked `advanced: true` are only shown when the user has opted
 * into advanced mode in Settings.
 */
export const PRIMARY_ROUTES: PrimaryRoute[] = [
  { key: "wallet", to: "/", label: "Wallet", description: "Balance, send, receive, and history" },
  { key: "migration", to: "/migration", label: "Move from Namebase", description: "Guided transfer of your domains from Namebase" },
  { key: "portfolio", to: "/portfolio", label: "Portfolio", description: "Inventory, batches, renewals, and DNS", advanced: true },
  { key: "settings", to: "/settings", label: "Settings", description: "Configuration and safety" },
];

export const PORTFOLIO_TABS: WorkspaceTab<PortfolioSectionKey>[] = [
  { key: "inventory", label: "Inventory", description: "All imported TLDs" },
  { key: "batches", label: "Batches", description: "Migration groups" },
  { key: "renewals", label: "Renewals", description: "Expiration tracking" },
  { key: "dns", label: "DNS", description: "Resource records" },
];

export const MIGRATION_TABS: WorkspaceTab<MigrationSectionKey>[] = [
  { key: "namebase", label: "Namebase", description: "Connect and transfer source" },
  { key: "sync", label: "Sync & Verify", description: "Match on-chain ownership" },
];
