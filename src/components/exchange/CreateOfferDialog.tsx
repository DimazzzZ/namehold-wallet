import { useState } from "react";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { GatedButton } from "./GatedButton";
import { mapError } from "../../lib/errors";
import { doosToHnsDigits, parseHnsAmount } from "../../lib/hnsAmount";
import {
  btcSatsFor,
  btcText,
  checkTradeLimits,
  grouped,
  hnsDoosText,
  msatToPriceDigits,
  parsePriceMsat,
  priceMsatText,
  satsText,
  type TradeLimits,
} from "../../lib/exchangeAmount";
import {
  MARKET_PRICES,
  SECURE_WINDOW_WAIT,
  SMALLEST_ABOVE_AMOUNT,
  TRADE_LIMITS,
  TRADE_TOO_LARGE,
  TRADE_TOO_SMALL,
} from "../../lib/exchangeText";
import { cn } from "../../lib/utils";
import type { CreateOfferArgs } from "../../queries/exchange";
import type { OfferRow, OfferSide } from "../../types/exchange";

export interface OfferForm {
  side: OfferSide;
  /** The HNS the offer trades: the whole trade, or its largest part. */
  amount: string;
  price: string;
  /** A buyer may take part of `amount`, down to `smallest`. */
  split: boolean;
  smallest: string;
}

/** What one trade of the offer locks and brings, formatted. */
export interface OfferTrade {
  lock: string;
  receive: string;
}

/**
 * Each field's sentence (null = fine or still empty), what a trade locks and
 * brings once amount and price read, the limits sentence when it does not fit,
 * and the arguments once everything reads and fits.
 */
export interface OfferCheck {
  amount: string | null;
  price: string | null;
  smallest: string | null;
  trade: OfferTrade | null;
  limit: string | null;
  args: CreateOfferArgs | null;
}

const satsWithBtc = (lo: bigint, hi: bigint) =>
  lo === hi
    ? `${satsText(lo)} (${btcText(lo)})`
    : `${grouped(lo)} to ${satsText(hi)} (${btcText(lo)?.replace(/ BTC$/, "")} to ${btcText(hi)})`;
const hnsRange = (lo: bigint, hi: bigint) =>
  lo === hi ? hnsDoosText(hi) : `${hnsDoosText(lo).replace(/ HNS$/, "")} to ${hnsDoosText(hi)}`;

/**
 * Read the Create offer form exactly: an amount in HNS (the money-field rule
 * of `hnsAmount.ts`), a price in sats per HNS (at most 3 decimals, above 0),
 * and, when a buyer may take part of it, the smallest part (≤ the amount).
 * With `limits` (the network's policy) a trade must fit them: the sentence
 * says what it is worth and what would fit. An empty field has no sentence
 * yet but keeps the form from submitting.
 */
export function checkOffer(form: OfferForm, limits: TradeLimits | null): OfferCheck {
  const blank = (t: string) => t.trim() === "";
  const amount = blank(form.amount) ? null : parseHnsAmount(form.amount);
  const price = blank(form.price) ? null : parsePriceMsat(form.price);
  const smallest = !form.split || blank(form.smallest) ? null : parseHnsAmount(form.smallest);
  const out: OfferCheck = {
    amount: amount && "error" in amount ? amount.error : null,
    price: price && "error" in price ? price.error : null,
    smallest: smallest && "error" in smallest ? smallest.error : null,
    trade: null,
    limit: null,
    args: null,
  };
  if (!amount || "error" in amount || !price || "error" in price) return out;
  const hi = BigInt(amount.doos);
  let lo = hi;
  if (form.split) {
    if (!smallest || "error" in smallest) return out;
    lo = BigInt(smallest.doos);
    if (lo > hi) {
      out.smallest = SMALLEST_ABOVE_AMOUNT;
      return out;
    }
  }
  const sats = satsWithBtc(btcSatsFor(lo, price.msat), btcSatsFor(hi, price.msat));
  const hns = hnsRange(lo, hi);
  out.trade = form.side === "sell" ? { lock: hns, receive: sats } : { lock: sats, receive: hns };
  if (limits) {
    const v = checkTradeLimits(lo, hi, price.msat, limits);
    if (!v.ok) {
      const t = {
        worth: satsText(v.sats) ?? "",
        limit: satsText(v.tooSmall ? limits.minSats : limits.maxSats) ?? "",
        price: priceMsatText(price.msat),
        hns: hnsDoosText(v.hnsDoos),
        amount: hnsDoosText(v.tooSmall ? lo : hi),
        needPrice: priceMsatText(v.priceMsat),
      };
      out.limit = v.tooSmall ? TRADE_TOO_SMALL(t) : TRADE_TOO_LARGE(t);
      return out;
    }
  }
  out.args = {
    side: form.side,
    price: msatToPriceDigits(price.msat),
    minHns: doosToHnsDigits(Number(lo)),
    maxHns: doosToHnsDigits(Number(hi)),
  };
  return out;
}

/** The order book's best prices from other makers: the lowest sell, the highest buy. */
export function marketPrices(offers: OfferRow[]): string | null {
  let sell: bigint | null = null;
  let buy: bigint | null = null;
  for (const o of offers) {
    if (o.own) continue;
    const p = parsePriceMsat(o.price);
    if ("error" in p) continue;
    if (o.side === "sell" && (sell === null || p.msat < sell)) sell = p.msat;
    if (o.side === "buy" && (buy === null || p.msat > buy)) buy = p.msat;
  }
  if (sell === null && buy === null) return null;
  return MARKET_PRICES(
    sell === null ? null : priceMsatText(sell),
    buy === null ? null : priceMsatText(buy),
  );
}

