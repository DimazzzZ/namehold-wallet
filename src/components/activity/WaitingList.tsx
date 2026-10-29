import type { MergedRow } from "../../lib/activity";
import { useEffect, useState } from "react";
import { formatDate, formatSince } from "../../lib/utils";
import { PURCHASE_ACTION } from "../../queries/shakedex";
import { SPEED_UP_ACTION } from "../../queries/speedUp";
import { shortTxid, SpeedUpButton, useSpeedUpReason } from "../SpeedUp";
import { Tooltip } from "../ui/Tooltip";
import { rowBadge, rowIso, waitingSentence } from "./activityText";
import { NameButton, TxLink } from "./cells";

/** Sent, not in a block yet. Rows leave once mined. */
export function WaitingList({
  rows,
  onNameClick,
  onTxClick,
}: {
  rows: MergedRow[];
  onNameClick: (name: string) => void;
  onTxClick: (txid: string) => void;
}) {
  // One clock for the list: "sent 14 min ago" moves on every minute.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);
  return (
    <section className="rounded-md border border-gray-200 bg-white" data-testid="activity-waiting">
      <div className="flex items-center justify-between gap-3 px-4 py-3 border-b border-gray-100">
        <h3 className="text-sm font-semibold text-gray-900">
          Waiting for a block <span className="font-normal text-gray-500"> · {rows.length}</span>
        </h3>
        <span className="text-xs text-gray-500">Sent; they leave this list once mined.</span>
      </div>
      <ul>
        {rows.map((row, i) => (
          <WaitingRow
            key={row.key}
            row={row}
            first={i === 0}
            now={now}
            onNameClick={onNameClick}
            onTxClick={onTxClick}
          />
        ))}
      </ul>
    </section>
  );
}

/** Speed up for our own sent draft; a child can't be sped up again. */
function speedUpFor(row: MergedRow): { draftId: string; disabledReason?: string } | null {
  if (!row.draftId) return null; // e.g. an incoming payment in the mempool
  if (row.action === SPEED_UP_ACTION) {
    return { draftId: row.draftId, disabledReason: "A speed-up can't be sped up again" };
  }
  // R16: only once the node holds it; a broadcast_pending one is not in its mempool yet.
  if (row.status !== "broadcasted") return null;
  return row.spedUpBy
    ? { draftId: row.draftId, disabledReason: `Already sped up by ${shortTxid(row.spedUpBy)}` }
    : { draftId: row.draftId };
}

function WaitingRow({
  row,
  first,
  now,
  onNameClick,
  onTxClick,
}: {
  row: MergedRow;
  first: boolean;
  now: number;
  onNameClick: (name: string) => void;
  onTxClick: (txid: string) => void;
}) {
  const s = waitingSentence(row);
  const speedUp = speedUpFor(row);
  const batchFirst = row.nameList && row.nameList.length > 1 ? row.nameList[0]! : null;
  return (
    <li
      className={`flex items-center gap-3 px-4 py-3 ${first ? "" : "border-t border-gray-100"}`}
      data-testid="activity-waiting-row"
    >
      <span className="h-2 w-2 rounded-full bg-amber-500 shrink-0" />
      <div className="flex-1 min-w-0">
        <div className="text-sm text-gray-900">
          {s.lead}
          {s.name &&
            (batchFirst ? (
              <NameButton name={batchFirst} label={s.name} onClick={onNameClick} />
            ) : (
              <NameButton name={row.name!} onClick={onNameClick} />
            ))}
          <span className="text-gray-500">{s.tail}</span>
        </div>
        {row.spedUpBy && (
          <div className="text-xs text-gray-500 font-mono" data-testid="activity-sped-up-by">
            Sped up by {shortTxid(row.spedUpBy)}
          </div>
        )}
        {row.speedsUp && (
          <div className="text-xs text-gray-500 font-mono" data-testid="activity-speeds-up">
            Speeds up {shortTxid(row.speedsUp)}
          </div>
        )}
        {speedUp && !speedUp.disabledReason && <SpeedUpReasonLine draftId={speedUp.draftId} />}
      </div>
      {row.txid && <TxLink txid={row.txid} onClick={onTxClick} />}
      {row.sortTs > 0 && (
        <Tooltip
          content={`Sent ${formatDate(rowIso(row))} UTC. ${rowBadge(row).hint}`}
          className="inline-flex"
        >
          <span
            className="text-xs text-gray-500 whitespace-nowrap"
            data-testid="activity-waiting-sent"
          >
            sent {formatSince(rowIso(row), now)}
          </span>
        </Tooltip>
      )}
      {speedUp && (
        <SpeedUpButton
          draftId={speedUp.draftId}
          isPurchase={row.action === PURCHASE_ACTION}
          disabledReason={speedUp.disabledReason}
        />
      )}
    </li>
  );
}

/** The backend's refusal, visible under the row instead of only on hover. */
function SpeedUpReasonLine({ draftId }: { draftId: string }) {
  const { reason } = useSpeedUpReason(draftId);
  if (!reason) return null;
  return (
    <div className="text-xs text-gray-500" data-testid="activity-speed-up-refused">
      Speed up isn't available right now: {reason}
    </div>
  );
}
