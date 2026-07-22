import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { StagedError } from "../lib/errors";
import type {
  WalletProfileSummary,
  SignerSessionSummary,
  WriteCapability,
  WalletBalances,
  TxDraftSummary,
  BroadcastResult,
  NameSignature,
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

/**
 * Per-wallet spendable balance (from the node-synced chain cache). Keyed by the
 * active profile id (no cross-wallet bleed) and never auto-refetched: each wallet
 * shows its own last-known value — persisted server-side, so it survives a
 * restart — and only changes on Refresh (which invalidates the `["wallet"]` prefix).
 */
export function useWalletBalances() {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery({
    queryKey: ["wallet", "balances", profileId],
    enabled: profileId != null,
    queryFn: () =>
      invoke<WalletBalances>("get_wallet_balances", { walletProfileId: profileId }),
    staleTime: Infinity,
    gcTime: Infinity,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: false,
  });
}

export function useTxDrafts() {
  return useQuery({
    queryKey: ["wallet", "drafts"],
    queryFn: async () => {
      // Advance any broadcast drafts pending→confirmed/dropped before listing.
      // Best-effort and node-free-safe: when the node is unreachable (or there
      // are no broadcast drafts) this is a fast no-op and statuses are unchanged.
      try {
        await invoke("refresh_tx_confirmations", { walletProfileId: null });
      } catch {
        /* ignore — a node blip must never break the drafts list */
      }
      return invoke<TxDraftSummary[]>("list_tx_drafts");
    },
    // Poll so a broadcast tx visibly settles to Confirmed (or Not confirmed)
    // without the user hitting Refresh.
    refetchInterval: 15_000,
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

/** Repair owned-name cache: reconcile inventory + tracked names against live explorer data. */
export function useRepairOwnedNames() {
  return useWalletMutation<void>("repair_owned_names");
}

export function useBuildSendDraft() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (a: { toAddress: string; valueDoos: number; feeRate?: number; max?: boolean }) =>
      invoke<TxDraftSummary>("build_send_hns_draft", {
        toAddress: a.toAddress,
        valueDoos: a.max ? 0 : a.valueDoos,
        feeRate: a.feeRate ?? null,
        max: a.max ?? false,
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

/** True for the "Wallet locked" / "wallet is locked" `AppError` text (see
 *  `error.rs::AppError::WalletLocked` and its `sign_tx_draft`-style profile
 *  -mismatch message, which is deliberately NOT matched here — only the
 *  literal locked-signer error should trigger an automatic unlock+retry). */
function isWalletLockedError(e: unknown): boolean {
  return /wallet (is )?locked/i.test(String(e));
}

/**
 * Sign an arbitrary message with the wallet key that owns a name (hsd
 * `signmessagewithname` parity — see `sign_name_message`), for domain-claim
 * verification flows such as Namebase's. Not a spend: no draft, no broadcast.
 *
 * If the signer is locked, transparently unlocks (for `walletProfileId`) and
 * retries the sign once — mirrors `useExecuteDraft`'s unlock-if-needed step,
 * minus the broadcast leg this command has no use for.
 */
export function useSignNameMessage() {
  const unlock = useUnlockSigner();
  const sign = useMutation({
    mutationFn: (a: { name: string; message: string; walletProfileId: string | null }) =>
      invoke<NameSignature>("sign_name_message", {
        name: a.name,
        message: a.message,
        walletProfileId: a.walletProfileId,
      }),
  });

  const run = async (
    name: string,
    message: string,
    walletProfileId: string | null,
  ): Promise<NameSignature> => {
    try {
      return await sign.mutateAsync({ name, message, walletProfileId });
    } catch (e) {
      if (!isWalletLockedError(e)) throw e;
      // Locked — unlock this wallet's signer, then retry the sign once.
      await unlock.mutateAsync(walletProfileId ?? "");
      return await sign.mutateAsync({ name, message, walletProfileId });
    }
  };

  return { run, pending: unlock.isPending || sign.isPending, unlock, sign };
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
 * Export every bid commitment (name, value, blind, nonce, address, txids) for
 * a profile as a JSON string, for the user to save as a local backup file.
 * Contains secret material — callers must warn the user to store it alongside
 * their seed.
 */
export function useExportBidCommitments() {
  return useMutation({
    mutationFn: (walletProfileId: string | null) =>
      invoke<string>("export_bid_commitments", { walletProfileId }),
  });
}

/**
 * Execute a built draft end-to-end: unlock the signer (if locked), sign, then
 * broadcast. Returns an async runner; the caller passes the draft id, the
 * active profile id, and whether the signer is already unlocked.
 *
 * A rejection from `run()` is a `StagedError` tagging which leg threw
 * ("sign" covers both the unlock-if-needed step and the sign step, since
 * unlocking only exists to get to signing; "broadcast" is the final leg).
 * `StagedError#toString()` delegates to the original error, so any existing
 * caller that just does `mapError(e)` / `String(e)` on the rejection — with
 * no idea `StagedError` exists — keeps seeing byte-identical output.
 * `unlock`/`sign`/`broadcast` are also returned for callers that need the
 * raw mutation objects.
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
      try {
        await unlock.mutateAsync(profileId);
      } catch (e) {
        throw new StagedError("sign", e);
      }
    }
    try {
      await sign.mutateAsync(draftId);
    } catch (e) {
      throw new StagedError("sign", e);
    }
    try {
      return await broadcast.mutateAsync(draftId);
    } catch (e) {
      throw new StagedError("broadcast", e);
    }
  };
  return {
    run,
    pending: unlock.isPending || sign.isPending || broadcast.isPending,
    unlock,
    sign,
    broadcast,
  };
}
