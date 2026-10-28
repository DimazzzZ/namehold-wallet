import { OwnershipActions } from "../name-actions/OwnershipActions";
import { approxBlocksDate } from "../../lib/nameStatus";
import { TAB_GRID } from "./OverviewTab";
import type { NameActions } from "./useNameActions";

const card = "bg-white border border-gray-200 rounded-lg p-4 text-[13px] text-gray-700";

/** The Renew card's sentence: a date only where blocks keep time. */
export function renewText(blocksLeft: number | null | undefined, network: string | undefined) {
  const base = "Extends the name's registration from the renewal block.";
  if (blocksLeft == null) return base;
  const blocks = `${blocksLeft.toLocaleString("en-US")} blocks`;
  const exp = approxBlocksDate(blocksLeft, network);
  return exp
    ? `${base} It currently expires ${exp.short} (${blocks}).`
    : `${base} It currently expires in ${blocks}.`;
}

/** Transfer & sell: transfer, sell, renew and revoke for a name this wallet owns. */
export function TransferTab({ a, sellRefusal }: { a: NameActions; sellRefusal: string | null }) {
  const { name, caps, info, sections, busy, run, build } = a;

  const left = (() => {
    if (sections.ownership.kind === "upcoming") {
      return (
        <div className={card}>
          <b>Transfer, sell, renew and revoke</b> — available after you register this name.
        </div>
      );
    }
    if (sections.ownership.kind !== "live") {
      return (
        <div className={card}>
          {caps?.pendingBroadcastAction
            ? "Actions come back once the sent transaction is mined."
            : "Only the name's owner can transfer, sell, renew or revoke it. Watch it to hear when it becomes available."}
        </div>
      );
    }
    return (
      <OwnershipActions
        name={name}
        caps={caps}
        busy={busy}
        recipient={a.recipient}
        onRecipientChange={a.setRecipient}
        actionDisabled={a.actionDisabled}
        actionReason={a.actionReason}
        onTransfer={() =>
          run("TRANSFER", () => build.transfer.mutateAsync({ name, recipient: a.recipient.trim() }))
        }
        onFinalize={() => run("FINALIZE", () => build.finalize.mutateAsync({ name }))}
        onCancelTransfer={() => run("CANCEL", () => build.cancel.mutateAsync({ name }))}
        onRenew={() => run("RENEW", () => build.renew.mutateAsync({ name }))}
        onRevoke={() => run("REVOKE", () => build.revoke.mutateAsync({ name }))}
        onSell={() => a.setSelling(true)}
        sellRefusal={sellRefusal}
        transferPending={!!caps?.transferPending}
        renewText={renewText(info?.stats?.blocksUntilExpire, a.profile?.network)}
        revokeOpen={a.revokeOpen}
        onRevokeOpenChange={a.setRevokeOpen}
      />
    );
  })();

  const rightNow = !caps?.ownsName
    ? "Nothing to manage here: this wallet doesn't own the name."
    : caps.transferPending
      ? `A transfer is in progress (started at height ${info?.transfer ?? "?"}). Finalize it after the lockup, or cancel it.`
      : caps.lockedForSale
        ? "The name is locked for sale on Shakedex — manage the listing in Market › My listings."
        : "No transfer or listing in progress.";

  return (
    <div className={TAB_GRID}>
      <div className="flex flex-col gap-4 min-w-0">{left}</div>
      <aside className={`${card} flex flex-col gap-2`}>
        <h3 className="text-sm font-semibold text-gray-900">Right now</h3>
        <p data-testid="transfer-right-now">{rightNow}</p>
      </aside>
    </div>
  );
}
