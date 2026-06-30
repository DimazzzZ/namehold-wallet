import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type { HsdBalance, HsdName } from "../types";

export function useWalletConnection() {
  return useQuery({
    queryKey: ["wallet", "connection"],
    queryFn: () => invoke<{ connected: boolean; info?: unknown; error?: string }>("check_connection"),
    refetchInterval: 15_000,
    refetchOnMount: "always",
    retry: 1,
  });
}

export function useWalletBalance() {
  return useQuery({
    queryKey: ["wallet", "balance"],
    queryFn: () => invoke<HsdBalance>("get_balance"),
    refetchInterval: 30_000,
    retry: false,
  });
}

export function useWalletAddress() {
  return useQuery({
    queryKey: ["wallet", "address"],
    queryFn: () => invoke<string>("get_address"),
    retry: false,
  });
}

export function useWalletNames() {
  return useQuery({
    queryKey: ["wallet", "names"],
    queryFn: () => invoke<HsdName[]>("get_names"),
    retry: false,
  });
}

export function useWalletTransactions() {
  return useQuery({
    queryKey: ["wallet", "transactions"],
    queryFn: () => invoke<unknown[]>("get_transactions"),
    retry: false,
  });
}

export function useWalletList() {
  return useQuery({
    queryKey: ["wallet", "list"],
    queryFn: () => invoke<string[]>("list_wallets"),
    retry: false,
    staleTime: 30_000,
  });
}

export function useSendHns() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { address: string; value: number; passphrase: string }) =>
      invoke("send_hns", args as Record<string, unknown>),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["wallet"] });
    },
  });
}

export function useTransferName() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { name: string; address: string; passphrase: string }) =>
      invoke("transfer_name", args as Record<string, unknown>),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["assets"] });
    },
  });
}
