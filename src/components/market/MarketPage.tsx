import { useState } from "react";
import { PageHeader } from "../ui/PageHeader";
import { Alert } from "../ui/Alert";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Disclosure } from "../ui/Disclosure";
import { ImportListing } from "./ImportListing";
import {
  MARKET_MAINNET_ONLY,
  NOT_VERIFIED,
  approxWait,
  hiddenReasonText,
  hiddenSummary,
  listedUntilText,
} from "./marketText";
import { useMarketPage } from "../../queries/shakedex";
import { useActiveProfile } from "../../queries/wallet";
import { displayName } from "../../lib/idn";
import { mapError } from "../../lib/errors";
import { formatHns } from "../../lib/utils";
import type { ImportSource, MarketRow } from "../../types";

function PriceCell({ row }: { row: MarketRow }) {
  if (row.verdict.verdict !== "buyable") return <span className="text-gray-500">—</span>;
  return (
    <div>
      <div>{formatHns(row.currentPrice)} HNS</div>
      {row.kind === "reverseAuction" && (
        <div className="text-xs text-gray-500">
          {row.nextPrice != null && row.nextValidInSecs != null && (
            <>
              next {formatHns(row.nextPrice)} in {approxWait(row.nextValidInSecs)} ·{" "}
            </>
          )}
          floor {formatHns(row.floorPrice)}
        </div>
      )}
      {row.kind === "reverseAuction" && row.steps.length > 0 && (
        <Disclosure
          summary={`All ${row.steps.length} price steps`}
          className="text-xs"
          testId="price-steps-toggle"
        >
          <ul className="space-y-0.5" data-testid="price-steps">
            {row.steps.map((s, i) => {
              // Only buyable rows reach here, and `market_row` gives each of
              // their steps a wait; a missing one would read as "now".
              const wait = s.validInSecs ?? 0;
              return (
                <li key={i}>
                  {formatHns(s.price)} HNS — {wait === 0 ? "now" : `in ${approxWait(wait)}`}
                </li>
              );
            })}
          </ul>
        </Disclosure>
      )}
    </div>
  );
}

/** Where a listed row came from: the market page, or an import. */
type Origin = "market" | "file" | "link";

/** An imported row and how it was imported. */
interface ImportedRow {
  row: MarketRow;
  origin: Exclude<Origin, "market">;
}

const ORIGIN_BADGE: Record<Exclude<Origin, "market">, string> = {
  file: "From file",
  link: "From LearnHNS link",
};

interface ListingsTableProps {
  rows: { row: MarketRow; origin: Origin }[];
}

