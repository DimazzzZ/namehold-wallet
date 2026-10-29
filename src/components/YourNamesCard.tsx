import { ArrowRight } from "lucide-react";
import { useNavigate } from "react-router-dom";
import {
  auctionPhase,
  pendingBroadcastBadge,
  taskStateUrgencyRank,
  taskSummaryFromCapabilities,
} from "../lib/auction";
import { displayName } from "../lib/idn";
import { needsYouWhen, type NeedsYouItem } from "../lib/needsYou";
import { fromInteractiveChild } from "../lib/rowClick";
import { formatHnsShort } from "../lib/utils";
import { useReadNames, useReadRenewals } from "../queries/read";
import type { HsdName, NameActionCapabilities, WalletNetwork } from "../types";
import { listingStateLabel } from "./market/marketText";
import { Icon } from "./ui/Icon";
import { Tooltip } from "./ui/Tooltip";

/** After the Needs you order and the urgency rank, these come before the rest. */
const NEXT_UP = ["readyToBid", "revealBroadcastPending", "waitingForBidding"];

/** The Status cell: what the name needs, then a muted suffix. */
function status(
  n: HsdName,
  c: NameActionCapabilities | undefined,
  network: WalletNetwork,
): { main: string; suffix: string | null } {
  if (n.listing) {
    return {
      main: listingStateLabel(n.listing.state, n.listing.blocksUntilFinalize),
      suffix: null,
    };
  }
  const pending = pendingBroadcastBadge(c?.pendingBroadcastAction);
  if (pending) return { main: pending, suffix: null };
  const when =
    c?.countdownBlocks != null ? needsYouWhen(c.countdownBlocks, c.countdownHours, network) : null;
  switch (c?.taskState) {
    case "readyToReveal":
      return { main: "Reveal your bid", suffix: when };
    case "expiringSoon":
      return { main: "Renew soon", suffix: null };
    case "transferPendingFinalize":
      return { main: "Transfer ready", suffix: "finalize it" };
    case "wonNeedsRegister":
      return { main: "Won", suffix: "register it" };
    case "lostNeedsRedeem":
      return { main: "Lost", suffix: "reclaim the lockup" };
    case "readyToBid":
      return c.hasBidCommitment
        ? { main: "Bid placed", suffix: when && `reveal in ${when}` }
        : { main: "Bidding", suffix: null };
    case "waitingForBidding":
      return { main: "Opening", suffix: "bidding starts soon" };
    default:
      return {
        main: taskSummaryFromCapabilities(c)?.label ?? auctionPhase(n.state).label,
        suffix: null,
      };
  }
}

/**
 * Wallet's short list of the names most worth a look, and the way into the
 * Names page. Purchases on their way are left out: they are not ours yet.
 */
export function YourNamesCard({
  capsByName,
  needs,
  network,
  onOpenName,
}: {
  capsByName: Map<string, NameActionCapabilities>;
  needs: NeedsYouItem[];
  network: WalletNetwork;
  onOpenName: (name: string) => void;
}) {
  const navigate = useNavigate();
  const { data: names = [] } = useReadNames();
  const { data: renewals } = useReadRenewals();
  const renewalBy = new Map((renewals?.names ?? []).map((r) => [r.name, r]));
  const needIndex = (name: string) => {
    const i = needs.findIndex((x) => x.name === name);
    return i < 0 ? Number.POSITIVE_INFINITY : i;
  };
  const rank = (name: string) => {
    const t = capsByName.get(name)?.taskState;
    if (!t) return 100;
    const next = NEXT_UP.indexOf(t);
    return next >= 0 ? 10 + next : taskStateUrgencyRank(t);
  };
  const rows = names
    .filter((n) => !n.shakedex)
    .sort(
      (a, b) =>
        needIndex(a.name) - needIndex(b.name) ||
        rank(a.name) - rank(b.name) ||
        a.name.localeCompare(b.name),
    )
    .slice(0, 5);

  return (
    <section className="rounded-md border border-gray-200 bg-white" data-testid="your-names">
      <div className="px-4 py-3 border-b border-gray-100 flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold text-gray-700">Your names</h3>
        <button
          type="button"
          className="text-xs text-blue-600 hover:underline inline-flex items-center gap-1"
          onClick={() => navigate("/names?filter=owned")}
          data-testid="your-names-all"
        >
          All {names.length} names <Icon icon={ArrowRight} size={12} />
        </button>
      </div>
      {rows.length === 0 ? (
        <div className="py-4 text-center text-sm text-gray-500">
          No owned names found yet. Click Sync to scan for names this wallet owns.
        </div>
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-gray-500 border-b">
              <th className="py-1 pr-4 pl-4">Name</th>
              <th className="py-1 pr-4">Status</th>
              <th className="py-1 pr-4 text-right">Bid / value, HNS</th>
              <th className="py-1">Expires</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((n) => {
              const c = capsByName.get(n.name);
              const st = status(n, c, network);
              const { phase } = auctionPhase(c?.phase ?? n.state);
              const bidding = phase === "OPENING" || phase === "BIDDING" || phase === "REVEAL";
              const r = renewalBy.get(n.name);
              const expires =
                r && (r.blocksUntilExpire != null || r.daysUntilExpire != null)
                  ? needsYouWhen(
                      r.blocksUntilExpire,
                      r.daysUntilExpire == null ? null : r.daysUntilExpire * 24,
                      network,
                    )
                  : null;
              return (
                <tr
                  key={n.name}
                  className="border-t border-gray-100 hover:bg-gray-50 cursor-pointer"
                  onClick={(e) => {
                    if (!fromInteractiveChild(e)) onOpenName(n.name);
                  }}
                  data-testid="your-names-row"
                >
                  <td className="py-1 pr-4 pl-4">
                    <Tooltip content="Open the name page">
                      <button
                        type="button"
                        className="font-mono text-[13px] text-gray-900 hover:text-blue-700 hover:underline"
                        onClick={() => onOpenName(n.name)}
                      >
                        .{displayName(n.name)}
                      </button>
                    </Tooltip>
                  </td>
                  <td className="py-1 pr-4">
                    {st.main}
                    {st.suffix && <span className="text-gray-500"> · {st.suffix}</span>}
                  </td>
                  <td className="py-1 pr-4 text-right tabular-nums whitespace-nowrap">
                    {bidding && c?.bidValueDoos != null ? (
                      <>
                        {formatHnsShort(c.bidValueDoos)} <span className="text-gray-500">bid</span>
                      </>
                    ) : n.value != null ? (
                      formatHnsShort(n.value)
                    ) : (
                      <span className="text-gray-500">—</span>
                    )}
                  </td>
                  <td
                    className={`py-1 text-xs tabular-nums whitespace-nowrap ${
                      r?.expiringSoon ? "text-red-600" : "text-gray-500"
                    }`}
                  >
                    {expires ?? "—"}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </section>
  );
}
