import { useEffect, useRef, useState, type MutableRefObject, type ReactNode } from "react";
import { Button } from "../ui/Button";
import { displayName } from "../../lib/idn";
import { mapError } from "../../lib/errors";
import { parseFeeRateArg } from "../../lib/feeRate";
import { formatHns, formatHnsShort } from "../../lib/utils";
import { usePurchasePreview } from "../../queries/shakedex";
import { useScrollportHeight } from "../../hooks/useScrollportHeight";
import type { PurchasePreview } from "../../types";
import { PriceSteps } from "./PriceSteps";
import { usePurchase } from "./usePurchase";
import {
  feeRateError,
  fixedPriceCaption,
  priceDropText,
  registeredUntilMonth,
  uncheckedText,
  waitStep,
  WAIT_UNKNOWN,
} from "./marketText";
import { primaryAction, type Entry, type ListedEntry, type Origin } from "./marketModel";

const SOURCE: Record<Origin, string> = {
  market: "LearnHNS Market",
  file: "Imported from file",
  link: "Imported from LearnHNS link",
};

const SECTION = "border-t border-gray-100 px-5 py-4";

interface ListingDetailProps {
  entry: Entry | null;
  /** Bought from this panel in this session. */
  sent: boolean;
  /** The profile's or the node's refusal (`shakedexRefusal`), or null. */
  refusal: string | null;
  gated: boolean;
  onTurnOn: () => void;
  availableDoos: number | null;
  /** Whether to pay the market fee for this listing; null until the user or the first preview says. */
  pay: boolean | null;
  onPay: (pay: boolean) => void;
  checking: boolean;
  onRetry: () => void;
  onBought: (e: ListedEntry) => void;
  /** Enter's action: set to the panel's primary, null when there is none. */
  primaryRef: MutableRefObject<(() => void) | null>;
}

/** The sticky card beside the list: the selected listing, what the node checked, and what buying costs. */
export function ListingDetail(p: ListingDetailProps) {
  const ref = useRef<HTMLElement>(null);
  // No taller than the page area, so Buy at its foot is in sight beside a
  // long list: the panel scrolls inside instead (its action stays pinned).
  const maxHeight = useScrollportHeight(ref);
  return (
    <aside
      ref={ref}
      aria-live="polite"
      aria-label="Listing"
      data-testid="listing-detail"
      style={maxHeight != null ? { maxHeight } : undefined}
      className="sticky top-0 min-w-0 max-w-[420px] flex-[1_1_340px] overflow-y-auto rounded-md border border-gray-200 bg-white"
    >
      {p.entry ? (
        <Selected {...p} entry={p.entry} />
      ) : (
        <p className="px-6 py-10 text-center text-[13px] text-gray-500 [text-wrap:pretty]">
          Pick a listing to see its price, what your node checked, and what buying involves.
        </p>
      )}
    </aside>
  );
}

function Selected(p: ListingDetailProps & { entry: Entry }) {
  const { entry } = p;
  const kind =
    entry.kind === "listed"
      ? entry.row.kind === "reverseAuction"
        ? "Price drops over time"
        : "Fixed price"
      : null;
  return (
    <>
      <div className="px-5 pb-3.5 pt-4">
        <p className="flex flex-wrap items-center gap-2 text-xs text-gray-500">
          {kind && (
            <>
              <span>{kind}</span>
              <span aria-hidden>·</span>
            </>
          )}
          <span>{SOURCE[entry.origin]}</span>
        </p>
        <h3 className="mt-1 break-all font-mono text-[22px] font-semibold text-gray-900">
          .{displayName(entry.name)}
        </h3>
      </div>
      {entry.kind === "unchecked" ? (
        <div className={SECTION} data-testid="detail-unchecked">
          <p className="text-sm font-semibold text-amber-800">Couldn't check this listing</p>
          <p className="mt-1 text-[13px] text-gray-600 [text-wrap:pretty]">
            {uncheckedText(entry.reason)}
          </p>
          <Button
            size="sm"
            className="mt-3"
            data-testid="detail-check-again"
            disabled={p.checking}
            onClick={p.onRetry}
          >
            {p.checking ? "Checking…" : "Check again"}
          </Button>
        </div>
      ) : (
        <Verified {...p} entry={entry} />
      )}
    </>
  );
}

