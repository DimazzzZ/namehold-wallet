import { useState } from "react";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { GatedButton } from "./GatedButton";
import { mapError } from "../../lib/errors";
import { doosToHnsDigits, parseHnsAmount } from "../../lib/hnsAmount";
import { msatToPriceDigits, parsePriceMsat } from "../../lib/exchangeAmount";
import { MIN_ABOVE_MAX, SECURE_WINDOW_WAIT } from "../../lib/exchangeText";
import { cn } from "../../lib/utils";
import type { CreateOfferArgs } from "../../queries/exchange";
import type { OfferSide } from "../../types/exchange";

export interface OfferForm {
  side: OfferSide;
  price: string;
  min: string;
  max: string;
}

/** Each field's sentence (null = fine or still empty), and the arguments once every field reads. */
export interface OfferCheck {
  price: string | null;
  min: string | null;
  max: string | null;
  args: CreateOfferArgs | null;
}

/**
 * Read the Create offer form exactly: a price in sats per HNS (at most 3
 * decimals, above 0), the smallest and largest trade in HNS (the money-field
 * rule of `hnsAmount.ts`), smallest ≤ largest. An empty field has no sentence
 * yet but keeps the form from submitting.
 */
export function checkOffer(form: OfferForm): OfferCheck {
  const blank = (t: string) => t.trim() === "";
  const price = blank(form.price) ? null : parsePriceMsat(form.price);
  const min = blank(form.min) ? null : parseHnsAmount(form.min);
  const max = blank(form.max) ? null : parseHnsAmount(form.max);
  const out: OfferCheck = {
    price: price && "error" in price ? price.error : null,
    min: min && "error" in min ? min.error : null,
    max: max && "error" in max ? max.error : null,
    args: null,
  };
  if (price && "msat" in price && min && "doos" in min && max && "doos" in max) {
    if (min.doos > max.doos) {
      out.min = MIN_ABOVE_MAX;
      return out;
    }
    out.args = {
      side: form.side,
      price: msatToPriceDigits(price.msat),
      minHns: doosToHnsDigits(min.doos),
      maxHns: doosToHnsDigits(max.doos),
    };
  }
  return out;
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
  onClose,
  onCreate,
}: {
  refusal: string | null;
  busy: boolean;
  onClose: () => void;
  onCreate: (args: CreateOfferArgs) => Promise<void>;
}) {
  const [form, setForm] = useState<OfferForm>({ side: "sell", price: "", min: "", max: "" });
  const [error, setError] = useState<string | null>(null);
  const check = checkOffer(form);
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
            label="Price in sats per HNS"
            id="offer-price"
            data-testid="offer-price-input"
            inputMode="decimal"
            autoComplete="off"
            value={form.price}
            onChange={(e) => set({ price: e.target.value })}
          />
          <FieldError testId="offer-price-error" text={check.price} />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <Input
              label="Smallest trade (HNS)"
              id="offer-min"
              data-testid="offer-min-input"
              inputMode="decimal"
              autoComplete="off"
              value={form.min}
              onChange={(e) => set({ min: e.target.value })}
            />
            <FieldError testId="offer-min-error" text={check.min} />
          </div>
          <div>
            <Input
              label="Largest trade (HNS)"
              id="offer-max"
              data-testid="offer-max-input"
              inputMode="decimal"
              autoComplete="off"
              value={form.max}
              onChange={(e) => set({ max: e.target.value })}
            />
            <FieldError testId="offer-max-error" text={check.max} />
          </div>
        </div>
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
