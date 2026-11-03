import { MAX_BATCH, type NameBatchActions } from "../../hooks/useNameBatchActions";
import { Button } from "../ui/Button";
import { FeeRateOverride } from "../ui/FeeRateOverride";
import { Input } from "../ui/Input";
import { Tooltip } from "../ui/Tooltip";

/** The batch bar for the selected names. Secondary buttons: Look up is the page's primary. */
export function NameBatchBar({
  batch,
  transferBlockers = [],
  onSelectTransferable,
  onDropBlockers,
  transferableLeft = 0,
}: {
  batch: NameBatchActions;
  /** Selected names that refuse a transfer, with the backend's reason (or none yet). */
  transferBlockers?: { name: string; reason: string }[];
  /** Replace the selection with every shown name that can be transferred. */
  onSelectTransferable?: () => void;
  /** Drop every selected name that cannot be transferred, keep the rest selected. */
  onDropBlockers?: () => void;
  /** How many shown names can be transferred and are not yet selected. */
  transferableLeft?: number;
}) {
  const { eligibility: e, run, selected, recipient, setRecipient, progress } = batch;
  const batches = Math.ceil(selected.size / MAX_BATCH);
  const blockerText =
    transferBlockers.length > 0
      ? `Not transferable: ${transferBlockers
          .slice(0, 3)
          .map((b) => `${b.name} (${b.reason})`)
          .join(
            ", ",
          )}${transferBlockers.length > 3 ? ` and ${transferBlockers.length - 3} more` : ""}`
      : null;
  const actions = [
    {
      key: "renew",
      label: "Renew Selected",
      ok: e.canRenew,
      why: "All selected names must be renewable now",
      go: run.renew,
    },
    {
      key: "reveal",
      label: "Reveal Selected",
      ok: e.canReveal,
      why: "All selected names must be in the REVEAL phase",
      go: run.reveal,
    },
    {
      key: "redeem",
      label: "Redeem Selected",
      ok: e.canRedeem,
      why: "All selected names must have redeemable losing bids",
      go: run.redeem,
    },
    {
      key: "finalize",
      label: "Finalize Selected",
      ok: e.canFinalize,
      why: "All selected names must have a transfer ready to finalize",
      go: run.finalize,
    },
  ];
  const transferWhy = !e.canTransfer
    ? (blockerText ?? "Capabilities are still loading for some selected names")
    : !recipient.trim()
      ? "Enter a recipient address"
      : undefined;
  const seriesNote =
    batches > 1
      ? `${selected.size} names go out as ${batches} transactions of up to ${MAX_BATCH}, one confirm for all`
      : null;
  return (
    <div
      className="bg-blue-50 border border-blue-200 rounded-md px-3 py-2.5 flex flex-col gap-2"
      data-testid="batch-action-bar"
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-semibold text-blue-900">{selected.size} selected</span>
        {actions.map((a) => (
          <Tooltip key={a.key} content={a.ok ? undefined : a.why}>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void a.go()}
              disabled={!a.ok}
              data-testid={`batch-${a.key}-btn`}
            >
              {a.label}
            </Button>
          </Tooltip>
        ))}
        <span className="h-5 w-px bg-blue-200" aria-hidden />
        <Input
          className="w-56 font-mono"
          placeholder="Transfer to hs1q… / rs1q…"
          value={recipient}
          onChange={(ev) => setRecipient(ev.target.value)}
          data-testid="batch-transfer-recipient-input"
        />
        <Tooltip content={transferWhy}>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => void run.transfer()}
            disabled={!!transferWhy}
            data-testid="batch-transfer-btn"
          >
            Transfer Selected
          </Button>
        </Tooltip>
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto"
          onClick={() => {
            batch.clear();
            setRecipient("");
          }}
        >
          Clear
        </Button>
      </div>
      {progress && (
        <div
          className="text-sm font-medium text-blue-900"
          role="status"
          data-testid="batch-progress"
        >
          {progress.step === "building" ? "Building" : "Signing and sending"} {progress.action}{" "}
          {progress.total > 1
            ? `transaction ${progress.index} of ${progress.total}`
            : "transaction"}
          …
        </div>
      )}
      {(transferWhy || onSelectTransferable) && (
        <div
          className="flex flex-wrap items-center gap-2 text-xs text-blue-900"
          data-testid="batch-transfer-help"
        >
          {transferWhy && !e.canTransfer && <span>{transferWhy}</span>}
          {transferBlockers.length > 0 && onDropBlockers && (
            <Button
              size="sm"
              variant="ghost"
              onClick={onDropBlockers}
              data-testid="batch-drop-blockers"
            >
              Drop {transferBlockers.length} not transferable, keep{" "}
              {selected.size - transferBlockers.length}
            </Button>
          )}
          {seriesNote && <span>{seriesNote}</span>}
          {onSelectTransferable && (
            <Button
              size="sm"
              variant="ghost"
              onClick={onSelectTransferable}
              disabled={transferableLeft === 0}
              data-testid="batch-select-transferable"
            >
              Select all {transferableLeft} transferable
            </Button>
          )}
        </div>
      )}
      <FeeRateOverride
        value={batch.feeRate}
        onChange={batch.setFeeRate}
        label="Fee rate override"
      />
    </div>
  );
}
