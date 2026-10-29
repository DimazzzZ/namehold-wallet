import { isSignerUnlockedFor } from "../lib/signer";
import { Button } from "./ui/Button";
import { useActiveProfile, useSignerSession } from "../queries/wallet";
import { useSignerToggle } from "../hooks/useSignerToggle";

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
  const { unlock, busy } = useSignerToggle();

  // Nothing to unlock, or already unlocked → render nothing.
  if (!profile || isSignerUnlockedFor(signer, profile.id)) return null;

  return (
    <Button
      size={size}
      variant={variant}
      className={className}
      onClick={async () => {
        if (await unlock()) onUnlocked?.();
      }}
      disabled={busy === "unlocking"}
      data-testid="unlock-now"
    >
      {busy === "unlocking" ? "Unlocking…" : label}
    </Button>
  );
}
