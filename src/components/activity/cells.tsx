/** Cell renderers shared by Activity's lists and the Wallet's Recent activity card. */
import { useState } from "react";
import type { MergedRow } from "../../lib/activity";
import { displayName } from "../../lib/idn";
import { amountTone, formatDateShort, formatHns, formatHnsShort } from "../../lib/utils";
import { shortTxid } from "../SpeedUp";
import { Badge } from "../ui/Badge";
import { Tooltip } from "../ui/Tooltip";
import { ACTION_META, FALLBACK_META, rowBadge, rowIso, showsNameValue } from "./activityText";

const NAME_BUTTON =
  "font-mono text-gray-900 hover:text-blue-700 hover:underline cursor-pointer text-left";
const LINK = "font-mono text-blue-600 hover:text-blue-700 hover:underline cursor-pointer";

/** A name that opens the name's surface. */
export function NameButton({
  name,
  label,
  onClick,
}: {
  name: string;
  /** Text to show instead of `.name` (a batch's ".a + 2 more"). */
  label?: string;
  onClick: (name: string) => void;
}) {
  return (
    <Tooltip content="View name info" className="inline-flex align-baseline">
      <button
        type="button"
        className={NAME_BUTTON}
        onClick={() => onClick(name)}
        data-testid="activity-name-info-link"
      >
        {label ?? `.${displayName(name)}`}
      </button>
    </Tooltip>
  );
}

/** The row's name: one button, or for a batch the first name's ".a + N more". */
export function RowName({
  row,
  onNameClick,
}: {
  row: MergedRow;
  onNameClick: (n: string) => void;
}) {
  if (row.nameList && row.nameList.length > 1) {
    return (
      <NameButton
        name={row.nameList[0]!}
        label={`.${displayName(row.nameList[0]!)} + ${row.nameList.length - 1} more`}
        onClick={onNameClick}
      />
    );
  }
  return row.name ? <NameButton name={row.name} onClick={onNameClick} /> : null;
}

/** A batch row's ".first + N more" that unfolds every name as its own link. */
export function BatchNames({
  row,
  onNameClick,
}: {
  row: MergedRow;
  onNameClick: (n: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const names = row.nameList ?? [];
  return (
    <span className="inline-flex flex-col">
      <span>
        <Tooltip content={open ? "Collapse" : "Expand"} className="inline-flex">
          <button
            type="button"
            className="font-mono text-gray-900 hover:underline cursor-pointer"
            onClick={() => setOpen((v) => !v)}
            data-testid="activity-batch-summary-toggle"
          >
            .{displayName(names[0] ?? "")}{" "}
            <span className="font-sans text-blue-600">+ {names.length - 1} more</span>
          </button>
        </Tooltip>
      </span>
      {open && (
        <span className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-xs">
          {names.map((n) => (
            <Tooltip key={n} content="View name info" className="inline-flex">
              <button
                type="button"
                className={LINK}
                onClick={() => onNameClick(n)}
                data-testid="activity-name-info-link"
              >
                .{displayName(n)}
              </button>
            </Tooltip>
          ))}
        </span>
      )}
    </span>
  );
}

/** Action badge + the row's name (batches unfold). */
export function WhatCell({
  row,
  onNameClick,
}: {
  row: MergedRow;
  onNameClick: (n: string) => void;
}) {
  const meta = ACTION_META[row.action] ?? FALLBACK_META;
  return (
    <span className="inline-flex flex-wrap items-baseline gap-x-2">
      <Badge variant={meta.variant}>{meta.label}</Badge>
      {row.nameList && row.nameList.length > 1 ? (
        <BatchNames row={row} onNameClick={onNameClick} />
      ) : row.name ? (
        <NameButton name={row.name} onClick={onNameClick} />
      ) : null}
    </span>
  );
}

/** Date in lists; "Pending" before the row has a time. */
export function DateCell({ row }: { row: MergedRow }) {
  return <>{row.sortTs > 0 ? formatDateShort(rowIso(row)) : "Pending"}</>;
}

const LOCKED_VALUE_TAIL =
  "is the name's locked value, carried onto your own new coin — not spent. Only the network fee leaves your wallet.";

/** Signed short amount with the exact value on hover; a covenant row's locked value. */
export function AmountCell({ row }: { row: MergedRow }) {
  if (showsNameValue(row)) {
    return (
      <Tooltip
        content={
          <>
            <span className="font-medium">{formatHns(row.nameValueDoos!)} HNS</span>{" "}
            {LOCKED_VALUE_TAIL}
          </>
        }
        className="inline-flex"
      >
        <span className="text-gray-500 tabular-nums">
          ⤷&nbsp;{formatHnsShort(row.nameValueDoos!)}
        </span>
      </Tooltip>
    );
  }
  const tone = amountTone(row);
  const toneClass =
    tone === "income" ? "text-green-600" : tone === "spend" ? "text-gray-900" : "text-gray-500";
  const sign = tone === "income" ? "+" : tone === "spend" ? "−" : "";
  const abs = Math.abs(row.valueDoos);
  return (
    <Tooltip content={`${sign}${formatHns(abs)} HNS`} className="inline-flex">
      <span className={`${toneClass} tabular-nums`} data-testid="activity-amount">
        {sign}
        {formatHnsShort(abs)}
      </span>
    </Tooltip>
  );
}

/** The row's status badge with what it means on hover. */
export function StatusCell({ row, className }: { row: MergedRow; className?: string }) {
  const b = rowBadge(row);
  return (
    <Badge variant={b.variant} title={b.hint} className={className}>
      {b.label}
    </Badge>
  );
}

/** A short txid that opens the transaction's info. */
export function TxLink({
  txid,
  onClick,
  testId = "activity-tx-info-link",
}: {
  txid: string;
  onClick: (txid: string) => void;
  testId?: string;
}) {
  return (
    <Tooltip content={`View transaction info — ${txid}`} className="inline-flex align-baseline">
      <button
        type="button"
        className={`${LINK} text-xs`}
        onClick={() => onClick(txid)}
        data-testid={testId}
      >
        {shortTxid(txid)}
      </button>
    </Tooltip>
  );
}

export { LOCKED_VALUE_TAIL };
