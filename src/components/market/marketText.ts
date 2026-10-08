import { settingToBool } from "../../lib/settingsBool";
import { formatDate } from "../../lib/utils";
import type {
  HiddenCounts,
  MarketRow,
  ShakedexHidden,
  WalletNetwork,
  WalletProfileKind,
  WriteCapability,
} from "../../types";

/** Why Buy and Finalize are disabled for any profile but a recovery-phrase one (R16). */
export const RECOVERY_PHRASE_ONLY = "Shakedex works with a recovery-phrase wallet for now";

/** Why Buy and Finalize are disabled when the node cannot send (R6). */
export const NEEDS_SENDING_NODE =
  "Shakedex needs a local node, or a remote node with sending allowed";

/**
 * Whether a profile can buy or finalize through Shakedex: a seed-backed
 * recovery-phrase wallet on a node that can send, as the backend enforces
 * (`software_writer_ctx`). Unknown (still loading) is not.
 */
export function canUseShakedex(
  kind: WalletProfileKind | undefined,
  writeCap: WriteCapability | null | undefined,
): boolean {
  return kind === "mnemonic_hot" && writeCap?.broadcasterAvailable === true;
}

/**
 * The backend's own sentence for why Shakedex is refused here, once the
 * profile or the write capability is known to refuse it; `null` otherwise.
 */
export function shakedexRefusal(
  kind: WalletProfileKind | undefined,
  writeCap: WriteCapability | null | undefined,
): string | null {
  if (kind !== undefined && kind !== "mnemonic_hot") return RECOVERY_PHRASE_ONLY;
  if (writeCap != null && !writeCap.broadcasterAvailable) return NEEDS_SENDING_NODE;
  return null;
}

/** Why a market link cannot be imported off mainnet (R5). */
export const MARKET_MAINNET_ONLY = "LearnHNS Market lists mainnet names only";

/** Why Buy is disabled on mainnet until Settings allows it (R15). Finalize is not gated. */
export const MAINNET_EXPERIMENTAL =
  "Shakedex purchases on mainnet are experimental: enable them in Settings";

/**
 * The backend's sentence for why a new purchase is refused here, in the order
 * its gates run (`commands/shakedex.rs::prepare`): profile, node, then the
 * mainnet flag, which only `"true"` enables, as in the backend. `null` when
 * nothing known refuses it.
 */
export function purchaseRefusal(
  kind: WalletProfileKind | undefined,
  writeCap: WriteCapability | null | undefined,
  network: WalletNetwork | undefined,
  experimental: string | undefined,
): string | null {
  const refusal = shakedexRefusal(kind, writeCap);
  if (refusal) return refusal;
  if (network === "mainnet" && !settingToBool(experimental)) return MAINNET_EXPERIMENTAL;
  return null;
}

/** Whether a new purchase can be made: `canUseShakedex` plus the mainnet flag. */
export function canBuyShakedex(
  kind: WalletProfileKind | undefined,
  writeCap: WriteCapability | null | undefined,
  network: WalletNetwork | undefined,
  experimental: string | undefined,
): boolean {
  return (
    canUseShakedex(kind, writeCap) &&
    network !== undefined &&
    purchaseRefusal(kind, writeCap, network, experimental) === null
  );
}

/** "~6 h" — the wait is measured against the node's median time, so it is only approximate. */
export function approxWait(secs: number): string {
  if (secs < 3600) return `~${Math.max(1, Math.round(secs / 60))} min`;
  if (secs < 48 * 3600) return `~${Math.round(secs / 3600)} h`;
  return `~${Math.round(secs / 86400)} d`;
}

/** The "Hidden N: X already sold or cancelled, ..." line; zero parts are omitted. */
export function hiddenSummary(h: HiddenCounts): string | null {
  const parts: [number, string][] = [
    [h.soldOrCancelled, "already sold or cancelled"],
    [h.failedVerification, "failed verification"],
    [h.expiresBeforeFinalize, "expire before they can be finalized"],
    [h.notYetValid, "not valid yet"],
    [h.couldNotCheck, "could not be checked"],
  ];
  const total = parts.reduce((n, [c]) => n + c, 0);
  if (total === 0) return null;
  const text = parts
    .filter(([c]) => c > 0)
    .map(([c, label]) => `${c} ${label}`)
    .join(", ");
  return `Hidden ${total}: ${text}`;
}

/** A row the profile's chain source (SPV, Explorer) cannot check (R8). */
export const NOT_VERIFIED = "Not verified — needs a full or remote node";

/** A human sentence for why a listing is not buyable. */
export function hiddenReasonText(r: ShakedexHidden): string {
  switch (r.kind) {
    case "soldOrCancelled":
      return "Already sold or cancelled.";
    case "failedVerification":
      return `Failed verification: ${r.reason}`;
    case "expiresBeforeFinalize":
      return "The name expires before a purchase could be finalized.";
    case "notYetValid":
      return `Not valid yet; the first price step becomes valid in ${approxWait(r.firstValidInSecs)}.`;
    case "couldNotCheck":
      return `Could not be checked: ${nodeReason(r.reason)}`;
    case "unverified":
      return NOT_VERIFIED;
  }
}

/**
 * "Listed until July 10, 2027 · stays buyable until sold or cancelled": the
 * seller's own `expiresAt` (unix seconds). Shown, never enforced: the signed
 * price steps stay valid on chain until the lock coin is spent, whatever the
 * date says (R2), and the spec has the wallet say so wherever the date is
 * shown (§4). `null` when the value is beyond what a date can hold — it comes
 * from a foreign file as any u64.
 */
export function listedUntilText(expiresAt: number): string | null {
  const d = new Date(expiresAt * 1000);
  if (Number.isNaN(d.getTime())) return null;
  return `Listed until ${formatDate(d.toISOString().slice(0, 10))} · stays buyable until sold or cancelled`;
}

/**
 * A node failure in the user's words. The backend's text names the layer
 * ("Node RPC error: …"); the user needs what happened and where to fix it.
 */
export function nodeReason(reason: string): string {
  const bare = reason.replace(/^Node RPC error:\s*/i, "").replace(/^HTTP error:\s*/i, "");
  if (/error sending request|connection refused|tcp connect|dns error/i.test(bare)) {
    return "your node did not answer. Is it running? Check Settings → Connections.";
  }
  if (/timed out|timeout/i.test(bare)) {
    return "your node took too long to answer. Try again in a moment.";
  }
  return bare;
}

/** How a listing's status reads on the Market: a tone and one sentence. */
export type StatusTone = "ok" | "warn" | "muted" | "bad";

export function listingStatus(row: MarketRow): { tone: StatusTone; text: string } {
  const v = row.verdict;
  if (v.verdict === "buyable") {
    return v.warnExpiry
      ? {
          tone: "warn",
          text: "Ready to buy. The name expires soon after it can be finalized: finalize in time.",
        }
      : { tone: "ok", text: "Ready to buy" };
  }
  switch (v.kind) {
    case "soldOrCancelled":
    case "unverified":
    case "notYetValid":
      return { tone: "muted", text: hiddenReasonText(v) };
    case "couldNotCheck":
    case "expiresBeforeFinalize":
      return { tone: "warn", text: hiddenReasonText(v) };
    case "failedVerification":
      return { tone: "bad", text: hiddenReasonText(v) };
  }
}
