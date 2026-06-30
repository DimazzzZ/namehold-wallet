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
}

/**
 * Single source of truth for the application's primary navigation.
 * The shell sidebar and router both consume this list.
 */
export const PRIMARY_ROUTES: PrimaryRoute[] = [
  { key: "overview", to: "/", label: "Overview", description: "Operational summary and quick actions" },
  { key: "portfolio", to: "/portfolio", label: "Portfolio", description: "Inventory, batches, renewals, and DNS" },
  { key: "migration", to: "/migration", label: "Migration", description: "Namebase source and on-chain sync" },
  { key: "wallet", to: "/wallet", label: "Wallet", description: "Balance, send, receive, and history" },
  { key: "node", to: "/node", label: "Node", description: "hsd runtime status and lifecycle" },
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
