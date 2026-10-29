import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { draftActionsForStatus, type MergedRow } from "../../lib/activity";
import { isSignerUnlockedFor } from "../../lib/signer";
import { mapError, StagedError } from "../../lib/errors";
import { invalidatePurchaseViews, PURCHASE_ACTION } from "../../queries/shakedex";
import { useDeleteTxDraft, useExecuteDraft, useSignerSession } from "../../queries/wallet";
import { useUiStore } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Tooltip } from "../ui/Tooltip";
import { finishSentence } from "./activityText";
import { NameButton, StatusCell, TxLink } from "./cells";

/** Drafts the user still has to send, retry or discard. */
export function ToFinishList({
  rows,
  profileId,
  onNameClick,
  onTxClick,
}: {
  rows: MergedRow[];
  profileId: string | null;
  onNameClick: (name: string) => void;
  onTxClick: (txid: string) => void;
}) {
  return (
    <section
      className="rounded-md border border-gray-200 bg-white"
      data-testid="activity-to-finish"
    >
      <div className="flex items-center justify-between gap-3 px-4 py-3 border-b border-gray-100">
        <h3 className="text-sm font-semibold text-gray-900">
          To finish <span className="font-normal text-gray-500"> · {rows.length}</span>
        </h3>
        <span className="text-xs text-gray-500">Nothing below is on-chain.</span>
      </div>
      <ul>
        {rows.map((row, i) => (
          <ToFinishRow
            key={row.key}
            row={row}
            first={i === 0}
            profileId={profileId}
            onNameClick={onNameClick}
            onTxClick={onTxClick}
          />
        ))}
      </ul>
    </section>
  );
}

function ToFinishRow({
  row,
  first,
  profileId,
  onNameClick,
  onTxClick,
}: {
  row: MergedRow;
  first: boolean;
  profileId: string | null;
  onNameClick: (name: string) => void;
  onTxClick: (txid: string) => void;
}) {
  const { data: signer } = useSignerSession();
  const execDraft = useExecuteDraft();
  const deleteDraft = useDeleteTxDraft();
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const [busy, setBusy] = useState<"execute" | "discard" | null>(null);
  const actions = draftActionsForStatus(row.status);
  const s = finishSentence(row);
  const batchFirst = row.nameList && row.nameList.length > 1 ? row.nameList[0]! : null;

  const handleExecute = async () => {
    if (!row.draftId || !profileId) return;
    setBusy("execute");
    try {
      const result = await execDraft.run(
        row.draftId,
        profileId,
        isSignerUnlockedFor(signer, profileId),
      );
      showToast(`Broadcast ${result.txid.slice(0, 12)}…`, "success");
    } catch (e) {
      const stage = e instanceof StagedError ? e.stage : undefined;
      const prefix = stage === "broadcast" ? "Broadcast failed" : "Signing failed";
      showToast(`${prefix}: ${mapError(e)}`, "error");
    } finally {
      // A purchase's send, whichever way it ended, moves its purchase record.
      if (row.action === PURCHASE_ACTION) invalidatePurchaseViews(qc);
      setBusy(null);
    }
  };

  const handleDiscard = async () => {
    if (!row.draftId) return;
    if (
      !window.confirm(
        "Discard this draft? Any coins it reserved will be freed. This cannot be undone.",
      )
    ) {
      return;
    }
    setBusy("discard");
    try {
      await deleteDraft.mutateAsync(row.draftId);
      showToast("Draft discarded", "success");
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      // The discard deleted the purchase record Owned Names and the market read.
      if (row.action === PURCHASE_ACTION) invalidatePurchaseViews(qc);
      setBusy(null);
    }
  };

  return (
    <li
      className={`flex items-center gap-4 px-4 py-3 ${first ? "" : "border-t border-gray-100"}`}
      data-testid="activity-to-finish-row"
    >
      <div className="flex-1 min-w-0">
        <div className="text-sm text-gray-900">
          {s.lead}
          {s.name &&
            (row.nameList && row.nameList.length > 1 ? (
              <NameButton name={batchFirst!} label={s.name} onClick={onNameClick} />
            ) : (
              <NameButton name={row.name!} onClick={onNameClick} />
            ))}
          {s.tail} <StatusCell row={row} className="text-[11px] align-middle" />
        </div>
        <div
          className={`mt-0.5 text-xs [text-wrap:pretty] ${
            s.detailTone === "error" ? "text-red-700" : "text-gray-500"
          }`}
          data-testid={row.lostReason ? "activity-lost-reason" : undefined}
        >
          {s.detail}
          {row.txid && (
            <>
              {" "}
              <TxLink txid={row.txid} onClick={onTxClick} />
            </>
          )}
        </div>
      </div>
      <div className="flex gap-1 shrink-0">
        {actions.execute && (
          <Tooltip content={actions.execute}>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void handleExecute()}
              loading={busy === "execute"}
              disabled={busy !== null || execDraft.pending}
              data-testid="activity-draft-execute"
            >
              {actions.execute}
            </Button>
          </Tooltip>
        )}
        {actions.discard && (
          <Tooltip content="Discard draft">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void handleDiscard()}
              loading={busy === "discard"}
              disabled={busy !== null || deleteDraft.isPending}
              data-testid="activity-draft-discard"
            >
              Discard
            </Button>
          </Tooltip>
        )}
      </div>
    </li>
  );
}
