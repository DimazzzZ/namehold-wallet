import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";

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
