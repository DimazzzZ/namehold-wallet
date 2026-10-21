/**
 * Speed up (child pays for parent, spec 2026-10-21): whether a sent,
 * unconfirmed transaction can be sped up, the child it would get, and
 * building that child's draft. The child is then signed and sent like any
 * draft (`useExecuteDraft`).
 */
import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type { SpeedUpCheck, SpeedUpSummary, TxDraftSummary } from "../types";

/** The draft action of a Speed up child (`speed_up.rs::SPEED_UP_ACTION`). */
export const SPEED_UP_ACTION = "speed_up";

/** Whether `draftId` can be sped up now, and why not (R1-R6). */
export function useSpeedUpCheck(draftId: string | null, enabled = true) {
  return useQuery({
    queryKey: ["wallet", "speedUpCheck", draftId],
    queryFn: () => invoke<SpeedUpCheck>("speed_up_check", { draftId }),
    enabled: enabled && draftId != null,
    // The node's mempool moves on its own; the drafts list polls too.
    refetchInterval: 30_000,
    retry: false,
  });
}

/** How long a typed rate must rest before the node is asked for its preview. */
export const PREVIEW_DEBOUNCE_MS = 400;

/**
 * The child Speed up would build at `feeRate` doos/vB (the node's estimate
 * when null). A typed rate is asked for once it has rested
 * `PREVIEW_DEBOUNCE_MS`, not per keystroke; `settling` is true until then,
 * and the preview shown is for `settledRate`.
 */
export function useSpeedUpPreview(draftId: string, feeRate: number | null, enabled: boolean) {
  const [settledRate, setSettledRate] = useState(feeRate);
  useEffect(() => {
    const t = setTimeout(() => setSettledRate(feeRate), PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [feeRate]);
  const query = useQuery({
    queryKey: ["wallet", "speedUpPreview", draftId, settledRate],
    queryFn: () => invoke<SpeedUpSummary>("preview_speed_up", { draftId, feeRate: settledRate }),
    enabled,
    retry: false,
  });
  return { ...query, settledRate, settling: settledRate !== feeRate };
}

/** Build the child's draft. */
export function useBuildSpeedUp() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ draftId, feeRate }: { draftId: string; feeRate: number | null }) =>
      invoke<TxDraftSummary>("build_speed_up_draft", { draftId, feeRate }),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: ["wallet", "drafts"] });
      void qc.invalidateQueries({ queryKey: ["wallet", "speedUpCheck"] });
    },
  });
}

/** A backend refusal as the user reads it: without the error kind's prefix. */
export function refusalText(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e);
  return raw.replace(/^Invalid input: /, "");
}
