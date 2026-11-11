/**
 * What a trade's status means to this party, in plain English, and which
 * actions it allows. Every `TradeStatus` variant has its own case (with an
 * exhaustiveness check), so a variant the backend adds cannot render as
 * nothing.
 */
import type {
  ExchangeChain,
  NeedsAttentionReason,
  TradeRole,
  TradeRow,
  TradeStatus,
} from "../types/exchange";

/**
 * Unix seconds as "Nov 7, 2026, 14:05 UTC", or null for a value no `Date`
 * can hold. The times are the backend's u64s: a value past the year 275760
 * (or not a finite number at all) would make `toLocaleString` throw, which
 * would take down the page, so it is refused here instead.
 */
export function exchangeTimeText(unixSecs: number): string | null {
  if (typeof unixSecs !== "number" || !Number.isFinite(unixSecs) || unixSecs < 0) return null;
  const d = new Date(unixSecs * 1000);
  if (Number.isNaN(d.getTime())) return null;
  const text = d.toLocaleString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: "UTC",
  });
  return `${text} UTC`;
}

/**
 * A trade needs this party's Commit: `awaitingBuyerCommit` for the Buyer,
 * `awaitingSellerCommit` for the Seller (spec §9).
 */
export function needsMyCommit(trade: Pick<TradeRow, "status" | "role">): boolean {
  return (
    (trade.status.kind === "awaitingBuyerCommit" && trade.role === "buyer") ||
    (trade.status.kind === "awaitingSellerCommit" && trade.role === "seller")
  );
}

/**
 * The statuses before this party's own Commit: the Buyer commits first, so
 * only while waiting for it; the Seller commits second, so until its own.
 */
const BEFORE_MY_COMMIT: Record<TradeRole, ReadonlySet<TradeStatus["kind"]>> = {
  buyer: new Set(["awaitingBuyerCommit"]),
  seller: new Set(["awaitingBuyerCommit", "awaitingBtcLock", "awaitingSellerCommit"]),
};

/**
 * `exchange_cancel_trade` is offered while the terms are being agreed, or
 * before this party's Commit; a declined trade is already over.
 */
export function canCancel(
  trade: Pick<TradeRow, "status" | "role" | "negotiating" | "declined">,
): boolean {
  if (trade.declined !== null) return false;
  // A trade that already ended (an own cancel, an expired take) has nothing
  // left to cancel, even while its terms were never agreed.
  if (trade.status.kind === "expired") return false;
  return trade.negotiating || BEFORE_MY_COMMIT[trade.role].has(trade.status.kind);
}

/** The deadline of the Commit this trade waits for from this party, or null. */
export function myCommitDeadline(trade: Pick<TradeRow, "status" | "role">): number | null {
  if (!needsMyCommit(trade)) return null;
  const s = trade.status;
  return s.kind === "awaitingBuyerCommit" || s.kind === "awaitingSellerCommit" ? s.deadline : null;
}

export type StatusTone = "action" | "waiting" | "success" | "warning" | "ended";

export interface StatusText {
  headline: string;
  sentence: string;
  tone: StatusTone;
}

/** A time inside a sentence: the formatted time, or a phrase naming it when it cannot be shown. */
function at(unixSecs: number | null | undefined, fallback: string): string {
  const text = unixSecs == null ? null : exchangeTimeText(unixSecs);
  return text ?? fallback;
}

const COIN: Record<ExchangeChain, string> = { hns: "HNS", btc: "BTC" };
/** Whose lock a chain's coins sit in: the Seller locks HNS, the Buyer locks BTC. */
const OWNER: Record<ExchangeChain, TradeRole> = { hns: "seller", btc: "buyer" };
/** Where a refund lands. */
const HOME: Record<ExchangeChain, string> = { hns: "your wallet", btc: "your BTC account" };

/** The Commit, for the party about to make it: one confirmation signs every transaction of its side. */
export function commitSentence(role: TradeRole, deadline: number, refundAt: number | null): string {
  const by = at(deadline, "the deadline");
  return role === "buyer"
    ? `Confirm and sign before ${by}. That one confirmation signs every transaction of your side at once: paying your BTC into the lock, taking it back if the trade fails, and claiming the HNS. If the seller never locks the HNS, your BTC comes back to you automatically after ${at(refundAt, "the BTC refund time")}.`
    : `The buyer's BTC is locked and checked. Confirm and sign before ${by}. That one confirmation signs every transaction of your side at once: paying your HNS into the lock, taking it back if the trade fails, and claiming the BTC. If the buyer never claims the HNS, it comes back to you automatically after ${at(refundAt, "the HNS refund time")}.`;
}

