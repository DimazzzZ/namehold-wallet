import { useState } from "react";
import { Alert } from "../ui/Alert";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { FeeRateOverride } from "../ui/FeeRateOverride";
import { parseFeeRateArg } from "../../lib/feeRate";
import { mapError } from "../../lib/errors";
import { displayName } from "../../lib/idn";
import { useFinalizeAndSign, useKeptDraft } from "../../queries/shakedex";
import {
  useActiveProfile,
  useBroadcastTxDraft,
  useWithSigner,
  useWriteCapability,
} from "../../queries/wallet";
import { useUiStore } from "../../stores/ui";
import type { ShakedexListingSummary, ShakedexScheduleInput } from "../../types";
import {
  REVERSE_NEEDS_SCHEDULE,
  STEP_SIGNATURE_PERMANENCE,
  approxWait,
  listingStateLabel,
  parseStepPrice,
  scheduleProblem,
  scheduleWarning,
  shakedexRefusal,
  withKept,
} from "./marketText";

/** A finalize signed without a draft id to send (`ListingSummary.finalizeDraftId` null). */
const NO_FINALIZE_DRAFT =
  "The finalize was signed, but no transaction came back to send: My listings shows what is left to do.";

export interface FinalizeAndSignProps {
  open: boolean;
  listing: ShakedexListingSummary;
  onClose: () => void;
}

/**
 * A typed decimal number, or null when the field is blank or is not one
 * (hex, binary, exponent or any other notation `Number` would read): the
 * schedule is then incomplete. A number that is not whole is left to
 * `scheduleProblem`, which refuses it in its words, never cut down.
 */
function numberOf(text: string): number | null {
  const t = text.trim();
  return /^\d+(\.\d+)?$/.test(t) ? Number(t) : null;
}

/**
 * Minutes as typed to seconds. Rounded to a millionth so that a decimal
 * number of minutes (15.1) gives its whole seconds (906), not a float's
 * 906.0000000000001; a fraction of a second stays one.
 */
function minutesToSecs(min: number): number {
  return Number((min * 60).toFixed(6));
}

/**
 * Finalize & sign (R19, R20, R24): the price of a Buy Now, or the schedule
 * of a reverse auction (the listing's mode, chosen at Lock), is chosen here
 * and sent to the backend in that mode's shape only. The backend builds the
 * FINALIZE into the lock and asks in its secure window, which lists the fee
 * and every price with when it becomes valid, before it signs anything; the
 * signed FINALIZE is then sent.
 */
