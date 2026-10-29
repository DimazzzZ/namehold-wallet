import { ArrowRight, ChevronDown, ChevronRight, Copy } from "lucide-react";
import { Icon } from "../ui/Icon";
import { useEffect, useState, type ReactNode } from "react";
import type { MergedRow } from "../../lib/activity";
import { writeText } from "../../lib/clipboard";
import { fromInteractiveChild } from "../../lib/rowClick";
import { formatDate, formatDateShort, formatHns } from "../../lib/utils";
import { useUiStore } from "../../stores/ui";
import { shortTxid } from "../SpeedUp";
import { Button } from "../ui/Button";
import { Tooltip } from "../ui/Tooltip";
import { LOCKED_VALUE_TAIL, AmountCell, DateCell, StatusCell, TxLink, WhatCell } from "./cells";
import { rowBadge, rowIso, showsNameValue } from "./activityText";

export interface HistoryPager {
  page: number;
  totalPages: number;
  startRow: number;
  endRow: number;
  totalRows: number;
  onGoTo: (n: number) => void;
}

/** Finished transactions: 5 columns, a row click shows fee, block and txid. */
export function HistoryTable({
  rows,
  allRows,
  toolbar,
  note,
  emptyText,
  pager,
  resetKey,
  onNameClick,
  onBlockClick,
  onTxClick,
}: {
  /** The rows of this page. */
  rows: MergedRow[];
  /** Every merged row, to name the transaction a Speed up child speeds up. */
  allRows: MergedRow[];
  toolbar: ReactNode;
  note?: ReactNode;
  emptyText: string;
  pager: HistoryPager | null;
  /** Open rows close when this changes (the filters). */
  resetKey: string;
  onNameClick: (name: string) => void;
  onBlockClick: (height: number) => void;
  onTxClick: (txid: string) => void;
}) {
  const [open, setOpen] = useState<Set<string>>(new Set());
  useEffect(() => setOpen(new Set()), [resetKey]);
  const toggle = (key: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const byTxid = new Map(allRows.filter((r) => r.txid).map((r) => [r.txid!, r]));

  return (
    <section className="rounded-md border border-gray-200 bg-white" data-testid="activity-history">
      <div className="px-4 py-2.5 flex flex-wrap items-center justify-between gap-3 border-b border-gray-100">
        <h3 className="text-sm font-semibold text-gray-900">History</h3>
        <div className="flex flex-wrap items-center gap-2">{toolbar}</div>
      </div>
      {note}
      {rows.length === 0 ? (
        <div className="py-8 text-center text-sm text-gray-400">{emptyText}</div>
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-gray-500 border-b">
              <th className="py-1 pr-4 pl-4 w-[170px]">Date</th>
              <th className="py-1 pr-4">What</th>
              <th className="py-1 pr-4 text-right">Amount (HNS)</th>
              <th className="py-1 pr-4 w-[120px]">Status</th>
              <th className="py-1 w-6" />
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => {
              const isOpen = open.has(row.key);
              const parent = row.speedsUp ? byTxid.get(row.speedsUp) : undefined;
              return [
                <tr
                  key={row.key}
                  className={`border-t border-gray-100 hover:bg-gray-50 cursor-pointer ${
                    isOpen ? "bg-gray-50" : ""
                  }`}
                  onClick={(e) => {
                    if (!fromInteractiveChild(e)) toggle(row.key);
                  }}
                  data-testid="activity-history-row"
                >
                  <td className="py-1 pr-4 pl-4 text-xs text-gray-500 whitespace-nowrap">
                    <DateCell row={row} />
                  </td>
                  <td className="py-1 pr-4">
                    <WhatCell row={row} onNameClick={onNameClick} />
                    {row.speedsUp && (
                      <span className="ml-2 text-xs text-gray-500" data-testid="activity-speeds-up">
                        {parent && parent.sortTs > 0
                          ? `speeds up the send of ${formatDateShort(rowIso(parent))}`
                          : `speeds up ${shortTxid(row.speedsUp)}`}
                      </span>
                    )}
                    {row.spedUpBy && (
                      <span
                        className="ml-2 text-xs text-gray-500"
                        data-testid="activity-sped-up-by"
                      >
                        sped up
                      </span>
                    )}
                    {row.lostReason && (
                      <div className="text-xs text-red-700" data-testid="activity-lost-reason">
                        {row.lostReason}
                      </div>
                    )}
                  </td>
                  <td className="py-1 pr-4 text-right text-xs whitespace-nowrap">
                    <AmountCell row={row} />
                  </td>
                  <td className="py-1 pr-4">
                    <StatusCell row={row} />
                  </td>
                  <td className="py-1 text-gray-500">
                    <Tooltip
                      content={isOpen ? "Hide details" : "Show fee, block and transaction"}
                      className="inline-flex"
                    >
                      <button
                        type="button"
                        aria-expanded={isOpen}
                        aria-label="Show details"
                        className="px-1 hover:text-gray-700"
                        onClick={() => toggle(row.key)}
                        data-testid="activity-row-toggle"
                      >
                        <Icon icon={isOpen ? ChevronDown : ChevronRight} size={14} />
                      </button>
                    </Tooltip>
                  </td>
                </tr>,
                isOpen && (
                  <tr
                    key={`${row.key}:detail`}
                    className="border-t border-gray-100 hover:bg-gray-50 bg-gray-50"
                    data-testid="activity-row-detail"
                  >
                    <td colSpan={5} className="py-1 pl-[186px] pr-4">
                      <RowDetail row={row} onBlockClick={onBlockClick} onTxClick={onTxClick} />
                    </td>
                  </tr>
                ),
              ];
            })}
          </tbody>
        </table>
      )}
      {pager && pager.totalRows > 0 && <Pager {...pager} />}
    </section>
  );
}

