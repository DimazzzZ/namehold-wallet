import { Card } from "../ui/Card";
import { Badge } from "../ui/Badge";
import { Spinner } from "../ui/Spinner";
import { GatedButton } from "./GatedButton";
import { mapError } from "../../lib/errors";
import { btcText, hnsText, priceText, UNREADABLE } from "../../lib/exchangeAmount";
import { SIDE_EMPTY, SIDE_SUBTITLE, SIDE_TITLE, takeRefusal } from "../../lib/exchangeText";
import type { ExchangeOverview, MarketOffers, OfferRow, OfferSide } from "../../types/exchange";

/** "100 – 2,500 HNS", or a dash when either end cannot be read. */
export function hnsRangeText(offer: Pick<OfferRow, "minHns" | "maxHns">): string {
  const min = hnsText(offer.minHns);
  const max = hnsText(offer.maxHns);
  if (min === null || max === null) return UNREADABLE;
  return `${min.replace(/ HNS$/, "")} – ${max}`;
}

/** "≈ 0.00020000 – 0.00500000 BTC", or a dash when either end cannot be shown. */
export function btcRangeText(offer: Pick<OfferRow, "minSats" | "maxSats">): string {
  const min = btcText(offer.minSats);
  const max = btcText(offer.maxSats);
  if (min === null || max === null) return UNREADABLE;
  return `≈ ${min.replace(/ BTC$/, "")} – ${max}`;
}

function OfferList({
  side,
  offers,
  overview,
  onTake,
}: {
  side: OfferSide;
  offers: OfferRow[];
  overview: ExchangeOverview | undefined;
  onTake: (offer: OfferRow) => void;
}) {
  return (
    <Card title={SIDE_TITLE[side]} subtitle={SIDE_SUBTITLE[side]} padded={false}>
      <div data-testid={`order-book-${side}`}>
        {offers.length === 0 ? (
          <p className="px-4 py-6 text-center text-sm text-gray-500">{SIDE_EMPTY[side]}</p>
        ) : (
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-gray-100 text-left text-xs text-gray-500">
                <th className="px-4 py-2 font-medium">Price (sats per HNS)</th>
                <th className="px-4 py-2 font-medium">Amount</th>
                <th className="px-4 py-2 font-medium">BTC</th>
                <th className="px-4 py-2 font-medium">Maker</th>
                <th className="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              {offers.map((o) => (
                <tr
                  key={o.id}
                  data-testid="offer-row"
                  data-offer-id={o.id}
                  className="border-b border-gray-50 last:border-0"
                >
                  <td className="px-4 py-2 tabular-nums" data-testid="offer-price">
                    {priceText(o.price)?.replace(/ sats\/HNS$/, "") ?? UNREADABLE}
                  </td>
                  <td className="px-4 py-2 tabular-nums" data-testid="offer-range">
                    {hnsRangeText(o)}
                  </td>
                  <td className="px-4 py-2 tabular-nums text-gray-600" data-testid="offer-btc">
                    {btcRangeText(o)}
                  </td>
                  <td className="px-4 py-2">
                    <span className="font-mono text-xs text-gray-600">{o.makerKey}</span>
                    {o.own && (
                      <Badge variant="info" className="ml-2" data-testid="offer-own">
                        Yours
                      </Badge>
                    )}
                  </td>
                  <td className="px-4 py-2 text-right">
                    <GatedButton
                      size="sm"
                      variant="secondary"
                      data-testid={`take-offer-${o.id}`}
                      refusal={takeRefusal(overview, o)}
                      onClick={() => onTake(o)}
                    >
                      Take
                    </GatedButton>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </Card>
  );
}

/** The relay's offers, split by the maker's side. */
export function OrderBook({
  overview,
  market,
  loading,
  error,
  onTake,
}: {
  overview: ExchangeOverview | undefined;
  market: MarketOffers | undefined;
  loading: boolean;
  error: unknown;
  onTake: (offer: OfferRow) => void;
}) {
  if (loading) {
    return (
      <div className="flex items-center gap-2 py-6 text-sm text-gray-500">
        <Spinner /> Reading the order book…
      </div>
    );
  }
  const offers = market?.offers ?? [];
  const problem = error != null ? mapError(error) : (market?.error ?? null);
  return (
    <div className="space-y-4">
      {problem && (
        <p role="alert" className="text-sm text-red-600" data-testid="order-book-error">
          {problem}
        </p>
      )}
      <OfferList
        side="sell"
        offers={offers.filter((o) => o.side === "sell")}
        overview={overview}
        onTake={onTake}
      />
      <OfferList
        side="buy"
        offers={offers.filter((o) => o.side === "buy")}
        overview={overview}
        onTake={onTake}
      />
    </div>
  );
}
