import { Spinner } from "../ui/Spinner";
import { GatedButton } from "./GatedButton";
import { mapError } from "../../lib/errors";
import { cn, truncateMiddle } from "../../lib/utils";
import { btcText, hnsText, priceText, satsText, UNREADABLE } from "../../lib/exchangeAmount";
import {
  canCancel,
  exchangeTimeText,
  myCommitDeadline,
  needsMyCommit,
  tradeStatusText,
  type StatusTone,
} from "../../lib/exchangeStatus";
import { NO_TRADES, TERMS_NOT_AGREED } from "../../lib/exchangeText";
import type { TradeOutpoint, TradeRow, TradeTimeline } from "../../types/exchange";

const TONE: Record<StatusTone, string> = {
  action: "text-amber-800",
  waiting: "text-gray-900",
  success: "text-green-800",
  warning: "text-red-700",
  ended: "text-gray-600",
};

const TIMELINE: { key: keyof TradeTimeline; label: string }[] = [
  { key: "buyerFundDeadline", label: "Buyer locks BTC by" },
  { key: "sellerFundDeadline", label: "Seller locks HNS by" },
  { key: "revealDeadline", label: "Buyer claims HNS by" },
  { key: "hnsRefundTime", label: "HNS can go back to the seller from" },
  { key: "btcRefundTime", label: "BTC can go back to the buyer from" },
];

const WATCHTOWER: Record<TradeRow["watchtower"], string> = {
  none: "Not registered with a watchtower",
  registered: "Registered with the watchtower",
  failed: "Registering with the watchtower failed",
};

function timeOrDash(t: number): string {
  return exchangeTimeText(t) ?? UNREADABLE;
}

function outpointText(o: TradeOutpoint | null): string {
  return o === null ? "Not yet" : `${truncateMiddle(o.txid, 10, 8)}:${o.vout}`;
}