function ListingsTable({ rows }: ListingsTableProps) {
  return (
    <table className="w-full text-sm">
      <thead>
        <tr className="text-left text-gray-500 border-b">
          <th className="py-1 pr-4">Name</th>
          <th className="py-1 pr-4">Type</th>
          <th className="py-1 pr-4">Price</th>
          <th className="py-1 pr-4">Notes</th>
          <th className="py-1">
            <span className="sr-only">Actions</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {rows.map(({ row, origin }, i) => {
          const listedUntil = row.expiresAt != null ? listedUntilText(row.expiresAt) : null;
          return (
            <tr
              key={`${row.name}-${i}`}
              className="border-t border-gray-100 hover:bg-gray-50"
              data-testid="market-row"
            >
              <td className="py-1 pr-4 font-mono">.{displayName(row.name)}</td>
              <td className="py-1 pr-4">
                {row.kind === "reverseAuction" ? "Reverse auction" : "Buy Now"}
              </td>
              <td className="py-1 pr-4">
                <PriceCell row={row} />
              </td>
              <td className="py-1 pr-4 text-gray-500">
                <div className="flex flex-wrap gap-1">
                  {origin !== "market" && <Badge>{ORIGIN_BADGE[origin]}</Badge>}
                  {row.verdict.verdict === "buyable" && row.verdict.warnExpiry && (
                    <Badge
                      variant="warning"
                      title="The name expires within about a month of the earliest finalize."
                    >
                      Expires soon
                    </Badge>
                  )}
                  {listedUntil && <span>{listedUntil}</span>}
                  {row.verdict.verdict === "hidden" && row.verdict.kind !== "unverified" && (
                    <span>{hiddenReasonText(row.verdict)}</span>
                  )}
                </div>
              </td>
              <td className="py-1 text-right">
                {row.verdict.verdict === "hidden" && row.verdict.kind === "unverified" && (
                  <span className="text-xs text-gray-500">{NOT_VERIFIED}</span>
                )}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

/**
 * What an empty market page says. A listing the node could not check is not
 * known to be unbuyable, so the page does not claim there are none.
 */
function emptyPageText(pageCount: number, couldNotCheck: number): string {
  const where = pageCount > 1 ? "on this page" : "right now";
  if (couldNotCheck > 0) {
    return `No listing ${where} could be shown as buyable; ${couldNotCheck} could not be checked against your node.`;
  }
  return `No buyable listings ${where}.`;
}

export default function MarketPage() {
  const [pageNumber, setPageNumber] = useState(1);
  const market = useMarketPage(pageNumber);
  const { data: profile } = useActiveProfile();
  const [imported, setImported] = useState<ImportedRow[]>([]);

  const onImported = (row: MarketRow, source: ImportSource["kind"]) =>
    setImported((prev) => [...prev, { row, origin: source === "link" ? "link" : "file" }]);

  const page = market.data;
  const summary = page ? hiddenSummary(page.hidden) : null;

  return (
    <div>
      <PageHeader
        title="Market"
        subtitle="Names listed through Shakedex. Every listing is checked against your node."
      />
      <div className="space-y-6">
        {market.isLoading && <p className="text-sm text-gray-500">Loading listings…</p>}
        {market.isError && (
          <Alert tone="error" title="Could not load the market">
            {mapError(market.error)}
          </Alert>
        )}
        {page && !page.networkHasMarket && (
          <Alert tone="info">
            There is no LearnHNS Market for this network. You can still check a listing file or
            pasted text below.
          </Alert>
        )}
        {page && page.networkHasMarket && page.rows.length === 0 && (
          <p className="text-sm text-gray-500">
            {emptyPageText(page.pageCount, page.hidden.couldNotCheck)}
          </p>
        )}
        {page && page.networkHasMarket && page.rows.length > 0 && (
          <ListingsTable rows={page.rows.map((row) => ({ row, origin: "market" as const }))} />
        )}
        {page && page.networkHasMarket && page.pageCount > 1 && (
          <div className="flex items-center gap-3 text-sm">
            <Button
              size="sm"
              data-testid="market-prev"
              disabled={page.page <= 1}
              onClick={() => setPageNumber(page.page - 1)}
            >
              Previous
            </Button>
            <span data-testid="market-page-of" className="text-gray-500">
              Page {page.page} of {page.pageCount}
            </span>
            <Button
              size="sm"
              data-testid="market-next"
              disabled={page.page >= page.pageCount}
              onClick={() => setPageNumber(page.page + 1)}
            >
              Next
            </Button>
          </div>
        )}
        {summary && page && (
          <Disclosure summary={summary} testId="hidden-summary">
            <ul className="space-y-1 text-sm">
              {page.hiddenRows.map((h, i) => (
                <li key={`${h.name}-${i}`} data-testid="hidden-row">
                  <span className="font-mono">
                    {h.name === null ? "Unnamed listing" : `.${displayName(h.name)}`}
                  </span>{" "}
                  <span className="text-gray-500">{hiddenReasonText(h.reason)}</span>
                </li>
              ))}
            </ul>
          </Disclosure>
        )}
        {imported.length > 0 && (
          <div>
            <h3 className="text-sm font-semibold text-gray-700 mb-1">Imported listings</h3>
            {/* Each import was verified on its own; the market page's state
                (loading, down, SPV) says nothing about it. */}
            <ListingsTable rows={imported} />
          </div>
        )}
        <div>
          <h3 className="text-sm font-semibold text-gray-700 mb-2">Import a listing</h3>
          <ImportListing
            onImported={onImported}
            linkRefusal={
              profile != null && profile.network !== "mainnet" ? MARKET_MAINNET_ONLY : null
            }
          />
        </div>
      </div>
    </div>
  );
}