/** Each `needsAttention` reason's own sentence. */
export function attentionText(
  reason: NeedsAttentionReason,
  chain: ExchangeChain,
  role: TradeRole,
  trade: Pick<TradeRow, "timeline">,
): { headline: string; sentence: string } {
  const btcBack = at(trade.timeline?.btcRefundTime, "the BTC refund time");
  const hnsBack = at(trade.timeline?.hnsRefundTime, "the HNS refund time");
  switch (reason) {
    case "sellerLockedWrongAmount":
      return {
        headline: "The seller locked the wrong amount",
        sentence:
          role === "buyer"
            ? `The seller's HNS lock does not match the agreed amount or script, so Namehold will not claim it and never reveals the secret. Your BTC comes back to you automatically after ${btcBack}.`
            : `Your HNS lock does not match the agreed amount or script, so the buyer cannot claim it. Your HNS comes back to you automatically after ${hnsBack}.`,
      };
    case "buyerLockedWrongAmount":
      return {
        headline: "The buyer locked the wrong amount",
        sentence:
          role === "seller"
            ? "The buyer's BTC lock does not match the agreed amount or script, so you will not be asked to lock your HNS. Nothing of yours was locked."
            : `Your BTC lock does not match the agreed amount or script, so the seller will not lock the HNS. Your BTC comes back to you automatically after ${btcBack}.`,
      };
    case "btcRefundedAfterReveal":
      return {
        headline: "The BTC went back after the secret was public",
        sentence:
          role === "seller"
            ? "The buyer took the BTC back after the secret was revealed, before your claim of the BTC was mined. The HNS went to the buyer."
            : "Your BTC came back to you after the secret was revealed, before the seller's claim of the BTC was mined.",
      };
    case "hnsRefundedAfterReveal":
      return {
        headline: "The HNS went back after the secret was public",
        sentence:
          role === "buyer"
            ? "The seller took the HNS back after your claim revealed the secret, before that claim was mined."
            : "Your HNS came back to you after the buyer's claim revealed the secret, before that claim was mined.",
      };
    case "missingPresigned":
      return {
        headline: "A signed transaction is missing",
        sentence: `A ${chain === "hns" ? "Handshake" : "Bitcoin"} transaction your side signed for this trade is not on this computer, so Namehold cannot send it. A watchtower that registered the trade still holds it.`,
      };
    default: {
      // A reason this build does not know: say so rather than throw while rendering.
      const unknown: never = reason;
      return {
        headline: "This trade needs attention",
        sentence: `Namehold reported a problem this version cannot describe (${String(unknown)}).`,
      };
    }
  }
}