export function TradeCard({
  trade,
  refusal,
  committing,
  cancelling,
  onCommit,
  onCancel,
}: {
  trade: TradeRow;
  /** The gate's refusal for every action on the trade, or null. */
  refusal: string | null;
  committing: boolean;
  cancelling: boolean;
  onCommit: (trade: TradeRow) => void;
  onCancel: (trade: TradeRow) => void;
}) {
  const status = tradeStatusText(trade);
  const mine = needsMyCommit(trade);
  const deadline = myCommitDeadline(trade);
  const buying = trade.role === "buyer";
  return (
    <section
      data-testid="trade-card"
      data-trade-id={trade.tradeId}
      data-needs-commit={mine ? "true" : undefined}
      className={cn(
        "rounded-lg border bg-white p-4 shadow-sm",
        mine ? "border-amber-300 ring-1 ring-amber-200" : "border-gray-200",
      )}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-xs text-gray-500" data-testid="trade-role">
            {buying ? "You buy HNS" : "You sell HNS"} · {trade.maker ? "your offer" : "you took it"}
          </div>
          <div className="mt-0.5 text-sm font-semibold text-gray-900" data-testid="trade-amounts">
            {hnsText(trade.hnsAmount) ?? UNREADABLE} for {btcText(trade.btcSats) ?? UNREADABLE}
          </div>
          <div className="text-xs text-gray-500">
            {satsText(trade.btcSats) ?? UNREADABLE} · {priceText(trade.price) ?? UNREADABLE}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {canCancel(trade) && (
            <GatedButton
              size="sm"
              variant="ghost"
              data-testid={`trade-cancel-${trade.tradeId}`}
              refusal={refusal}
              loading={cancelling}
              onClick={() => onCancel(trade)}
            >
              Cancel
            </GatedButton>
          )}
          {mine && (
            <GatedButton
              size="sm"
              variant="primary"
              data-testid={`trade-commit-${trade.tradeId}`}
              refusal={refusal ?? (trade.negotiating ? TERMS_NOT_AGREED : null)}
              loading={committing}
              onClick={() => onCommit(trade)}
            >
              Confirm and sign
            </GatedButton>
          )}
        </div>
      </div>

      {mine && deadline !== null && (
        <p
          className="mt-3 rounded-md bg-amber-50 px-3 py-1.5 text-xs font-medium text-amber-900"
          data-testid="trade-commit-deadline"
        >
          Needs your confirmation before {timeOrDash(deadline)}
        </p>
      )}

      <div className="mt-3">
        <div
          className={cn("text-sm font-semibold", TONE[status.tone])}
          data-testid="trade-headline"
        >
          {status.headline}
        </div>
        <p className="mt-0.5 text-sm text-gray-700 [text-wrap:pretty]" data-testid="trade-sentence">
          {status.sentence}
        </p>
        {trade.negotiating && trade.declined === null && (
          <p className="mt-1 text-xs text-gray-500" data-testid="trade-negotiating">
            Agreeing on the terms with the other side's Namehold. Nothing is signed yet.
          </p>
        )}
        {trade.declined !== null && (
          <p className="mt-1 text-xs text-red-700" data-testid="trade-declined">
            The other side declined: {trade.declined}
          </p>
        )}
        {trade.lastError && (
          <p role="alert" className="mt-1 text-xs text-red-700" data-testid="trade-last-error">
            {trade.lastError}
          </p>
        )}
      </div>

      <dl className="mt-3 grid grid-cols-1 gap-x-6 gap-y-1 text-xs text-gray-600 sm:grid-cols-2">
        {trade.timeline &&
          TIMELINE.map((t) => (
            <div key={t.key} className="flex justify-between gap-3" data-testid={`trade-${t.key}`}>
              <dt className="text-gray-500">{t.label}</dt>
              <dd className="tabular-nums text-gray-800">{timeOrDash(trade.timeline![t.key])}</dd>
            </div>
          ))}
        <div className="flex justify-between gap-3">
          <dt className="text-gray-500">BTC lock</dt>
          <dd className="font-mono text-gray-800" data-testid="trade-btc-lock">
            {outpointText(trade.btcLock)}
          </dd>
        </div>
        <div className="flex justify-between gap-3">
          <dt className="text-gray-500">HNS lock</dt>
          <dd className="font-mono text-gray-800" data-testid="trade-hns-lock">
            {outpointText(trade.hnsLock)}
          </dd>
        </div>
        <div className="flex justify-between gap-3 sm:col-span-2">
          <dt className="text-gray-500">Watchtower</dt>
          <dd className="text-right text-gray-800" data-testid="trade-watchtower">
            {WATCHTOWER[trade.watchtower]}
          </dd>
        </div>
      </dl>
    </section>
  );
}

/** One card per trade, those waiting for this party's Commit first. */
export function MyTrades({
  trades,
  loading,
  error,
  refusal,
  committingId,
  cancellingId,
  onCommit,
  onCancel,
}: {
  trades: TradeRow[];
  loading: boolean;
  error: unknown;
  refusal: string | null;
  committingId: string | null;
  cancellingId: string | null;
  onCommit: (trade: TradeRow) => void;
  onCancel: (trade: TradeRow) => void;
}) {
  if (loading) {
    return (
      <div className="flex items-center gap-2 py-6 text-sm text-gray-500">
        <Spinner /> Reading your trades…
      </div>
    );
  }
  const sorted = [...trades].sort(
    (a, b) => Number(needsMyCommit(b)) - Number(needsMyCommit(a)) || b.updatedAt - a.updatedAt,
  );
  return (
    <div className="space-y-3">
      {error != null && (
        <p role="alert" className="text-sm text-red-600" data-testid="my-trades-error">
          {mapError(error)}
        </p>
      )}
      {sorted.length === 0 ? (
        <p
          className="rounded-lg border border-dashed border-gray-300 bg-white px-4 py-6 text-center text-sm text-gray-500"
          data-testid="my-trades-empty"
        >
          {NO_TRADES}
        </p>
      ) : (
        sorted.map((t) => (
          <TradeCard
            key={t.tradeId}
            trade={t}
            refusal={refusal}
            committing={committingId === t.tradeId}
            cancelling={cancellingId === t.tradeId}
            onCommit={onCommit}
            onCancel={onCancel}
          />
        ))
      )}
    </div>
  );
}
