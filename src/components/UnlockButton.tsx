import { Button } from "./ui/Button";
import { useActiveProfile, useSignerSession, useUnlockSigner } from "../queries/wallet";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";

type Props = {
  size?: "sm" | "md";
  variant?: "primary" | "secondary" | "danger" | "ghost";
  className?: string;
  label?: string;
  onUnlocked?: () => void;
};

/**
 * A self-hiding unlock button for locked-wallet notices.
 * Renders nothing if there's no active profile or the wallet is already unlocked.
 * When clicked, triggers the unlock flow (passphrase prompt in secure window, or sync unlock).
 * Toasts on success/error.
 */
export function UnlockButton({
  size = "sm",
  variant = "primary",
  className,
  label = "Unlock",
  onUnlocked,
}: Props) {
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const unlock = useUnlockSigner();
  const showToast = useUiStore((s) => s.showToast);

  // Nothing to unlock, or already unlocked → render nothing.
  if (!profile || signer?.unlocked) return null;

  const handleUnlock = async () => {
    try {
      await unlock.mutateAsync(profile.id);
      showToast("Wallet unlocked", "success");
      onUnlocked?.();
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  return (
    <Button
      size={size}
      variant={variant}
      className={className}
      onClick={handleUnlock}
      disabled={unlock.isPending}
      data-testid="unlock-now"
    >
      {unlock.isPending ? "Unlocking…" : label}
    </Button>
  );
}