/** A check the node made, in one sentence: ✓ green, or ! amber. */
function Check({ ok, children }: { ok: boolean; children: ReactNode }) {
  return (
    <li className="grid grid-cols-[16px_1fr] gap-1">
      <span aria-hidden className={ok ? "text-green-700" : "font-semibold text-amber-700"}>
        {ok ? "✓" : "!"}
      </span>
      <span className={ok ? "text-gray-700" : "text-amber-800"}>{children}</span>
    </li>
  );
}

/**
 * How it works, in three steps. The wait is the profile network's lockup from
 * the preview: 288 blocks on mainnet, 10 on regtest.
 */
function steps(p: PurchasePreview | undefined): [string, string][] {
  return [
    ["Buy", "You pay the total and the name is locked to you on-chain."],
    p ? waitStep(p.lockupBlocks, p.wallClockTiming) : ["Wait", WAIT_UNKNOWN],
    ["Finalize", "One click, and the name moves into your wallet."],
  ];
}

function Verified(p: ListingDetailProps & { entry: ListedEntry }) {
  const { entry, sent } = p;
  const { row } = entry;
  const v = row.verdict.verdict === "buyable" ? row.verdict : null;
  const fromMarket = entry.origin !== "file";
  const [feeOpen, setFeeOpen] = useState(false);
  const [feeRateRaw, setFeeRateRaw] = useState("");
  const rateError = feeRateError(feeRateRaw);
  const feeRate = rateError ? null : parseFeeRateArg(feeRateRaw);
  // R33: the backend refuses an own row before it asks the node, so it is not priced.
  const preview = usePurchasePreview(
    row.own || sent ? null : row.listingJson,
    p.pay ?? true,
    fromMarket,
    feeRate,
  );
  const purchase = usePurchase();
  const data = preview.data;
  const fee = data?.marketFee ?? null;
  // A listing from a file did not come through LearnHNS: its fee is never paid.
  const payable = fromMarket && fee != null && fee.payable;
  const paying = p.pay === true && payable;
  const { pay, onPay } = p;

  useEffect(() => {
    if (pay === null && data) onPay(payable && fee.published);
  }, [pay, onPay, data, payable, fee]);

  const action = primaryAction({
    name: row.name,
    own: row.own,
    refusal: p.refusal,
    gated: p.gated,
    totalDoos: data?.totalDoos ?? null,
    // The fee box's choice is not settled until its preview lands.
    pricing: preview.isFetching || (p.pay === null && !preview.isError),
    previewError: preview.isError ? mapError(preview.error) : null,
    feeRateError: rateError,
    availableDoos: p.availableDoos,
    drop:
      row.kind === "reverseAuction" && row.nextPrice != null && row.nextValidInSecs != null
        ? { price: row.nextPrice, inSecs: row.nextValidInSecs }
        : null,
    busy: purchase.busy,
  });

  const run = async () => {
    if (action.kind === "fixit") return p.onTurnOn();
    if (action.kind !== "buy" || !data) return;
    const ok = await purchase.buy(
      {
        listingJson: row.listingJson,
        acceptedMarketFeeDoos: paying && fee ? fee.valueDoos : null,
        fromMarket,
        feeRate,
      },
      () => void preview.refetch(),
    );
    if (ok) p.onBought(entry);
  };

  const { primaryRef } = p;
  useEffect(() => {
    primaryRef.current = sent ? null : () => void run();
    return () => {
      primaryRef.current = null;
    };
  });

  const expiry = v ? registeredUntilMonth(v.expiryEnd, v.tip) : null;
  const stepList = steps(data);

  return (
    <>
      <div className={SECTION}>
        <p className="text-xs text-gray-500">Price now</p>
        <p
          className="text-[26px] font-semibold tabular-nums text-gray-900"
          data-testid="detail-price"
        >
          {row.currentPrice != null ? `${formatHnsShort(row.currentPrice)} HNS` : "—"}
        </p>
        {row.kind === "reverseAuction" ? (
          <>
            <p className="mt-1 text-[13px] text-gray-700 [text-wrap:pretty]">
              {priceDropText(row)}
            </p>
            <PriceSteps row={row} />
            <p className="mt-2 text-xs text-gray-500">
              Waiting gets a lower price, but anyone can buy it first.
            </p>
          </>
        ) : (
          <p className="mt-1 text-xs text-gray-500 [text-wrap:pretty]">
            {fixedPriceCaption(row.expiresAt)}
          </p>
        )}
      </div>

      <div className={SECTION}>
        <p className="text-xs font-semibold text-gray-700">Checked by your node</p>
        <ul className="mt-2 space-y-1.5 text-[13px]" data-testid="detail-checks">
          <Check ok>The seller's signature matches the locked name</Check>
          <Check ok>The locked coin is unspent, so the name is still for sale</Check>
          {/* The row's verdict, or the fresher one the preview checked. */}
          {v?.warnExpiry || data?.warnExpiry ? (
            <Check ok={false}>
              The name expires{expiry ? ` in ${expiry}` : ""}, soon after you can finalize. Finalize
              and renew in time or the name is lost.
            </Check>
          ) : (
            expiry && <Check ok>Registered until {expiry}</Check>
          )}
        </ul>
      </div>

      {sent ? (
        <div className={SECTION}>
          <div className="rounded-md bg-green-50 px-3.5 py-3 text-[13px]" data-testid="detail-sent">
            <p className="font-semibold text-green-800">Purchase sent</p>
            <p className="mt-0.5 text-green-800 [text-wrap:pretty]">
              <span className="font-mono">.{displayName(row.name)}</span> is locked to you. Finalize
              it from <strong>Your purchases</strong> above once its wait is over.
            </p>
          </div>
        </div>
      ) : (
        <>
          <div className={SECTION}>
            <dl className="space-y-1.5 text-[13px] tabular-nums" data-testid="purchase-rows">
              <div className="flex justify-between gap-3">
                <dt className="text-gray-600">Price to seller</dt>
                <dd>{data ? `${formatHns(data.priceDoos)} HNS` : "—"}</dd>
              </div>
              <div>
                <div className="flex items-center justify-between gap-3">
                  <dt>
                    <label className="flex items-center gap-2 text-gray-600">
                      <input
                        type="checkbox"
                        data-testid="purchase-market-fee-pay"
                        checked={paying}
                        disabled={!payable || purchase.busy}
                        onChange={(e) => onPay(e.target.checked)}
                      />
                      LearnHNS fee{fee?.percentText != null ? ` (${fee.percentText})` : ""}
                    </label>
                  </dt>
                  <dd
                    data-testid="purchase-market-fee"
                    className={paying ? undefined : "text-gray-400"}
                  >
                    {fee && fromMarket ? `${formatHns(fee.valueDoos)} HNS` : "—"}
                  </dd>
                </div>
                {!fromMarket ? (
                  <p className="mt-0.5 text-xs text-gray-500">
                    This listing didn't come through LearnHNS, so there's no market fee.
                  </p>
                ) : (
                  <>
                    {fee?.warning && <p className="mt-0.5 text-xs text-amber-800">{fee.warning}</p>}
                    {payable && !paying && (
                      <p className="mt-0.5 text-xs text-gray-500">
                        Optional. Without it LearnHNS isn't paid; the seller still gets the full
                        price.
                      </p>
                    )}
                  </>
                )}
              </div>
              <div className="flex justify-between gap-3">
                <dt className="text-gray-600">Network fee</dt>
                <dd>{data ? `${formatHns(data.networkFeeDoos)} HNS` : "—"}</dd>
              </div>
              <div className="flex justify-between gap-3 mt-0.5 border-t border-gray-200 pt-2 font-semibold">
                <dt>Total</dt>
                <dd data-testid="purchase-total">
                  {data ? `${formatHns(data.totalDoos)} HNS` : "—"}
                </dd>
              </div>
            </dl>
            <div className="mt-2 text-xs">
              <button
                type="button"
                data-testid="fee-rate-toggle"
                aria-expanded={feeOpen}
                onClick={() => setFeeOpen((o) => !o)}
                className="cursor-pointer text-gray-600 hover:text-gray-900"
              >
                {feeOpen ? "▾" : "▸"} Fee rate
                {!feeOpen && feeRateRaw.trim() !== "" ? `: ${feeRateRaw.trim()} doos/kvB` : ""}
              </button>
              {feeOpen && (
                <div className="mt-1.5">
                  <input
                    value={feeRateRaw}
                    onChange={(e) => setFeeRateRaw(e.target.value)}
                    inputMode="numeric"
                    placeholder="Fee rate, doos/kvB (empty = default)"
                    aria-label="Fee rate, doos/kvB"
                    data-testid="fee-rate-input"
                    className="w-full rounded border border-gray-300 px-2 py-1 text-xs tabular-nums focus:outline-none focus:ring-2 focus:ring-blue-500"
                  />
                  {rateError ? (
                    <p className="mt-1 text-red-600" data-testid="fee-rate-error">
                      {rateError}
                    </p>
                  ) : (
                    <p className="mt-1 text-gray-500">
                      This transaction only. 1,000 doos/kvB = 1 sat/byte.
                    </p>
                  )}
                </div>
              )}
            </div>
          </div>

          <div className="border-t border-gray-100 px-5 py-3.5">
            <ol className="space-y-3" data-testid="detail-steps">
              {stepList.map(([title, text], i) => (
                <li key={title} className="relative grid grid-cols-[18px_1fr] gap-2.5">
                  {i < stepList.length - 1 && (
                    <span
                      aria-hidden
                      className="absolute left-[8.5px] top-5 h-[calc(100%-4px)] w-px bg-gray-200"
                    />
                  )}
                  <span className="flex h-[18px] w-[18px] items-center justify-center rounded-full bg-blue-50 text-[11px] font-semibold text-blue-700">
                    {i + 1}
                  </span>
                  <span>
                    <span className="block text-[13px] font-medium text-gray-900">{title}</span>
                    <span className="block text-xs text-gray-500">{text}</span>
                  </span>
                </li>
              ))}
            </ol>
          </div>

          <div
            className="sticky bottom-0 border-t border-gray-100 bg-white px-5 py-4"
            data-testid="detail-action"
          >
            {action.reason && (
              <p
                id="detail-action-reason"
                data-testid="detail-action-reason"
                className="mb-2 rounded border border-amber-200 bg-amber-50 px-2.5 py-1.5 text-xs text-amber-800 [text-wrap:pretty]"
              >
                {action.reason}
              </p>
            )}
            <Button
              // A profile or node that cannot buy at all gets no filled
              // button: beside the gate banner's it would make two.
              variant={action.kind === "fixit" ? "fixit" : p.refusal ? "secondary" : "primary"}
              className="w-full py-[9px] text-sm"
              data-testid="purchase-buy"
              disabled={action.kind === "disabled"}
              aria-describedby={action.reason ? "detail-action-reason" : undefined}
              onClick={() => void run()}
            >
              {action.label}
            </Button>
            {purchase.error && (
              <p role="alert" className="mt-2 text-xs text-red-600" data-testid="purchase-error">
                {purchase.error}
              </p>
            )}
          </div>
        </>
      )}
    </>
  );
}
