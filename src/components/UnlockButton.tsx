import { LockOpen, type LucideIcon } from "lucide-react";
import { Icon } from "./ui/Icon";
import { cn } from "../lib/utils";
import { isSignerUnlockedFor } from "../lib/signer";
import { Button } from "./ui/Button";
import { useActiveProfile, useSignerSession } from "../queries/wallet";
import { useSignerToggle } from "../hooks/useSignerToggle";

type Props = {
  size?: "sm" | "md";
  variant?: "primary" | "secondary" | "danger" | "ghost";
  className?: string;
  label?: string;
  /** Icon before the label; null for none. */
  icon?: LucideIcon | null;
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
  // Secondary: Unlock sits next to (or above) the page's or dialog's own
  // primary, which it only unblocks (DESIGN_SYSTEM §1.1).
  variant = "secondary",
  className,
  label = "Unlock",
  icon = LockOpen,
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
      className={cn("inline-flex items-center gap-1.5", className)}
      onClick={async () => {
        if (await unlock()) onUnlocked?.();
      }}
      disabled={busy === "unlocking"}
      data-testid="unlock-now"
    >
      {icon && <Icon icon={icon} size={14} />}
      {busy === "unlocking" ? "Unlocking…" : label}
    </Button>
  );
}
