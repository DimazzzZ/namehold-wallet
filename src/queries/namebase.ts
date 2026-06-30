import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type { StatusTone } from "../types";

/**
 * A domain transfer ("withdrawal") as Namebase tracks it (`/api/domains/withdrawals`).
 * `status` is Namebase's own lifecycle string for the two-phase Handshake transfer:
 * `transfer_pending` → `transfer_completed` → `finalize_pending` → `finalize_completed`.
 */
export interface DomainWithdrawal {
  id: string;
  domain: string;
  destination_address: string;
  status: string;
  status_note: string | null;
  created_at: string;
  updated_at: string;
}

/**
 * Domain transfers exactly as Namebase reports them (`/api/domains/withdrawals`),
 * so the app mirrors Namebase's own transfer list + statuses rather than guessing
 * from on-chain state. Shares the `["namebase-domain-withdrawals"]` key that a
 * transfer already invalidates.
 */
export function useNamebaseDomainWithdrawals() {
  return useQuery<DomainWithdrawal[]>({
    queryKey: ["namebase-domain-withdrawals"],
    queryFn: async () => {
      const raw = await invoke<{ withdrawals?: DomainWithdrawal[] } | DomainWithdrawal[]>(
        "fetch_namebase_domain_withdrawals",
      );
      return Array.isArray(raw) ? raw : (raw?.withdrawals ?? []);
    },
    retry: false,
  });
}

/**
 * A custodial domain's expiry, from Namebase's renewal calendar
 * (`/api/domains/renewals` → `{ expiring: [...] }`). `estimated_date` is ISO+Z.
 */
export interface NamebaseRenewal {
  domain: string;
  expire_block: number;
  estimated_date: string;
}

/**
 * Domains expiring on Namebase, soonest first — so a migrating user can renew or
 * move a name BEFORE it lapses. Backed by the already-wired `fetch_namebase_renewals`
 * command (unwraps the `expiring` array, mirroring the withdrawals hooks).
 */
export function useNamebaseRenewals(enabled: boolean) {
  return useQuery<NamebaseRenewal[]>({
    queryKey: ["namebase-renewals"],
    enabled,
    queryFn: async () => {
      const raw = await invoke<{ expiring?: NamebaseRenewal[] } | NamebaseRenewal[]>(
        "fetch_namebase_renewals",
      );
      const list = Array.isArray(raw) ? raw : (raw?.expiring ?? []);
      return [...list].sort((a, b) =>
        (a.estimated_date ?? "").localeCompare(b.estimated_date ?? ""),
      );
    },
    retry: false,
  });
}

/** True once a domain transfer has fully landed (finalized) on Namebase's side. */
export function isDomainTransferDone(status: string): boolean {
  return (status || "").toLowerCase() === "finalize_completed";
}

/**
 * Humanize a Namebase status string into a label + badge tone. Covers both the
 * domain-transfer vocabulary (`transfer_*` / `finalize_*`) and the currency
 * withdrawal vocabulary (`pending` / `completed`), so the app shows the same
 * words Namebase does.
 */
export function namebaseStatus(status: string): { label: string; tone: StatusTone } {
  const s = (status || "").toLowerCase().trim();
  if (!s) return { label: "—", tone: "default" };
  if (s.includes("fail") || s.includes("cancel") || s.includes("error") || s.includes("reject"))
    return { label: humanizeStatus(s), tone: "error" };
  if (s === "finalize_completed") return { label: "Completed", tone: "success" };
  if (s === "transfer_completed") return { label: "Transfer sent — finalizing", tone: "info" };
  if (s === "completed" || s === "complete") return { label: "Completed", tone: "success" };
  if (s.includes("pending") || s.includes("progress") || s.includes("processing") || s.includes("waiting"))
    return { label: humanizeStatus(s), tone: "info" };
  return { label: humanizeStatus(s), tone: "default" };
}

/** "transfer_completed" → "Transfer completed". */
function humanizeStatus(status: string): string {
  return status
    .split(/[_\s]+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

export interface NamebaseAccount {
  email?: string;
  hns_balance?: number;
  [key: string]: unknown;
}

export interface NamebaseStatus {
  connected: boolean;
  has_cookie: boolean;
  account?: NamebaseAccount;
  error?: string;
}

export interface NamebaseDomain {
  name: string;
  [key: string]: unknown;
}

export function useNamebaseStatus() {
  return useQuery({
    queryKey: ["namebase", "status"],
    queryFn: () => invoke<NamebaseStatus>("get_namebase_status"),
    retry: false,
  });
}

export function useNamebaseDomains(enabled: boolean) {
  return useQuery({
    queryKey: ["namebase", "domains"],
    queryFn: () =>
      invoke<{ domains: NamebaseDomain[] }>("fetch_namebase_domains"),
    enabled,
  });
}

export function useNamebaseStakedDomains(enabled: boolean) {
  return useQuery({
    queryKey: ["namebase", "staked"],
    queryFn: () =>
      invoke<{ stakedDomains: NamebaseDomain[] }>("fetch_namebase_staked"),
    enabled,
  });
}

export function useConnectNamebase() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (cookie: string) =>
      invoke("connect_namebase", { cookie: cookie.trim() }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["namebase"] }),
  });
}

export function useDisconnectNamebase() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => invoke("disconnect_namebase"),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["namebase"] }),
  });
}

export interface NamebaseImportResult {
  imported: number;
  staked_count: number;
  [key: string]: unknown;
}

export function useImportFromNamebase() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => invoke<NamebaseImportResult>("import_from_namebase"),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["assets"] });
      qc.invalidateQueries({ queryKey: ["dashboard"] });
      qc.invalidateQueries({ queryKey: ["overview"] });
    },
  });
}

export function useTransferNamebaseDomain() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { name: string; address: string }) =>
      invoke("namebase_transfer_domain", {
        name: args.name,
        address: args.address,
      }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["namebase"] });
      qc.invalidateQueries({ queryKey: ["assets"] });
    },
  });
}

/** A currency withdrawal from the Namebase custodial balance. */
export interface NamebaseWithdrawal {
  id: string;
  currency: string;
  amount: string; // dollarydoos, as a string
  destination_address: string;
  status: string; // e.g. "pending" | "completed"
  status_note: string | null;
  created_at: string;
}

/** HNS/BTC withdrawals from Namebase (`/api/withdrawals`), newest-first. */
export function useNamebaseWithdrawals() {
  return useQuery<NamebaseWithdrawal[]>({
    queryKey: ["namebase-withdrawals"],
    queryFn: async () => {
      const raw = await invoke<{ withdrawals?: NamebaseWithdrawal[] } | NamebaseWithdrawal[]>(
        "fetch_namebase_withdrawals",
      );
      const list = Array.isArray(raw) ? raw : (raw?.withdrawals ?? []);
      return list;
    },
    retry: false,
  });
}

/** Withdraw HNS funds from Namebase to an address. `amount` is dollarydoos. */
export function useWithdrawHns() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { address: string; amount: string }) =>
      invoke("namebase_withdraw_hns", { address: args.address, amount: args.amount }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["namebase"] });
      qc.invalidateQueries({ queryKey: ["namebase-withdrawals"] });
    },
  });
}
