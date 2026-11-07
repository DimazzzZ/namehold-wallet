import { Card } from "../ui/Card";
import { Badge } from "../ui/Badge";
import { Spinner } from "../ui/Spinner";
import { GatedButton } from "./GatedButton";
import { btcRangeText, hnsRangeText } from "./OrderBook";
import { mapError } from "../../lib/errors";
import { priceText, UNREADABLE } from "../../lib/exchangeAmount";
import { NO_OFFERS, SIDE_TITLE } from "../../lib/exchangeText";
import type { OfferRow } from "../../types/exchange";

/** The user's own offers: live or paused, each with Withdraw. */
export function MyOffers({
  offers,
  loading,
  error,
  refusal,
  withdrawingId,
  onWithdraw,
}: {
  offers: OfferRow[];
  loading: boolean;
  error: unknown;
  refusal: string | null;
  withdrawingId: string | null;
  onWithdraw: (offer: OfferRow) => void;
}) {
  if (loading) {
    return (
      <div className="flex items-center gap-2 py-6 text-sm text-gray-500">
        <Spinner /> Reading your offers…
      </div>
    );
  }
  return (
    <Card padded={false}>
      {error != null && (
        <p role="alert" className="px-4 pt-3 text-sm text-red-600" data-testid="my-offers-error">
          {mapError(error)}
        </p>
      )}
      {offers.length === 0 ? (
        <p className="px-4 py-6 text-center text-sm text-gray-500" data-testid="my-offers-empty">
          {NO_OFFERS}
        </p>
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-gray-100 text-left text-xs text-gray-500">
              <th className="px-4 py-2 font-medium">Side</th>
              <th className="px-4 py-2 font-medium">Price (sats per HNS)</th>
              <th className="px-4 py-2 font-medium">Amount</th>
              <th className="px-4 py-2 font-medium">BTC</th>
              <th className="px-4 py-2 font-medium">State</th>
              <th className="px-4 py-2" />
            </tr>
          </thead>
          <tbody>
            {offers.map((o) => (
              <tr
                key={o.id}
                data-testid="my-offer-row"
                data-offer-id={o.id}
                className="border-b border-gray-50 last:border-0"
              >
                <td className="px-4 py-2">{SIDE_TITLE[o.side]}</td>
                <td className="px-4 py-2 tabular-nums">
                  {priceText(o.price)?.replace(/ sats\/HNS$/, "") ?? UNREADABLE}
                </td>
                <td className="px-4 py-2 tabular-nums">{hnsRangeText(o)}</td>
                <td className="px-4 py-2 tabular-nums text-gray-600">{btcRangeText(o)}</td>
                <td className="px-4 py-2">
                  {o.live ? (
                    <Badge
                      variant="success"
                      data-testid="my-offer-live"
                      title="Published on the relay and renewed by this Namehold"
                    >
                      Live
                    </Badge>
                  ) : (
                    <Badge
                      variant="default"
                      data-testid="my-offer-paused"
                      title="Not renewed on the relay right now, so others do not see it"
                    >
                      Paused
                    </Badge>
                  )}
                </td>
                <td className="px-4 py-2 text-right">
                  <GatedButton
                    size="sm"
                    variant="secondary"
                    data-testid={`withdraw-offer-${o.id}`}
                    refusal={refusal}
                    loading={withdrawingId === o.id}
                    onClick={() => onWithdraw(o)}
                  >
                    Withdraw
                  </GatedButton>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Card>
  );
}
