import type { NameActionCapabilities } from "../types";
import type { NameTab } from "./nameRoute";

/** The single-name actions the Names row menu runs in place (BatchConfirmModal, one name). */
export type RowMenuAction = "renew" | "reveal" | "redeem" | "finalize";

export interface RowMenuItem {
  label: string;
  do:
    { kind: "open"; tab?: NameTab } | { kind: "run"; action: RowMenuAction } | { kind: "editTags" };
}

const RUN: { action: RowMenuAction; label: string; cap: keyof NameActionCapabilities }[] = [
  { action: "renew", label: "Renew", cap: "canRenew" },
  { action: "reveal", label: "Reveal", cap: "canReveal" },
  { action: "redeem", label: "Redeem", cap: "canRedeem" },
  { action: "finalize", label: "Finalize", cap: "canFinalize" },
];

/**
 * The ⋯ menu of a Names row. One-click actions the name allows right now run
 * in place; those that need input open the tab of the name page that holds
 * them. Revoke is never here: it burns the name, so it stays on the name page
 * behind its own confirmation. Watching is the row's own star column, so the
 * menu only edits a watched name's tags. A watch-only wallet signs nothing,
 * so it gets no actions. Without capabilities (not fetched yet) no action is
 * offered.
 */
export function nameRowMenu({
  caps,
  watched,
  watchOnly,
}: {
  caps: NameActionCapabilities | undefined;
  watched: boolean;
  watchOnly: boolean;
}): RowMenuItem[] {
  const items: RowMenuItem[] = [{ label: "Open name", do: { kind: "open" } }];
  if (caps && !watchOnly) {
    const allowed = (k: keyof NameActionCapabilities) =>
      (caps[k] as { allowed?: boolean } | undefined)?.allowed === true;
    for (const r of RUN) {
      if (allowed(r.cap)) items.push({ label: r.label, do: { kind: "run", action: r.action } });
    }
    if (allowed("canCancelTransfer")) {
      // The cancel lives on the transfer card of the name's Overview.
      items.push({ label: "Cancel transfer…", do: { kind: "open" } });
    }
    if (allowed("canTransfer")) {
      items.push({ label: "Transfer or sell…", do: { kind: "open", tab: "transfer" } });
    }
    if (allowed("canUpdate")) {
      items.push({ label: "Edit DNS records", do: { kind: "open", tab: "dns" } });
    }
    if (caps.ownsName && caps.nameIsRegistered) {
      items.push({ label: "Sign message", do: { kind: "open", tab: "sign" } });
    }
  }
  if (watched) items.push({ label: "Edit tags", do: { kind: "editTags" } });
  return items;
}
