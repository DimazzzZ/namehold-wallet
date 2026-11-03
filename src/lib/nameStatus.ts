/**
 * The name page's status line and lifecycle timeline, in plain words (pure).
 * Times are approximate and come from block counts at ten minutes a block,
 * only where blocks keep time (mainnet, testnet); regtest reads blocks.
 */
import type { HsdName, NameActionCapabilities, WalletNetwork } from "../types";
import {
  auctionPhase,
  formatCountdown,
  SECS_PER_BLOCK,
  type AuctionTaskSummary,
  type PhaseCountdown,
} from "./auction";
import { formatHnsShort } from "./utils";

export type StatusTone = "ok" | "warn" | "bad" | "info" | "muted";

export const STATUS_DOT: Record<StatusTone, string> = {
  ok: "bg-green-500",
  warn: "bg-amber-500",
  bad: "bg-red-500",
  info: "bg-blue-500",
  muted: "bg-gray-400",
};

/** Blocks keep wall-clock time on these networks. */
export function wallClock(network: WalletNetwork | string | null | undefined): boolean {
  return network === "mainnet" || network === "testnet";
}

/** A countdown as this network can tell it: blocks only where they don't keep time. */
export function countdownForNetwork(
  c: PhaseCountdown,
  network: WalletNetwork | string | null | undefined,
): PhaseCountdown {
  return wallClock(network) ? c : { ...c, hours: null };
}

/** "Oct 30" / "~2 days", or "Sep 2028" / "~23 months"; null where blocks don't keep time. */
export function approxBlocksDate(
  blocks: number,
  network: WalletNetwork | string | null | undefined,
  nowMs: number = Date.now(),
): { short: string; rel: string } | null {
  if (!wallClock(network)) return null;
  const secs = blocks * SECS_PER_BLOCK;
  const d = new Date(nowMs + secs * 1000);
  const days = secs / 86_400;
  if (days < 60) {
    const hours = secs / 3600;
    const rel =
      hours < 36
        ? `~${Math.max(1, Math.round(hours))}h`
        : `~${Math.round(days)} ${Math.round(days) === 1 ? "day" : "days"}`;
    return { short: d.toLocaleDateString("en-US", { month: "short", day: "numeric" }), rel };
  }
  const rel =
    days < 730
      ? `~${Math.round(days / 30.44)} months`
      : `~${(days / 365.25).toFixed(1).replace(/\.0$/, "")} years`;
  return { short: d.toLocaleDateString("en-US", { month: "short", year: "numeric" }), rel };
}

const blocksText = (n: number) => `${n.toLocaleString("en-US")} ${n === 1 ? "block" : "blocks"}`;

function lowerFirst(s: string): string {
  return s ? s[0]!.toLowerCase() + s.slice(1) : s;
}

function title(action: string): string {
  return action.charAt(0).toUpperCase() + action.slice(1).toLowerCase();
}

export interface StatusInput {
  caps: NameActionCapabilities | null | undefined;
  summary?: AuctionTaskSummary | null;
  info: HsdName | null | undefined;
  countdown: PhaseCountdown | null;
  network: WalletNetwork | string | null | undefined;
  loading?: boolean;
  capsPending?: boolean;
  error?: boolean;
  nowMs?: number;
}

export interface StatusLine {
  tone: StatusTone;
  status: string;
  sub: string | null;
  note: string | null;
}