/** The headline, the sentence and the tone of a trade's status, for this party. */
export function tradeStatusText(trade: Pick<TradeRow, "status" | "role" | "timeline">): StatusText {
  const { status: s, role } = trade;
  const btcBack = at(trade.timeline?.btcRefundTime, "the BTC refund time");
  const hnsBack = at(trade.timeline?.hnsRefundTime, "the HNS refund time");
  switch (s.kind) {
    case "awaitingBuyerCommit":
      return role === "buyer"
        ? {
            headline: "Confirm to start the trade",
            sentence: commitSentence("buyer", s.deadline, trade.timeline?.btcRefundTime ?? null),
            tone: "action",
          }
        : {
            headline: "Waiting for the buyer to confirm",
            sentence: `The buyer has until ${at(s.deadline, "the deadline")} to confirm and lock their BTC. You have not signed anything yet.`,
            tone: "waiting",
          };
    case "awaitingBtcLock":
      return role === "buyer"
        ? {
            headline: "Locking your BTC",
            sentence: `Your BTC lock has ${s.confirmations} of ${s.needed} confirmations. If the seller never locks the HNS, your BTC comes back to you automatically after ${btcBack}.`,
            tone: "waiting",
          }
        : {
            headline: "Waiting for the buyer's BTC",
            sentence: `The buyer's BTC lock has ${s.confirmations} of ${s.needed} confirmations. You are asked to confirm once it has all of them.`,
            tone: "waiting",
          };
    case "awaitingSellerCommit":
      return role === "seller"
        ? {
            headline: "Confirm to lock your HNS",
            sentence: commitSentence("seller", s.deadline, trade.timeline?.hnsRefundTime ?? null),
            tone: "action",
          }
        : {
            headline: "Waiting for the seller to confirm",
            sentence: `The seller has until ${at(s.deadline, "the deadline")} to lock the HNS. If they never do, your BTC comes back to you automatically after ${btcBack}.`,
            tone: "waiting",
          };
    case "awaitingHnsLock":
      return role === "seller"
        ? {
            headline: "Locking your HNS",
            sentence: `Your HNS lock has ${s.confirmations} of ${s.needed} confirmations. If the buyer never claims it, your HNS comes back to you automatically after ${hnsBack}.`,
            tone: "waiting",
          }
        : {
            headline: "Waiting for the seller's HNS",
            sentence: `The seller's HNS lock has ${s.confirmations} of ${s.needed} confirmations. Namehold claims the HNS for you once it has all of them.`,
            tone: "waiting",
          };
    case "revealing":
      return role === "buyer"
        ? {
            headline: "Claiming your HNS",
            sentence:
              "Namehold is sending your claim of the HNS. It reveals the secret, which lets the seller take the BTC.",
            tone: "waiting",
          }
        : {
            headline: "The buyer is claiming the HNS",
            sentence:
              "The buyer's claim of the HNS is on its way. Namehold then uses the secret it reveals to claim your BTC.",
            tone: "waiting",
          };
    case "awaitingReveal":
      return role === "buyer"
        ? {
            headline: "Ready to claim your HNS",
            sentence: `Namehold claims the HNS for you as soon as your node confirms the seller's lock. If it cannot before ${at(s.refundOpensAt, "the HNS refund time")}, both sides get their own coins back.`,
            tone: "waiting",
          }
        : {
            headline: "Waiting for the buyer to claim",
            sentence: `The buyer can claim the HNS until ${at(s.refundOpensAt, "the HNS refund time")}. If they never claim it, your HNS comes back to you automatically after that time.`,
            tone: "waiting",
          };
    case "claiming":
      return role === "seller"
        ? {
            headline: "Claiming your BTC",
            sentence:
              "The secret is public. Namehold is sending your claim of the BTC and raises its fee until it is mined.",
            tone: "waiting",
          }
        : {
            headline: "The seller is claiming the BTC",
            sentence:
              "Your claim of the HNS revealed the secret. The seller is now claiming the BTC.",
            tone: "waiting",
          };
    case "completed":
      return {
        headline: "Trade complete",
        sentence:
          role === "buyer"
            ? "The HNS is yours, and the BTC goes to the seller."
            : "The BTC is yours, and the buyer has the HNS.",
        tone: "success",
      };
    case "awaitingRefund":
      return OWNER[s.chain] === role
        ? {
            headline: `Waiting to take your ${COIN[s.chain]} back`,
            sentence: `The trade did not finish. Your ${COIN[s.chain]} comes back to you automatically after ${at(s.opensAt, "its refund time")}.`,
            tone: "waiting",
          }
        : {
            headline: `The ${OWNER[s.chain]} is waiting to take their ${COIN[s.chain]} back`,
            sentence: `The trade did not finish. The ${OWNER[s.chain]}'s ${COIN[s.chain]} goes back to them after ${at(s.opensAt, "its refund time")}.`,
            tone: "waiting",
          };
    case "refunding":
      return OWNER[s.chain] === role
        ? {
            headline: `Taking your ${COIN[s.chain]} back`,
            sentence: `The trade did not finish. Namehold is sending the refund of your ${COIN[s.chain]} to ${HOME[s.chain]}.`,
            tone: "waiting",
          }
        : {
            headline: `The ${OWNER[s.chain]} is taking their ${COIN[s.chain]} back`,
            sentence: `The trade did not finish, and the ${OWNER[s.chain]}'s refund of the ${COIN[s.chain]} is on its way.`,
            tone: "waiting",
          };
    case "refunded":
      return OWNER[s.chain] === role
        ? {
            headline: `Your ${COIN[s.chain]} is back`,
            sentence: `The trade did not finish, and your ${COIN[s.chain]} is back in ${HOME[s.chain]}.`,
            tone: "ended",
          }
        : {
            headline: `The ${OWNER[s.chain]} took their ${COIN[s.chain]} back`,
            sentence: `The trade did not finish, and the ${OWNER[s.chain]}'s ${COIN[s.chain]} went back to them.`,
            tone: "ended",
          };
    case "expired":
      return {
        headline: "Trade expired",
        sentence: "The trade ended before either side locked any coins.",
        tone: "ended",
      };
    case "cannotCheck":
      return s.chain === "hns"
        ? {
            headline: "Can't check the HNS chain",
            sentence:
              "Namehold cannot read the Handshake chain through this wallet's node right now. The trade goes on once it can; its refund times do not change.",
            tone: "warning",
          }
        : {
            headline: "Can't check Bitcoin",
            sentence:
              "Namehold cannot get an answer it can check from its Bitcoin sources right now. The trade goes on once it can; its refund times do not change.",
            tone: "warning",
          };
    case "needsAttention":
      return { ...attentionText(s.reason, s.chain, role, trade), tone: "warning" };
    default: {
      // A status this build does not know: say so rather than throw while rendering.
      const unknown: never = s;
      return {
        headline: "Unknown status",
        sentence: `This version of Namehold cannot describe the trade's status (${String((unknown as { kind?: unknown }).kind)}).`,
        tone: "warning",
      };
    }
  }
}