const SIDES: { side: OfferSide; label: string; hint: string }[] = [
  { side: "sell", label: "Sell HNS", hint: "You lock HNS and receive BTC" },
  { side: "buy", label: "Buy HNS", hint: "You lock BTC and receive HNS" },
];

function FieldError({ testId, text }: { testId: string; text: string | null }) {
  if (!text) return null;
  return (
    <p role="alert" className="mt-1 text-xs text-red-600" data-testid={testId}>
      {text}
    </p>
  );
}

export function CreateOfferDialog({
  refusal,
  busy,
  limits,
  market,
  onClose,
  onCreate,
}: {
  refusal: string | null;
  busy: boolean;
  /** The network's limits for one trade; null until the overview answers. */
  limits: TradeLimits | null;
  /** The order book, for the price hint. */
  market: OfferRow[];
  onClose: () => void;
  onCreate: (args: CreateOfferArgs) => Promise<void>;
}) {
  const [form, setForm] = useState<OfferForm>({
    side: "sell",
    amount: "",
    price: "",
    split: false,
    smallest: "",
  });
  const [error, setError] = useState<string | null>(null);
  const check = checkOffer(form, limits);
  const prices = marketPrices(market);
  const set = (patch: Partial<OfferForm>) => {
    setForm((f) => ({ ...f, ...patch }));
    setError(null);
  };

  const submit = async () => {
    if (!check.args) return;
    setError(null);
    try {
      await onCreate(check.args);
    } catch (e) {
      setError(mapError(e));
    }
  };

  return (
    <Dialog open onClose={() => !busy && onClose()} title="Make an offer" className="max-w-[460px]">
      <div className="space-y-3 text-sm">
        <div role="radiogroup" aria-label="Side" className="grid grid-cols-2 gap-2">
          {SIDES.map((s) => (
            <button
              key={s.side}
              type="button"
              role="radio"
              aria-checked={form.side === s.side}
              data-testid={`offer-side-${s.side}`}
              onClick={() => set({ side: s.side })}
              className={cn(
                "rounded-md border px-3 py-2 text-left",
                form.side === s.side
                  ? "border-blue-600 bg-blue-50 text-blue-900"
                  : "border-gray-300 bg-white text-gray-700 hover:bg-gray-50",
              )}
            >
              <div className="font-medium">{s.label}</div>
              <div className="text-xs text-gray-500">{s.hint}</div>
            </button>
          ))}
        </div>
        <div>
          <Input
            label="Amount (HNS)"
            id="offer-amount"
            data-testid="offer-amount-input"
            inputMode="decimal"
            autoComplete="off"
            value={form.amount}
            onChange={(e) => set({ amount: e.target.value })}
          />
          <FieldError testId="offer-amount-error" text={check.amount} />
          <label className="mt-2 flex items-center gap-2 text-sm text-gray-700">
            <input
              type="checkbox"
              data-testid="offer-split"
              checked={form.split}
              onChange={(e) => set({ split: e.target.checked })}
            />
            {form.side === "sell" ? "Let a buyer take part of it" : "Let a seller fill part of it"}
          </label>
          {form.split && (
            <div className="mt-2">
              <Input
                label="Smallest part (HNS)"
                id="offer-smallest"
                data-testid="offer-smallest-input"
                inputMode="decimal"
                autoComplete="off"
                value={form.smallest}
                onChange={(e) => set({ smallest: e.target.value })}
              />
              <FieldError testId="offer-smallest-error" text={check.smallest} />
            </div>
          )}
        </div>
        <div>
          <Input
            label="Price (sats per 1 HNS)"
            id="offer-price"
            data-testid="offer-price-input"
            inputMode="decimal"
            autoComplete="off"
            value={form.price}
            onChange={(e) => set({ price: e.target.value })}
          />
          <FieldError testId="offer-price-error" text={check.price} />
          {prices && (
            <p className="mt-1 text-xs text-gray-500" data-testid="offer-market">
              {prices}
            </p>
          )}
        </div>
        {check.trade && (
          <dl
            className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 rounded-md bg-gray-50 px-3 py-2 text-gray-800"
            data-testid="offer-trade"
          >
            <dt className="text-gray-500">You lock</dt>
            <dd data-testid="offer-lock">{check.trade.lock}</dd>
            <dt className="text-gray-500">You receive</dt>
            <dd data-testid="offer-receive">{check.trade.receive}</dd>
          </dl>
        )}
        {check.limit && (
          <p
            role="alert"
            className="text-xs text-red-600 [text-wrap:pretty]"
            data-testid="offer-limit"
          >
            {check.limit}
          </p>
        )}
        {limits && (
          <p className="text-xs text-gray-500 [text-wrap:pretty]" data-testid="offer-limits">
            {TRADE_LIMITS(grouped(limits.minSats), satsText(limits.maxSats) ?? "")}
          </p>
        )}
        <p className="text-xs text-gray-500 [text-wrap:pretty]">
          The offer stays listed only while your Namehold keeps renewing it. Every trade someone
          starts from it still asks you to confirm in the secure window.
        </p>
        {error && (
          <p role="alert" className="text-xs text-red-600" data-testid="offer-create-error">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" data-testid="offer-cancel" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <GatedButton
            variant="primary"
            data-testid="offer-submit"
            refusal={refusal}
            disabled={check.args === null}
            loading={busy}
            onClick={() => void submit()}
          >
            {busy ? SECURE_WINDOW_WAIT : "Publish offer"}
          </GatedButton>
        </div>
      </div>
    </Dialog>
  );
}
