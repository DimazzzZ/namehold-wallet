import { useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useNameRecords } from "../../queries/read";
import { Badge } from "../ui/Badge";
import { Icon } from "../ui/Icon";
import { Tooltip } from "../ui/Tooltip";
import { formatHns, formatHnsShort } from "../../lib/utils";
import { auctionPhase, formatCountdown, type PhaseCountdown } from "../../lib/auction";
import { approxBlocksDate, countdownForNetwork } from "../../lib/nameStatus";
import { renderRecord } from "./ReadOnlyDnsRecords";
import type { HsdName, NameActionCapabilities, WalletNetwork } from "../../types";

/** How many records the Details card previews before "Edit records". */
const RECORDS_PREVIEW = 4;

interface NameDetailsProps {
  name: string;
  profileId: string | null;
  info: HsdName | null | undefined;
  caps?: NameActionCapabilities | null;
  network?: WalletNetwork | null;
  countdown?: PhaseCountdown | null;
  /** Our own bid is known locally and worth showing (pre-reveal). */
  showLocalBid?: boolean;
  /** The editable records section is live (owned, registered). */
  recordsLive?: boolean;
  /** Go to the DNS records tab. */
  onOpenRecords?: () => void;
}

function Row({
  label,
  value,
  sub,
  testId,
}: {
  label: ReactNode;
  value: ReactNode;
  sub?: ReactNode;
  testId?: string;
}) {
  return (
    <div className="flex justify-between gap-3 text-[13px]" data-testid={testId}>
      <span className="text-gray-500">{label}</span>
      <span className="text-right text-gray-900 tabular-nums">
        {value}
        {sub && <span className="text-gray-400">{sub}</span>}
      </span>
    </div>
  );
}

/** A display amount: short, the exact value on hover (DESIGN_SYSTEM §2). */
function Hns({ doos, testId }: { doos: number; testId?: string }) {
  return (
    <Tooltip content={`${formatHns(doos)} HNS`} className="inline">
      <span data-testid={testId}>{formatHnsShort(doos)} HNS</span>
    </Tooltip>
  );
}

/**
 * The name page's Details card: what is known on-chain about the name (bids,
 * price, expiry, heights), a preview of its records when it is ours, and the
 * owner UTXO behind a Technical disclosure.
 */
