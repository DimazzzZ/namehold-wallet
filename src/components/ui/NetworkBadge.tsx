import { netTheme } from "../../lib/networkTheme";
import { cn } from "../../lib/utils";

/** A wallet's network as a dot + label in the network's own colour. */
export function NetworkBadge({ network, className }: { network: string; className?: string }) {
  const t = netTheme(network);
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded border px-1.5 py-px text-[11px] font-medium",
        t.badge,
        className,
      )}
      data-testid="network-badge"
    >
      <span className={cn("size-1.5 rounded-full", t.dot)} aria-hidden />
      {t.label}
    </span>
  );
}
