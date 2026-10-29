import { NameSignMessage } from "../name-actions/NameSignMessage";
import { TAB_GRID } from "./OverviewTab";
import type { NameActions } from "./useNameActions";

const card = "bg-white border border-gray-200 rounded-lg p-4 text-[13px] text-gray-700";

/** Why signing isn't available, the first reason that applies; null when it is. */
function unavailable(a: NameActions): string | null {
  const { caps, sections } = a;
  const live = sections.ownership.kind === "live";
  if (live && caps?.hasOwnerCoin) return null;
  if (caps?.pendingBroadcastAction) return "Actions come back once the sent transaction is mined.";
  if (sections.ownership.kind === "upcoming") {
    return "Sign message — available after you register this name.";
  }
  if (live && caps?.lockedForSale) {
    return "Signing needs the owner coin, which is in the Shakedex lock right now.";
  }
  if (live) {
    return "Signing needs the owner coin, which hasn't synced locally yet. Connect a node and Refresh.";
  }
  return "Only the name's owner can sign with it.";
}

/** Sign message: prove control of the name with its owner key. */
export function SignTab({ a }: { a: NameActions }) {
  const reason = unavailable(a);
  return (
    <div className={TAB_GRID}>
      <div className={`${card} min-w-0`}>
        {reason ? (
          <p className="text-gray-500" data-testid="sign-unavailable">
            {reason}
          </p>
        ) : (
          <NameSignMessage name={a.name} profileId={a.profile?.id ?? null} caps={a.caps} />
        )}
      </div>
      <aside className={`${card} flex flex-col gap-2`}>
        <h3 className="text-sm font-semibold text-gray-900">What this proves</h3>
        <p>
          A signature made with this name&apos;s owner key shows you control the name, without
          sending anything on-chain.
        </p>
      </aside>
    </div>
  );
}
