import { useState } from "react";
import { isSignerUnlockedFor } from "../lib/signer";
import { mapError } from "../lib/errors";
import {
  useActiveProfile,
  useLockSigner,
  useSignerSession,
  useUnlockSigner,
} from "../queries/wallet";
import { useUiStore } from "../stores/ui";

/**
 * Lock / unlock the active hot wallet's signer, with the app's toasts. A
 * Ledger signs on the device and a watch-only wallet can't sign: neither has
 * a signer to lock (`canLock` false).
 */
export function useSignerToggle(): {
  canLock: boolean;
  unlocked: boolean;
  hasPassphrase: boolean;
  busy: "unlocking" | "locking" | null;
  /** True once unlocked; false when it failed or was refused. */
  unlock: () => Promise<boolean>;
  lock: () => Promise<void>;
  toggle: () => Promise<unknown>;
} {
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const unlockM = useUnlockSigner();
  const lockM = useLockSigner();
  const showToast = useUiStore((s) => s.showToast);
  const [busy, setBusy] = useState<"unlocking" | "locking" | null>(null);

  const canLock = !!profile && !profile.watchOnly && profile.kind !== "ledger_hardware";
  const unlocked = isSignerUnlockedFor(signer, profile?.id);

  const unlock = async () => {
    if (!profile) return false;
    setBusy("unlocking");
    try {
      await unlockM.mutateAsync(profile.id);
      showToast("Wallet unlocked", "success");
      return true;
    } catch (e) {
      showToast(mapError(e), "error");
      return false;
    } finally {
      setBusy(null);
    }
  };

  const lock = async () => {
    setBusy("locking");
    try {
      await lockM.mutateAsync();
      showToast("Wallet locked", "info");
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setBusy(null);
    }
  };

  return {
    canLock,
    unlocked,
    hasPassphrase: profile?.hasPassphrase ?? false,
    busy: busy ?? (unlockM.isPending ? "unlocking" : lockM.isPending ? "locking" : null),
    unlock,
    lock,
    toggle: () => (unlocked ? lock() : unlock()),
  };
}
