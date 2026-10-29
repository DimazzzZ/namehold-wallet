import { FlaskConical } from "lucide-react";
import { netTheme } from "../../lib/networkTheme";
import { cn } from "../../lib/utils";
import { useActiveProfile } from "../../queries/wallet";
import { useUiStore } from "../../stores/ui";
import { Icon } from "../ui/Icon";

/** A full-width band naming a test network, so it is never mistaken for mainnet. */
export function NetworkStrip() {
  const { data: profile } = useActiveProfile();
  const setWalletMenuOpen = useUiStore((s) => s.setWalletMenuOpen);
  const t = netTheme(profile?.network);
  if (!profile || !t.strip) return null;
  return (
    <div
      role="status"
      data-testid="network-strip"
      className={cn("flex items-center gap-2.5 px-6 py-1.5 text-xs", t.strip)}
    >
      <Icon icon={FlaskConical} size={14} />
      <strong className="uppercase tracking-[.06em]">{t.label}</strong>
      <span className="opacity-90">{t.note}</span>
      <button
        type="button"
        className="ml-auto rounded border border-current px-2 py-0.5 text-xs"
        // The switcher's own outside-click would close the menu this opens.
        onMouseDown={(e) => e.stopPropagation()}
        onClick={() => setWalletMenuOpen(true)}
      >
        Switch wallet
      </button>
    </div>
  );
}
