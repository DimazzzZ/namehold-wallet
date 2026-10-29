import { useRef } from "react";
import { stageOf } from "../lib/errors";
import type { BroadcastResult } from "../types";
import { useDeleteTxDraft, useExecuteDraft } from "./wallet";

/**
 * The one owner of a built draft's fate after it is built.
 *
 * A draft reserves the coins it spends from the moment it is built. Every flow
 * that builds one (Send, a batch, a name action, a purchase, a lock, a Speed
 * up, a listing's finalize) must therefore end it one of three ways:
 *
 * - **sent**: `run` / `send` signed and broadcast it;
 * - **failed before broadcast** (a declined secure window, a locked signer, a
 *   sign error): `settleFailed` discards it, unless the flow keeps it for a
 *   retry and abandons it later;
 * - **abandoned** (Back, Cancel, Escape, leaving the page): `abandon`
 *   discards it.
 *
 * A draft whose broadcast was tried is never discarded here: hsd may have taken
 * the transaction (it answers `sendrawtransaction` before the mempool decides),
 * and Activity settles it against the node. That is the rule every flow used
 * to copy by hand, and two of them (Send, batch Cancel) never did, which left
 * their coins reserved and the next send "not enough funds".
 *
 * Discarding is best effort: a failure leaves the row in Activity, which
 * offers Discard.
 */
export function useDraftRunner() {
  const exec = useExecuteDraft();
  const deleteDraft = useDeleteTxDraft();
  // Drafts whose broadcast was tried through this runner (sent or not).
  const broadcastTried = useRef(new Set<string>());

  const discard = async (draftId: string): Promise<boolean> => {
    try {
      await deleteDraft.mutateAsync(draftId);
      return true;
    } catch {
      return false;
    }
  };

  /** Unlock if needed, sign, broadcast; rejects with the `StagedError` of `useExecuteDraft`. */
  const run = async (
    draftId: string,
    profileId: string,
    unlocked: boolean,
  ): Promise<BroadcastResult> => {
    try {
      const result = await exec.run(draftId, profileId, unlocked);
      broadcastTried.current.add(draftId);
      return result;
    } catch (e) {
      if (stageOf(e) === "broadcast") broadcastTried.current.add(draftId);
      throw e;
    }
  };

  /**
   * After a failed send: discard the draft when the failure came before the
   * broadcast; keep it when the broadcast was tried. Resolves to whether it
   * discarded.
   */
  const settleFailed = async (draftId: string, error: unknown): Promise<boolean> => {
    if (stageOf(error) === "broadcast" || broadcastTried.current.has(draftId)) return false;
    return discard(draftId);
  };

  /** The user left a built, unsent draft: discard it. A no-op after a tried broadcast. */
  const abandon = async (draftId: string | null | undefined): Promise<boolean> => {
    if (!draftId || broadcastTried.current.has(draftId)) return false;
    return discard(draftId);
  };

  /**
   * `run`, then `settleFailed` on a failure (or keep the draft for a retry
   * with `keepOnFailure`, and `abandon` it when the user leaves). Rethrows.
   */
  const send = async (
    draftId: string,
    profileId: string,
    unlocked: boolean,
    opts: { keepOnFailure?: boolean } = {},
  ): Promise<BroadcastResult> => {
    try {
      return await run(draftId, profileId, unlocked);
    } catch (e) {
      if (!opts.keepOnFailure) await settleFailed(draftId, e);
      throw e;
    }
  };

  return {
    run,
    send,
    settleFailed,
    abandon,
    pending: exec.pending,
    unlock: exec.unlock,
    sign: exec.sign,
    broadcast: exec.broadcast,
  };
}

export type DraftRunner = ReturnType<typeof useDraftRunner>;