function Pair({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span>
      <span className="text-gray-500">{label}</span> {children}
    </span>
  );
}

function RowDetail({
  row,
  onBlockClick,
  onTxClick,
}: {
  row: MergedRow;
  onBlockClick: (height: number) => void;
  onTxClick: (txid: string) => void;
}) {
  const showToast = useUiStore((s) => s.showToast);
  const [copied, setCopied] = useState(false);
  const covenant = showsNameValue(row);
  const sign = row.valueDoos > 0 ? "+" : row.valueDoos < 0 ? "−" : "";
  const copy = async () => {
    if (!row.txid) return;
    await writeText(row.txid);
    showToast("Transaction ID copied", "success");
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  return (
    <div className="pb-2 text-xs text-gray-700 space-y-1.5">
      <div className="flex flex-wrap gap-x-6 gap-y-1.5">
        <Pair label="Date">{formatDate(rowIso(row))}</Pair>
        <Pair label="Amount">
          {covenant
            ? `⤷ ${formatHns(row.nameValueDoos!)} HNS (locked value)`
            : `${sign}${formatHns(Math.abs(row.valueDoos))} HNS`}
        </Pair>
        <Pair label="Fee">{row.feeDoos == null ? "—" : `${formatHns(row.feeDoos)} HNS`}</Pair>
        <Pair label="Block">
          {row.height == null ? (
            "—"
          ) : (
            <Tooltip content="View block info" className="inline-flex">
              <button
                type="button"
                className="font-mono text-blue-600 hover:underline"
                onClick={() => onBlockClick(row.height!)}
                data-testid="activity-block-info-link"
              >
                #{row.height}
              </button>
            </Tooltip>
          )}
        </Pair>
        <Pair label="Txid">
          {row.txid ? (
            <>
              <TxLink txid={row.txid} onClick={onTxClick} />{" "}
              <Button
                size="sm"
                variant="ghost"
                className="!px-1 !py-0 text-xs"
                onClick={() => void copy()}
                data-testid="activity-copy-txid"
              >
                <span className="inline-flex items-center gap-1">
                  <Icon icon={Copy} size={12} />
                  {copied ? "Copied" : "Copy"}
                </span>
              </Button>
            </>
          ) : (
            "—"
          )}
        </Pair>
        {row.speedsUp && (
          <Pair label="Speeds up">
            <TxLink txid={row.speedsUp} onClick={onTxClick} testId="activity-speeds-up-link" />
          </Pair>
        )}
        {row.spedUpBy && (
          <Pair label="Sped up by">
            <TxLink txid={row.spedUpBy} onClick={onTxClick} testId="activity-sped-up-by-link" />
          </Pair>
        )}
      </div>
      {covenant && (
        <p className="text-gray-500 [text-wrap:pretty]">
          {formatHns(row.nameValueDoos!)} HNS{" "}
          {LOCKED_VALUE_TAIL.replace("leaves your wallet", "left your wallet")}
        </p>
      )}
      {rowBadge(row).hint && <p className="text-gray-500">{rowBadge(row).hint}</p>}
    </div>
  );
}

function Pager({ page, totalPages, startRow, endRow, totalRows, onGoTo }: HistoryPager) {
  const btn =
    "text-blue-600 hover:underline disabled:opacity-40 disabled:cursor-not-allowed disabled:no-underline";
  return (
    <div className="flex items-center justify-between px-4 py-2.5 border-t border-gray-100 text-[13px] text-gray-500">
      <div>
        Rows {startRow}–{endRow} of {totalRows}
      </div>
      {totalPages > 1 && (
        <div className="flex items-center gap-4">
          <button
            type="button"
            className={btn}
            disabled={page <= 1}
            onClick={() => onGoTo(page - 1)}
          >
            Newer
          </button>
          <span className="text-gray-600">
            Page {page} of {totalPages}
          </span>
          <button
            type="button"
            className={btn}
            disabled={page >= totalPages}
            onClick={() => onGoTo(page + 1)}
          >
            <span className="inline-flex items-center gap-1">
              Show older <Icon icon={ArrowRight} size={12} />
            </span>
          </button>
        </div>
      )}
    </div>
  );
}