export function NameDetails({
  name,
  profileId,
  info,
  caps,
  network,
  countdown,
  showLocalBid,
  recordsLive,
  onOpenRecords,
}: NameDetailsProps) {
  const [technical, setTechnical] = useState(false);
  const owned = !!caps?.ownsName && (caps.nameIsRegistered || caps.transferPending);
  const { data: resource } = useNameRecords(owned ? name : null, profileId);

  if (!info) return null;

  const { phase } = auctionPhase(info.state);
  const closed = phase === "CLOSED";
  const blocksLeft = info.stats?.blocksUntilExpire ?? null;
  const exp = blocksLeft != null ? approxBlocksDate(blocksLeft, network) : null;
  const blocks = (n: number) => `${n.toLocaleString("en-US")} blocks`;
  const multi = (caps?.myBidCount ?? 0) > 1;

  const rows: ReactNode[] = [];
  // Bought on the market: that price is what this wallet paid. The auction
  // figures below are the name's original auction, so they say so.
  const bought = info.purchase ?? null;
  if (bought) {
    rows.push(
      <Row
        key="bought"
        testId="name-market-purchase"
        label="Bought on the market"
        value={<Hns doos={bought.priceDoos} />}
        sub={
          bought.height != null ? ` · block ${bought.height.toLocaleString("en-US")}` : undefined
        }
      />,
    );
  }
  if (showLocalBid && caps?.bidValueDoos != null) {
    rows.push(
      <Row
        key="bid"
        testId="name-your-bid"
        label={
          <Tooltip
            hint
            content="Your bid is sealed until reveal; the lockup is the most anyone can see."
          >
            {multi ? "Latest bid" : "Your bid"}
          </Tooltip>
        }
        value={<Hns doos={caps.bidValueDoos} />}
        sub={
          <>
            {caps.lockupValueDoos != null && (
              <span data-testid="name-your-lockup">
                {" · lockup "}
                <Hns doos={caps.lockupValueDoos} />
              </span>
            )}
            {multi && (
              <span data-testid="name-your-bid-count">{` · ${caps.myBidCount} of yours`}</span>
            )}
          </>
        }
      />,
    );
  }
  // Our own bid, when known, says more than the on-chain figure. Bids are
  // sealed while bidding, and a 0 means none revealed: never "0 HNS".
  if (!showLocalBid && phase === "BIDDING") {
    rows.push(
      <Row
        key="high"
        testId="name-high-bid"
        label="High bid"
        value={<span className="text-gray-500">hidden until reveal</span>}
      />,
    );
  } else if (!showLocalBid && (info.highest ?? 0) > 0) {
    rows.push(
      <Row
        key="high"
        testId="name-high-bid"
        label={bought ? "Auction winner's bid" : "High bid"}
        value={<Hns doos={info.highest!} />}
      />,
    );
  }
  // A claimed (reserved) name never had an auction: no bid, no price to show.
  if (closed && info.value != null && (info.value > 0 || (info.highest ?? 0) > 0)) {
    rows.push(
      <Row
        key="paid"
        label={
          <Tooltip
            hint
            content="Handshake uses a Vickrey second-price auction: the winner pays the second-highest bid, not their own bid."
          >
            {bought ? "Auction price (2nd-price)" : "Paid price (2nd-price)"}
          </Tooltip>
        }
        value={
          info.value > 0 ? (
            <Hns doos={info.value} />
          ) : (
            // One bid, so no second price: the winner paid nothing, never "0 HNS".
            <span className="text-gray-500">nothing — no second bid</span>
          )
        }
      />,
    );
  }
  if (closed && blocksLeft != null) {
    rows.push(
      exp ? (
        <Row key="exp" label="Expires" value={exp.short} sub={` · in ${blocks(blocksLeft)}`} />
      ) : (
        <Row key="exp" label="Expires" value={`in ${blocks(blocksLeft)}`} />
      ),
    );
  } else if (countdown) {
    rows.push(
      <Row
        key="next"
        testId="name-countdown"
        label="Next step"
        value={`${countdown.label} ${formatCountdown(countdownForNetwork(countdown, network))}`}
      />,
    );
  }
  if (info.renewal != null) {
    rows.push(<Row key="renewed" label="Renewed" value={`block ${info.renewal}`} />);
  }
  if (info.height != null) {
    rows.push(
      <Row key="opened" label="Opened" value={`block ${info.height.toLocaleString("en-US")}`} />,
    );
  }

  const records = resource?.records ?? [];
  const recordsLink = !recordsLive ? "View" : records.length ? "Edit records" : "Add records";

  return (
    <section
      className="bg-white border border-gray-200 rounded-lg p-4 flex flex-col gap-3"
      data-testid="name-details"
    >
      <h3 className="text-sm font-semibold text-gray-900">Details</h3>
      {rows.length ? (
        <div className="flex flex-col gap-1.5">{rows}</div>
      ) : (
        <div className="text-[13px] text-gray-400">Nothing on-chain yet.</div>
      )}

      {owned && (
        <div className="border-t border-gray-100 pt-3 flex flex-col gap-1.5">
          <div className="flex items-center justify-between text-[13px]">
            <span className="text-gray-500">Records</span>
            <button
              type="button"
              className="text-blue-600 hover:underline"
              onClick={onOpenRecords}
              data-testid="details-records-link"
            >
              {recordsLink}
            </button>
          </div>
          {records.slice(0, RECORDS_PREVIEW).map((rec, i) => {
            const { label, value } = renderRecord(rec);
            return (
              <div key={i} className="flex items-center gap-2 text-xs min-w-0">
                <Badge variant="default">{label}</Badge>
                <span className="font-mono text-gray-700 truncate">{value || "—"}</span>
              </div>
            );
          })}
        </div>
      )}

      {info.owner && (
        <div className="border-t border-gray-100 pt-3 text-xs">
          <button
            type="button"
            className="inline-flex items-center gap-1 text-gray-500 hover:text-gray-900"
            onClick={() => setTechnical((t) => !t)}
            aria-expanded={technical}
          >
            <Icon icon={technical ? ChevronDown : ChevronRight} size={13} />
            Technical
          </button>
          {technical && (
            <div className="mt-2 flex flex-col gap-1">
              <div className="text-gray-500">Owner UTXO</div>
              <div className="font-mono break-all text-gray-900">
                {info.owner.hash}:{info.owner.index}
              </div>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
