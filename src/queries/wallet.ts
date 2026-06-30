import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type {
  WalletProfileSummary,
  SignerSessionSummary,
  WriteCapability,
  WalletBalances,
  TxDraftSummary,
  BroadcastResult,
} from "../types";

// ---------------------------------------------------------------------------
// Non-custodial wallet hooks
// ---------------------------------------------------------------------------

export function useWalletProfiles() {
  return useQuery({
    queryKey: ["wallet", "profiles"],
    queryFn: () => invoke<WalletProfileSummary[]>("list_wallet_profiles"),
    retry: false,
  });
}

/** The active profile, derived from the profile list. */
export function useActiveProfile() {
  const q = useWalletProfiles();
  return { ...q, data: q.data?.find((p) => p.active) ?? null };
}

export function useSignerSession() {
  return useQuery({
    queryKey: ["wallet", "signer"],
    queryFn: () => invoke<SignerSessionSummary>("get_signer_session"),
    refetchInterval: 30_000,
    retry: false,
  });
}

export function useWriteCapability() {
  return useQuery({
    queryKey: ["wallet", "writeCapability"],
    queryFn: () => invoke<WriteCapability>("get_write_capability"),
    refetchInterval: 30_000,
    retry: false,
  });
}

export function useWalletBalances() {
  return useQuery({
    queryKey: ["wallet", "balances"],
    queryFn: () => invoke<WalletBalances>("get_wallet_balances"),
    retry: false,
  });
}

export function useTxDrafts() {
  return useQuery({
    queryKey: ["wallet", "drafts"],
    queryFn: () => invoke<TxDraftSummary[]>("list_tx_drafts"),
    retry: false,
  });
}

function useWalletMutation<TArgs>(
  cmd: string,
  args?: (a: TArgs) => Record<string, unknown>,
) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (a: TArgs) =>
      invoke(cmd, args ? args(a) : (a as Record<string, unknown>)),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
    },
  });
}

export function useSecureCreateWallet() {
  return useWalletMutation<{ label: string; network: string }>("secure_create_wallet");
}

export function useSecureImportWallet() {
  return useWalletMutation<{ label: string; network: string; kind: string }>(
    "secure_import_wallet",
  );
}

export function useRevealBackupPhrase() {
  return useMutation({
    mutationFn: (walletProfileId: string) =>
      invoke("secure_reveal_backup_phrase", { walletProfileId }),
  });
}

export function useUnlockSigner() {
  return useWalletMutation<string>("unlock_local_signer", (walletProfileId) => ({
    walletProfileId,
  }));
}

export function useLockSigner() {
  return useWalletMutation<void>("lock_local_signer", () => ({}));
}

export function useSetActiveProfile() {
  return useWalletMutation<string>("set_active_wallet_profile", (walletProfileId) => ({
    walletProfileId,
  }));
}

export function useDeleteProfile() {
  return useWalletMutation<string>("delete_wallet_profile", (walletProfileId) => ({
    walletProfileId,
  }));
}

export function useSyncWalletState() {
  return useWalletMutation<string | undefined>("sync_wallet_state", (walletProfileId) => ({
    walletProfileId: walletProfileId ?? null,
  }));
}

/// Node-free discovery of names the active wallet owns (crawls the explorer).
/// Persists results; invalidates the read queries so the owned-names list
/// repopulates.
export function useDiscoverOwnedNames() {
  return useWalletMutation<void>("discover_owned_names");
}

export function useBuildSendDraft() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (a: { toAddress: string; valueDoos: number; feeRate?: number }) =>
      invoke<TxDraftSummary>("build_send_hns_draft", {
        toAddress: a.toAddress,
        valueDoos: a.valueDoos,
        feeRate: a.feeRate ?? null,
      }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["wallet"] }),
  });
}

export function useSignTxDraft() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (draftId: string) => invoke<TxDraftSummary>("sign_tx_draft", { draftId }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["wallet"] }),
  });
}

export function useBroadcastTxDraft() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (draftId: string) => invoke<BroadcastResult>("broadcast_tx_draft", { draftId }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["wallet"] }),
  });
}

/** Build a covenant/name-action draft via one of the `build_*_draft` commands. */
export function useNameAction(command: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: Record<string, unknown>) =>
      invoke<TxDraftSummary>(command, args),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["wallet"] }),
  });
}

export function useBidCommitments() {
  return useQuery({
    queryKey: ["wallet", "bids"],
    queryFn: () => invoke<unknown[]>("list_tx_drafts"), // drafts include bid actions
    retry: false,
  });
}

/**
 * Execute a built draft end-to-end: unlock the signer (if locked), sign, then
 * broadcast. Returns an async runner; the caller passes the draft id, the
 * active profile id, and whether the signer is already unlocked.
 */
export function useExecuteDraft() {
  const unlock = useUnlockSigner();
  const sign = useSignTxDraft();
  const broadcast = useBroadcastTxDraft();
  const run = async (
    draftId: string,
    profileId: string,
    unlocked: boolean,
  ): Promise<BroadcastResult> => {
    if (!unlocked) {
      await unlock.mutateAsync(profileId);
    }
    await sign.mutateAsync(draftId);
    return broadcast.mutateAsync(draftId);
  };
  return { run, pending: unlock.isPending || sign.isPending || broadcast.isPending };
}
