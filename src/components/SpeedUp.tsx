/**
 * Speed up (spec 2026-10-21 R16): the button every sent, unconfirmed
 * transaction of ours shows (Activity, and an Unconfirmed purchase in the
 * Market), disabled with its reason when the backend refuses, and the form
 * it opens. The form shows the child's numbers as the backend computes them;
 * signing goes through the secure confirmation like any draft.
 */
import { useId, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "./ui/Button";
import { Dialog } from "./ui/Dialog";
import { Input } from "./ui/Input";
import { Tooltip } from "./ui/Tooltip";
import {
  refusalText,
  useBuildSpeedUp,
  useSpeedUpCheck,
  useSpeedUpPreview,
} from "../queries/speedUp";
import { invalidatePurchaseViews } from "../queries/shakedex";
import {
  useActiveProfile,
  useDeleteTxDraft,
  useExecuteDraft,
  useSignerSession,
} from "../queries/wallet";
import { isSignerUnlockedFor } from "../lib/signer";
import { mapError, StagedError } from "../lib/errors";
import { formatHns } from "../lib/utils";
import { useUiStore } from "../stores/ui";
import type { SpeedUpSummary, SpeedUpTargetSource } from "../types";

/** A rate in doos per kB (hsd's `getRate`) as doos per vbyte, as the backend writes it. */
export function rateText(perKb: number): string {
  const whole = Math.floor(perKb / 1000);
  const hundredths = Math.floor((perKb % 1000) / 10);
  return `${whole}.${String(hundredths).padStart(2, "0")} doos/vB`;
}

const SOURCE_TEXT: Record<SpeedUpTargetSource, string> = {
  estimate: "your node's estimate",
  setting: "your fee rate setting",
  fallback: "your node has no fee estimate yet; the minimum rate",
  typed: "the rate you chose",
};

/** The parent's mempool deadline in the viewer's local time. */
export function deadlineText(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleString("en-US", {
    year: "numeric",
    month: "long",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** R11's sentence on the deadline, which the form repeats. */
export function deadlineSentence(unixSecs: number): string {
  return `Speeding up does not extend the 72 hours your node keeps the original transaction: if it is not mined by ${deadlineText(unixSecs)}, both are dropped and nothing is paid by either.`;
}

/** The short form of a txid that rows name each other by. */
export function shortTxid(txid: string): string {
  return `${txid.slice(0, 10)}…`;
}

/**
 * "Speed up" for one sent, unconfirmed draft: enabled when the backend
 * offers it, otherwise disabled with the backend's reason, never hidden.
 */
export function SpeedUpButton({
  draftId,
  isPurchase = false,
  size = "sm",
}: {
  draftId: string;
  /** A purchase's send moves its purchase record. */
  isPurchase?: boolean;
  size?: "sm" | "md";
}) {
  const [open, setOpen] = useState(false);
  const check = useSpeedUpCheck(draftId);
  const reasonId = useId();
  const reason = check.isError
    ? refusalText(check.error)
    : check.data && !check.data.available
      ? (check.data.reason ?? "This transaction cannot be sped up now.")
      : null;
  const available = check.data?.available === true;
  return (
    <>
      <Tooltip content={reason ?? "Pay a higher fee for it through a second transaction"}>
        <Button
          size={size}
          variant="secondary"
          data-testid="speed-up-button"
          disabled={!available}
          loading={check.isLoading}
          aria-describedby={reason ? reasonId : undefined}
          onClick={() => setOpen(true)}
        >
          Speed up
        </Button>
      </Tooltip>
      {reason && (
        <span id={reasonId} className="sr-only" data-testid="speed-up-reason">
          {reason}
        </span>
      )}
      {open && (
        <SpeedUpDialog draftId={draftId} isPurchase={isPurchase} onClose={() => setOpen(false)} />
      )}
    </>
  );
}

/** Whole doos per vbyte from the form's text; null for empty, NaN for not a rate. */
function parseRate(raw: string): number | null {
  const t = raw.trim();
  if (!t) return null;
  return /^\d+$/.test(t) && Number(t) > 0 ? Number(t) : Number.NaN;
}

export function SpeedUpDialog({
  draftId,
  isPurchase,
  onClose,
}: {
  draftId: string;
  isPurchase: boolean;
  onClose: () => void;
}) {
  const [rateRaw, setRateRaw] = useState("");
  const rate = parseRate(rateRaw);
  const rateValid = rate === null || !Number.isNaN(rate);
  const preview = useSpeedUpPreview(draftId, rateValid ? rate : null, rateValid);
  const build = useBuildSpeedUp();
  const exec = useExecuteDraft();
  const del = useDeleteTxDraft();
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const showToast = useUiStore((s) => s.showToast);
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!profile) return;
    setBusy(true);
    let childId: string | null = null;
    try {
      const child = await build.mutateAsync({ draftId, feeRate: preview.settledRate });
      childId = child.id;
      const sent = await exec.run(child.id, profile.id, isSignerUnlockedFor(signer, profile.id));
      showToast(`Speed up sent ${sent.txid.slice(0, 12)}…`, "success");
      onClose();
    } catch (e) {
      // Declined or failed before signing: discard the child so its parent
      // is not held as "already being sped up".
      if (childId && e instanceof StagedError && e.stage === "sign") {
        void del.mutateAsync(childId).catch(() => {});
      }
      const stage = e instanceof StagedError ? e.stage : undefined;
      const prefix =
        stage === "broadcast" ? "Speed up not sent" : childId ? "Not signed" : "Speed up refused";
      showToast(
        `${prefix}: ${refusalText(mapError(e instanceof StagedError ? e.original : e))}`,
        "error",
      );
    } finally {
      if (isPurchase) invalidatePurchaseViews(qc);
      setBusy(false);
    }
  };

  return (
    <Dialog open onClose={onClose} title="Speed up">
      <div className="space-y-3 text-sm" data-testid="speed-up-dialog">
        <p className="text-gray-600 [text-wrap:pretty]">
          A second transaction spends this one's change and pays the fee for both, so a miner takes
          them together. The fee goes to the network only; nobody else is paid
          {isPurchase ? ", and the seller's price and the purchase itself are unchanged" : ""}. The
          original keeps its transaction id.
        </p>
        <label className="block">
          <span className="text-xs font-medium text-gray-700">Fee rate (doos/vB)</span>
          <Input
            inputSize="sm"
            inputMode="numeric"
            placeholder="your node's estimate"
            value={rateRaw}
            onChange={(e) => setRateRaw(e.target.value)}
            data-testid="speed-up-rate"
          />
        </label>
        {!rateValid && (
          <p className="text-xs text-red-700">Enter a whole number of doos per vbyte.</p>
        )}
        {preview.isError && (
          <p className="text-xs text-red-700" data-testid="speed-up-refusal">
            {refusalText(preview.error)}
          </p>
        )}
        {preview.data && <SpeedUpNumbers s={preview.data} />}
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            data-testid="speed-up-submit"
            disabled={!preview.data || !rateValid || preview.settling || busy}
            loading={busy}
            onClick={() => void submit()}
          >
            Review and sign
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function SpeedUpNumbers({ s }: { s: SpeedUpSummary }) {
  const rows: [string, string][] = [
    ["This transaction", `${s.parentSize} vbytes, fee ${formatHns(s.parentFeeDoos)} HNS`],
    ["Fee rate now", rateText(s.packageRateBefore)],
    ["Target", `${s.targetRatePerVbyte} doos/vB (${SOURCE_TEXT[s.targetSource]})`],
    ["Speed-up fee", `${formatHns(s.feeDoos)} HNS`],
    ["Fee rate with it", rateText(s.packageRateAfter)],
    ["Change left", `${formatHns(s.changeDoos)} HNS`],
    ["Mempool deadline", deadlineText(s.expiresAt)],
  ];
  return (
    <div data-testid="speed-up-numbers">
      <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-xs">
        {rows.map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="text-gray-500">{k}</dt>
            <dd className="font-mono tabular-nums text-gray-900">{v}</dd>
          </div>
        ))}
      </dl>
      <p className="mt-2 text-xs text-amber-800 [text-wrap:pretty]" data-testid="speed-up-deadline">
        {deadlineSentence(s.expiresAt)}
      </p>
    </div>
  );
}
