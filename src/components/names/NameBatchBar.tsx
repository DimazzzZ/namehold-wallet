import type { NameBatchActions } from "../../hooks/useNameBatchActions";
import { Button } from "../ui/Button";
import { FeeRateOverride } from "../ui/FeeRateOverride";
import { Input } from "../ui/Input";
import { Tooltip } from "../ui/Tooltip";

/** The batch bar for the selected names. Secondary buttons: Look up is the page's primary. */
export function NameBatchBar({ batch }: { batch: NameBatchActions }) {
  const { eligibility: e, run, selected, recipient, setRecipient } = batch;
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
    ? "All selected names must be transferable (owned, not mid-transfer)"
    : !recipient.trim()
      ? "Enter a recipient address"
      : undefined;
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
      <FeeRateOverride
        value={batch.feeRate}
        onChange={batch.setFeeRate}
        label="Fee rate override"
      />
    </div>
  );
}