export function nameStatusLine(input: StatusInput): StatusLine {
  const { caps, info, countdown, network } = input;
  const line = (
    tone: StatusTone,
    status: string,
    sub: string | null = null,
    note: string | null = null,
  ) => ({
    tone,
    status,
    sub,
    note,
  });
  const cd = countdown
    ? `${lowerFirst(countdown.label)} ${formatCountdown(countdownForNetwork(countdown, network))}`
    : null;

  if (caps?.pendingBroadcastAction) {
    return line(
      "warn",
      `${title(caps.pendingBroadcastAction)} sent`,
      "waiting for a block — nothing changes on-chain until it lands",
    );
  }
  if (input.loading || input.capsPending) return line("muted", "Checking…");
  if (input.error) return line("bad", "Couldn't load this name");
  if (!caps) return line("muted", auctionPhase(info?.state).label, cd);

  const blocksLeft = info?.stats?.blocksUntilExpire ?? null;
  const exp = blocksLeft != null ? approxBlocksDate(blocksLeft, network, input.nowMs) : null;
  const note = exp && blocksLeft != null ? `(${blocksText(blocksLeft)})` : null;
  const redeem =
    caps.redeemableValueDoos > 0
      ? `reclaim ${formatHnsShort(caps.redeemableValueDoos)} HNS`
      : "reclaim your lockup";
  const { phase } = auctionPhase(caps.phase ?? info?.state);

  switch (caps.taskState) {
    case "availableToOpen":
      return line("warn", "Available", "open the auction to start bidding");
    case "waitingForBidding":
      return line("muted", "Opening", cd ?? "bidding opens soon");
    case "readyToBid":
      return line(
        "warn",
        caps.hasBidCommitment ? "Bidding — your bid is in" : "Bidding is open",
        cd,
      );
    case "readyToReveal":
      return line(
        "warn",
        "Reveal your bid",
        cd ? `${cd}, or the lockup is lost` : "before the window closes",
      );
    case "revealBroadcastPending":
      return line("info", "Reveal sent", "waiting for a block");
    case "revealDoneWaitingForClose":
      return line("info", "Revealed", cd ?? "waiting for the auction to close");
    case "wonNeedsRegister":
      return line("ok", "You won", "register it to make it yours");
    case "lostNeedsRedeem":
      return line(
        "muted",
        phase === "REVOKED"
          ? "Revoked by the winner"
          : caps.ownsName
            ? "Owned · a losing bid of yours is still locked"
            : "You lost",
        redeem,
      );
    case "lostUnrevealed":
      return line("bad", "Lost — not revealed in time", "the lockup can't be recovered");
    case "closedResultUnknown":
      return line("muted", "Auction closed", "sync your node to see the result");
    case "transferPendingFinalize":
      return caps.canFinalize.allowed
        ? line("info", "Transferring", "ready to finalize")
        : line("info", "Transferring", cd ?? "finalize once the transfer lockup ends");
    case "ownedNoUrgentAction":
      return line(
        "ok",
        "Owned",
        exp
          ? `renew by ${exp.short}`
          : blocksLeft != null
            ? `renew within ${blocksText(blocksLeft)}`
            : "registered to this wallet",
        note,
      );
    case "expiringSoon":
      return line(
        "bad",
        "Expiring",
        exp
          ? `renew before ${exp.short} (${exp.rel}) or the name is lost`
          : blocksLeft != null
            ? `renew within ${blocksText(blocksLeft)} or the name is lost`
            : "renew now",
        note,
      );
    default: {
      if (phase === "REVOKED")
        return line("muted", "Revoked", "it returns to auction when it expires");
      const third = phase === "CLOSED" && !caps.ownsName;
      if (third) {
        return line(
          "muted",
          "Registered by someone else",
          exp
            ? `expires ${exp.short}`
            : blocksLeft != null
              ? `expires in ${blocksText(blocksLeft)}`
              : null,
          note,
        );
      }
      return line("muted", "Not available to you");
    }
  }
}

export type TimelineKind = "done" | "current" | "next" | "lost" | "na";
export interface TimelineStep {
  label: string;
  sub: string;
  kind: TimelineKind;
}

export interface TimelineInput {
  caps: NameActionCapabilities | null | undefined;
  info: HsdName | null | undefined;
  /** "3 bids", or "" when there are none. */
  bidCount: string;
  countdown: PhaseCountdown | null;
  network: WalletNetwork | string | null | undefined;
  nowMs?: number;
}

/** "~5 days" where blocks keep time, "{N} blocks" otherwise; "" when unknown. */
function duration(blocks: number | null | undefined, network: TimelineInput["network"]): string {
  if (blocks == null) return "";
  const d = approxBlocksDate(blocks, network, 0);
  return d ? d.rel : blocksText(blocks);
}

