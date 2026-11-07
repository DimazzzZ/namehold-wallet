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
  hnsText,
  offerHnsDoos,
  parsePriceMsat,
  priceText,
  satsText,
  UNREADABLE,
} from "../../lib/exchangeAmount";
import {
  OFFER_UNREADABLE,
  TAKE_ABOVE_MAX,
  TAKE_BELOW_MIN,
  TAKE_ZERO_SATS,
  takerRole,
} from "../../lib/exchangeText";
import type { OfferRow } from "../../types/exchange";

/** What the Take dialog makes of the typed amount against an offer. */
export type TakeCheck =
  { ok: true; hnsAmount: string; sats: bigint } | { ok: false; error: string | null };

/**
 * Read the amount the user typed against the offer: exact doos (the
 * `hnsAmount.ts` parser), inside the offer's min..max, worth at least 1 sat.
 * `error` is null for an empty field (nothing to say yet).
 */
export function checkTake(
  offer: Pick<OfferRow, "price" | "minHns" | "maxHns">,
  text: string,
): TakeCheck {
  const price = parsePriceMsat(offer.price);
  const min = offerHnsDoos(offer.minHns);
  const max = offerHnsDoos(offer.maxHns);
  if ("error" in price || min === null || max === null)
    return { ok: false, error: OFFER_UNREADABLE };
  if (text.trim() === "") return { ok: false, error: null };
  const read = parseHnsAmount(text);
  if ("error" in read) return { ok: false, error: read.error };
  const doos = BigInt(read.doos);
  if (doos < min) return { ok: false, error: TAKE_BELOW_MIN(offer.minHns) };
  if (doos > max) return { ok: false, error: TAKE_ABOVE_MAX(offer.maxHns) };
  const sats = btcSatsFor(doos, price.msat);
  if (sats === 0n) return { ok: false, error: TAKE_ZERO_SATS };
  return { ok: true, hnsAmount: doosToHnsDigits(read.doos), sats };
}

export function TakeOfferDialog({
  offer,
  refusal,
  busy,
  onClose,
  onTake,
}: {
  offer: OfferRow;
  /** The gate's refusal for this offer, re-checked while the dialog is open. */
  refusal: string | null;
  busy: boolean;
  onClose: () => void;
  onTake: (hnsAmount: string) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const check = checkTake(offer, text);
  const buying = takerRole(offer.side) === "buyer";

  const submit = async () => {
    if (!check.ok) return;
    setError(null);
    try {
      await onTake(check.hnsAmount);
    } catch (e) {
      setError(mapError(e));
    }
  };

  return (
    <Dialog
      open
      onClose={() => !busy && onClose()}
      title={buying ? "Buy HNS" : "Sell HNS"}
      className="max-w-[460px]"
    >
      <div className="space-y-3 text-sm">
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-gray-700">
          <dt className="text-gray-500">Price</dt>
          <dd data-testid="take-price">{priceText(offer.price) ?? UNREADABLE}</dd>
          <dt className="text-gray-500">Limits</dt>
          <dd>
            {hnsText(offer.minHns) ?? UNREADABLE} to {hnsText(offer.maxHns) ?? UNREADABLE}
          </dd>
          <dt className="text-gray-500">Maker</dt>
          <dd className="font-mono text-xs">{offer.makerKey}</dd>
        </dl>
        <div>
          <Input
            label={buying ? "HNS to buy" : "HNS to sell"}
            id="take-amount"
            data-testid="take-amount"
            inputMode="decimal"
            autoComplete="off"
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setError(null);
            }}
          />
          {!check.ok && check.error && (
            <p role="alert" className="mt-1 text-xs text-red-600" data-testid="take-amount-error">
              {check.error}
            </p>
          )}
        </div>
        {check.ok && (
          <p className="rounded-md bg-gray-50 px-3 py-2 text-gray-800" data-testid="take-btc">
            {buying ? "You pay" : "You receive"} <strong>{btcText(check.sats)}</strong> (
            {satsText(check.sats)}).
          </p>
        )}
        <p className="text-xs text-gray-500 [text-wrap:pretty]">
          Nothing is signed now. Once both sides agree the terms, the trade asks you to confirm in
          the secure window.
        </p>
        {error && (
          <p role="alert" className="text-xs text-red-600" data-testid="take-error">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" data-testid="take-cancel" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <GatedButton
            variant="primary"
            data-testid="take-submit"
            refusal={refusal}
            disabled={!check.ok}
            loading={busy}
            onClick={() => void submit()}
          >
            Take offer
          </GatedButton>
        </div>
      </div>
    </Dialog>
  );
}