export function FinalizeAndSign({ open, listing, onClose }: FinalizeAndSignProps) {
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: writeCap } = useWriteCapability();
  const withSigner = useWithSigner();
  const finalizeAndSign = useFinalizeAndSign();
  const broadcast = useBroadcastTxDraft();
  const keptDraft = useKeptDraft();
  const [price, setPrice] = useState("");
  const [start, setStart] = useState("");
  const [floor, setFloor] = useState("");
  const [steps, setSteps] = useState("");
  const [intervalMin, setIntervalMin] = useState("");
  const [feeRateRaw, setFeeRateRaw] = useState("");
  const [busy, setBusy] = useState(false);
  const [signed, setSigned] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reverse = listing.mode === "reverseAuction";
  // R16, R6; no experimental flag: the listing exists already (R15).
  const gate = shakedexRefusal(profile?.kind, writeCap);

  // The form's pricing, in the listing's mode only (carry 8): a Buy Now's
  // one price and no schedule, or a reverse auction's schedule and no price.
  let pricing: { prices: { price: string }[]; schedule: ShakedexScheduleInput | null } | null =
    null;
  let problem: string | null = null;
  let warning: string | null = null;
  let summary: string | null = null;
  if (!reverse) {
    if (price !== "") {
      const p = parseStepPrice(price);
      if ("error" in p) problem = p.error;
      else pricing = { prices: [{ price }], schedule: null };
    }
  } else {
    const stepCount = numberOf(steps);
    const minutes = numberOf(intervalMin);
    if (start.trim() === "" || floor.trim() === "" || stepCount === null || minutes === null) {
      problem = REVERSE_NEEDS_SCHEDULE;
    } else {
      const schedule: ShakedexScheduleInput = {
        startPrice: start,
        floorPrice: floor,
        steps: stepCount,
        intervalSecs: minutesToSecs(minutes),
      };
      problem = scheduleProblem(schedule);
      if (problem === null) {
        pricing = { prices: [], schedule };
        warning = scheduleWarning(schedule.steps, schedule.intervalSecs);
        summary = `${schedule.steps} prices from ${start.trim()} to ${floor.trim()} HNS, one every ${minutes} min; the last becomes valid ${approxWait((schedule.steps - 1) * schedule.intervalSecs)} after the first.`;
      }
    }
  }

  const submit = async () => {
    if (!profile || !pricing) return;
    const args = pricing;
    setBusy(true);
    setError(null);
    let draftId: string | null;
    try {
      const done = await withSigner.run(profile.id, () =>
        finalizeAndSign.mutateAsync({
          listingId: listing.id,
          ...args,
          feeRate: parseFeeRateArg(feeRateRaw),
        }),
      );
      draftId = done.finalizeDraftId;
    } catch (e) {
      // Declined or refused: nothing was signed or written.
      setError(mapError(e));
      setBusy(false);
      return;
    }
    const id = draftId;
    if (!id) {
      // Signed and written, but no draft came back to send here.
      setSigned(true);
      setError(NO_FINALIZE_DRAFT);
      setBusy(false);
      return;
    }
    try {
      await broadcast.mutateAsync(id);
      showToast(
        `Finalize sent. The listing becomes ${listingStateLabel("listed", null)} once the finalize is mined.`,
      );
      onClose();
    } catch (e) {
      // The signed FINALIZE is not this dialog's to sign again: My listings
      // offers it, when the backend kept it.
      setSigned(true);
      setError(withKept(mapError(e, "broadcast"), await keptDraft(id)));
    } finally {
      setBusy(false);
    }
  };

  const field = (
    testId: string,
    label: string,
    value: string,
    onChange: (v: string) => void,
    inputMode: "decimal" | "numeric",
  ) => (
    <label className="block">
      <span className="text-gray-700">{label}</span>
      <input
        data-testid={testId}
        className="mt-1 block w-full rounded-md border border-gray-300 px-3 py-1.5 text-sm"
        inputMode={inputMode}
        value={value}
        disabled={busy || signed}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  );

  return (
    <Dialog
      open={open}
      onClose={() => !busy && onClose()}
      title={`Finalize & sign .${displayName(listing.name)}`}
    >
      <div className="space-y-3 text-sm text-gray-700">
        {reverse ? (
          <div className="grid grid-cols-2 gap-3">
            {field("fas-start", "Start price (HNS)", start, setStart, "decimal")}
            {field("fas-floor", "Floor price (HNS)", floor, setFloor, "decimal")}
            {field("fas-steps", "Number of prices (2 to 1000)", steps, setSteps, "numeric")}
            {field(
              "fas-interval",
              "Minutes between prices (at least 15)",
              intervalMin,
              setIntervalMin,
              "decimal",
            )}
          </div>
        ) : (
          field("fas-price", "Price (HNS)", price, setPrice, "decimal")
        )}
        {problem && <p className="text-xs text-red-600">{problem}</p>}
        {summary && (
          <p data-testid="fas-summary" className="text-gray-700">
            {summary}
          </p>
        )}
        {warning && <Alert tone="warning">{warning}</Alert>}
        <p>{STEP_SIGNATURE_PERMANENCE}</p>
        <p className="text-xs text-gray-500">
          The secure window lists the finalize fee and every price, with when it becomes valid,
          before anything is signed.
        </p>
        {!signed && (
          <FeeRateOverride value={feeRateRaw} onChange={setFeeRateRaw} label="Fee rate override" />
        )}
        {gate && <p className="text-xs text-gray-600">{gate}</p>}
        {error && (
          <Alert tone="error">
            <span data-testid="fas-error">{error}</span>
          </Alert>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Close
          </Button>
          {!signed && (
            <Button
              variant="primary"
              data-testid="fas-submit"
              disabled={busy || profile == null || gate !== null || pricing === null}
              onClick={submit}
            >
              {busy ? "Processing…" : "Finalize & sign"}
            </Button>
          )}
        </div>
      </div>
    </Dialog>
  );
}