export function nameTimeline(input: TimelineInput): TimelineStep[] {
  const { caps, info, network } = input;
  const s = (label: string, kind: TimelineKind, sub = ""): TimelineStep => ({ label, sub, kind });
  const done = (label: string, sub = "") => s(label, "done", sub);
  const na = (label: string) => s(label, "na", "—");
  const phase = auctionPhase(caps?.phase ?? info?.state).phase;
  const opened = info?.height != null ? `block ${info.height.toLocaleString("en-US")}` : "";
  const cd = input.countdown ? formatCountdown(countdownForNetwork(input.countdown, network)) : "";
  const blocksLeft = info?.stats?.blocksUntilExpire ?? null;
  const expD = blocksLeft != null ? approxBlocksDate(blocksLeft, network, input.nowMs) : null;
  const exp =
    blocksLeft == null
      ? ""
      : expD
        ? `${expD.short} · ${blocksText(blocksLeft)}`
        : `in ${blocksText(blocksLeft)}`;
  const bidding = duration(caps?.auctionBiddingBlocks, network);
  const reveal = duration(caps?.auctionRevealBlocks, network);
  const task = caps?.taskState;
  const renewed =
    info?.renewal != null ? `renewed block ${info.renewal.toLocaleString("en-US")}` : "";

  if (task === "availableToOpen" || phase === "AVAILABLE") {
    return [
      s("Open", "current", "start the auction"),
      s("Bidding", "next", bidding),
      s("Reveal", "next", reveal),
      s("Result", "next"),
      s("Registered", "next"),
      s("Renew", "next"),
    ];
  }
  if (task === "waitingForBidding" || phase === "OPENING") {
    return [
      s("Opened", "current", cd),
      s("Bidding", "next", bidding),
      s("Reveal", "next", reveal),
      s("Result", "next"),
      s("Registered", "next"),
      s("Renew", "next"),
    ];
  }
  if (phase === "BIDDING") {
    return [
      done("Opened", opened),
      s(
        "Bidding",
        "current",
        caps?.hasBidCommitment
          ? "your bid is in"
          : input.bidCount
            ? `${input.bidCount} so far`
            : "",
      ),
      s("Reveal", "next", cd ? `in ${cd}` : ""),
      s("Result", "next"),
      s("Registered", "next"),
      s("Renew", "next"),
    ];
  }
  if (phase === "REVEAL") {
    return [
      done("Opened", opened),
      done("Bidding", input.bidCount),
      s(
        "Reveal",
        "current",
        task === "revealBroadcastPending"
          ? "reveal sent"
          : task === "revealDoneWaitingForClose"
            ? "revealed"
            : "reveal your bid",
      ),
      s("Result", "next", cd ? `in ${cd}` : ""),
      s("Registered", "next"),
      s("Renew", "next"),
    ];
  }
  if (task === "wonNeedsRegister") {
    return [
      done("Opened", opened),
      done("Bidding", input.bidCount),
      done("Reveal", "revealed"),
      s(
        "Won",
        "current",
        info?.highest != null ? `${formatHnsShort(info.highest)} HNS high bid` : "",
      ),
      s("Registered", "next", "register now"),
      s("Renew", "next", exp),
    ];
  }
  if (task === "lostNeedsRedeem" || task === "lostUnrevealed") {
    const unrevealed = task === "lostUnrevealed";
    return [
      done("Opened", opened),
      done("Bidding", input.bidCount),
      done("Reveal", unrevealed ? "not revealed" : "revealed"),
      s(
        "Lost",
        "lost",
        unrevealed
          ? "lockup forfeited"
          : caps && caps.redeemableValueDoos > 0
            ? `reclaim ${formatHnsShort(caps.redeemableValueDoos)} HNS`
            : "reclaim your lockup",
      ),
      na("Registered"),
      na("Renew"),
    ];
  }
  if (task === "closedResultUnknown") {
    return [
      done("Opened", opened),
      done("Bidding", input.bidCount),
      done("Reveal"),
      s("Closed", "current", "sync to see the result"),
      s("Registered", "next"),
      s("Renew", "next"),
    ];
  }
  if (phase === "REVOKED") {
    return [
      done("Opened", opened),
      done("Bidding"),
      done("Reveal"),
      done("Won", "by someone else"),
      s("Revoked", "lost", "burned by its owner"),
      na("Renew"),
    ];
  }
  if (!caps?.ownsName) {
    return [
      done("Opened", opened),
      done("Bidding"),
      done("Reveal"),
      done("Won", "by someone else"),
      s("Registered", "current", "not yours"),
      s("Expires", "next", exp),
    ];
  }
  if (caps.transferPending) {
    return [
      done("Opened", opened),
      done("Bidding"),
      done("Won"),
      done("Registered", renewed),
      s("Transfer", "current", "lockup running"),
      s("Finalize", "next", "then it leaves this wallet"),
    ];
  }
  return [
    done("Opened", opened),
    done("Bidding"),
    done("Reveal"),
    done("Won"),
    s("Registered", "current", renewed || "yours"),
    s(task === "expiringSoon" ? "Renew due" : "Expires", "next", exp),
  ];
}
