/**
 * The Names page's rows: one per name, whether owned, bid on, bought, listed
 * or watched, each described the same way (pure, no React).
 */
import type {
  AuctionTaskState,
  HsdName,
  NameActionCapabilities,
  WatchedName,
  WatchlistStatus,
} from "../types";
import type { StatusTone } from "../components/market/marketText";
import { listingStateLabel, MY_LISTINGS_TAB } from "../components/market/marketText";
import {
  auctionPhase,
  formatCountdown,
  formatSpanDays,
  nextTransition,
  pendingBroadcastBadge,
  taskStateUrgencyRank,
} from "./auction";
import { formatHnsShort } from "./utils";

export type NameCat = "needs" | "auction" | "owned" | "watching";
export type RowKind = "owned" | "listing" | "purchase" | "auction" | "watch";
export type RowTone = StatusTone | "info";

export interface NameRow {
  name: string;
  kind: RowKind;
  cats: NameCat[];
  status: string;
  detail: string | null;
  tone: RowTone;
  pending: string | null;
  statusTitle: string;
  value: string | null;
  expires: { text: string; className: string } | null;
  action: {
    label: string;
    kind: "open" | "myListings" | "finalizePurchase" | "viewPurchase";
    signs: boolean;
  } | null;
  selectable: boolean;
  height: number | null;
  renewal: number | null;
  watch: { tags: string; addedAt: string } | null;
  /** finalizePurchase rows only. */
  purchaseId?: string;
}

/**
 * Live task states that count as "still in the auction" for a position name
 * (one with a draft/commitment but no owner coin). A position is only ever
 * DISPLAYED when its current capabilities land in this set — this is what
 * makes the list self-cleaning: a won+registered name flips to `ownsName`
 * (excluded upstream by the backend before it even becomes a position) and a
 * lost+redeemed one settles into `unavailableOther`/no-caps, so both quietly
 * drop off without any extra bookkeeping here.
 */
export const ACTIVE_POSITION_TASK_STATES = new Set<AuctionTaskState>([
  "availableToOpen",
  "waitingForBidding",
  "readyToBid",
  "readyToReveal",
  // Keep the row visible while the reveal is in flight and after it confirms
  // (until the auction closes), so a successful reveal never makes the row
  // silently vanish — the exact "where did it go?" confusion in another form.
  "revealBroadcastPending",
  "revealDoneWaitingForClose",
  "wonNeedsRegister",
  "lostNeedsRedeem",
  // A bid never revealed: nothing to do, but the lockup is gone and the row
  // is where the user looks for the auction's outcome.
  "lostUnrevealed",
  // A closed auction with a bid of ours the wallet cannot settle yet (no
  // synced node): its lockup is ours and its outcome unknown, so the row
  // stays until a sync says won, lost or redeemable.
  "closedResultUnknown",
]);

export interface NameRowsInput {
  owned: HsdName[];
  positions: string[];
  capsByName: Map<string, NameActionCapabilities>;
  watched: WatchedName[];
  watchInfo: Record<string, HsdName>;
  watchStatus: WatchlistStatus[];
  /** Needs-you names, in their order (from useNeedsYou). */
  needs: string[];
  finalizeSent: Set<string | undefined>;
  /** mainnet / testnet: blocks keep wall-clock time, so spans read in days. */
  wallClock: boolean;
  isWatchOnly?: boolean;
}

/** Blocks keep wall-clock time on these networks (hsd's "main" is not a wallet value). */
export function wallClock(network: string | null | undefined): boolean {
  return network === "mainnet" || network === "testnet";
}

const short = (doos: number | null | undefined) => formatHnsShort(doos ?? 0);

function countdown(c: NameActionCapabilities | undefined): string | null {
  if (!c || c.countdownBlocks == null) return null;
  return formatCountdown({
    label: c.countdownLabel ?? "",
    blocks: c.countdownBlocks,
    hours: c.countdownHours,
  });
}

/** "45d" / "8mo" on a wall-clock network, "1,234 blocks" otherwise; null when unknown. */
function span(n: HsdName | undefined, clock: boolean): string | null {
  const s = n?.stats;
  if (!s) return null;
  if (clock && s.daysUntilExpire != null) return formatSpanDays(s.daysUntilExpire);
  if (s.blocksUntilExpire != null) return `${s.blocksUntilExpire.toLocaleString("en-US")} blocks`;
  return null;
}

function expiresCell(n: HsdName | undefined, clock: boolean, watched: boolean): NameRow["expires"] {
  const sp = span(n, clock);
  if (!sp) return null;
  const days = n?.stats?.daysUntilExpire;
  const blockMode = !(clock && days != null);
  const className = blockMode
    ? "text-gray-500"
    : days! <= 30
      ? "text-red-600"
      : days! < 90
        ? "text-yellow-600"
        : watched
          ? "text-green-600"
          : "text-gray-500";
  return { text: `in ${sp}`, className };
}

type Described = Pick<NameRow, "status" | "detail" | "tone" | "value" | "action"> & {
  /** For the "auction" chip. */
  inAuction: boolean;
};

const open = (label: string, signs = true): NameRow["action"] => ({ label, kind: "open", signs });

/** What a name of ours (or one we bid on) needs, from its capabilities. */
function describeCaps(
  n: HsdName | undefined,
  c: NameActionCapabilities,
  clock: boolean,
): Described {
  const cd = countdown(c);
  const bid = `${short(c.bidValueDoos)} bid`;
  const value = n?.value != null ? short(n.value) : null;
  const redeemable = c.redeemableValueDoos > 0 ? `${short(c.redeemableValueDoos)} HNS ` : "";
  const sp = span(n, clock);
  const { phase } = auctionPhase(c.phase ?? n?.state);
  const auctionPhaseNow = phase === "OPENING" || phase === "BIDDING" || phase === "REVEAL";
  switch (c.taskState) {
    case "readyToReveal":
      return {
        status: "Reveal your bid",
        detail: cd && `${cd} left`,
        tone: "warn",
        value: bid,
        action: open("Reveal"),
        inAuction: true,
      };
    case "revealBroadcastPending":
      return {
        status: "Reveal sent",
        detail: `waiting to confirm${cd ? ` · auction closes in ${cd}` : ""}`,
        tone: "info",
        value: bid,
        action: null,
        inAuction: true,
      };
    case "revealDoneWaitingForClose":
      return {
        status: "Revealed",
        detail: `waiting for close${cd ? ` · ${cd}` : ""}`,
        tone: "info",
        value: bid,
        action: null,
        inAuction: true,
      };
    case "wonNeedsRegister":
      return {
        status: "You won",
        detail: "register it to make it yours",
        tone: "warn",
        value,
        action: open("Register"),
        inAuction: true,
      };
    case "lostNeedsRedeem":
      if (phase === "REVOKED") {
        return {
          status: "Revoked",
          detail: `reclaim your ${redeemable}lockup`,
          tone: "warn",
          value: redeemable ? `${short(c.redeemableValueDoos)} locked` : null,
          action: open("Redeem"),
          inAuction: false,
        };
      }
      if (c.ownsName) {
        return {
          status: "Owned",
          detail: redeemable ? `reclaim ${redeemable}from your other bids` : "reclaim your lockup",
          tone: "warn",
          value,
          action: open("Redeem"),
          inAuction: false,
        };
      }
      return {
        status: "You lost",
        detail: redeemable ? `get your ${redeemable}back` : "get your lockup back",
        tone: "warn",
        value: redeemable ? `${short(c.redeemableValueDoos)} locked` : null,
        action: open("Redeem"),
        inAuction: true,
      };
    case "lostUnrevealed":
      return {
        status: "Lost",
        detail: "bid not revealed in time — the lockup can't be reclaimed",
        tone: "muted",
        value: null,
        action: null,
        inAuction: auctionPhaseNow,
      };
    case "closedResultUnknown":
      return {
        status: "Auction closed",
        detail: "sync to see the result",
        tone: "info",
        value: bid,
        action: null,
        inAuction: true,
      };
    case "expiringSoon":
      return {
        status: "Renew it or lose it",
        detail: sp ? `expires in ${sp}` : null,
        tone: "bad",
        value,
        action: open("Renew"),
        inAuction: false,
      };
    case "ownedNoUrgentAction":
      return {
        status: "Owned",
        detail: sp ? `renew within ${sp}` : null,
        tone: "ok",
        value,
        action: null,
        inAuction: false,
      };
    case "transferPendingFinalize":
      return c.canFinalize.allowed
        ? {
            status: "Transferring",
            detail: "ready to finalize",
            tone: "info",
            value,
            action: open("Finalize"),
            inAuction: false,
          }
        : {
            status: "Transferring",
            detail: "finalize once the transfer lockup ends",
            tone: "info",
            value,
            action: null,
            inAuction: false,
          };
    case "readyToBid":
      return c.hasBidCommitment
        ? {
            status: "Bidding",
            detail: `your bid ${short(c.bidValueDoos)} HNS${cd ? ` · ${cd}` : ""}`,
            tone: "info",
            value: bid,
            action: null,
            inAuction: true,
          }
        : {
            status: "Bidding",
            detail: cd,
            tone: "info",
            value: null,
            action: open("Bid"),
            inAuction: true,
          };
    case "waitingForBidding":
      return {
        status: "Opening",
        detail: cd ? `bidding opens in ${cd}` : null,
        tone: "info",
        value: null,
        action: null,
        inAuction: true,
      };
    case "availableToOpen":
      return {
        status: "Opening the auction",
        detail: "waiting for the open to be mined",
        tone: "info",
        value: null,
        action: null,
        inAuction: true,
      };
    default:
      return describePhase(n, value, auctionPhaseNow);
  }
}

/** No capabilities (loading, watch-only, unavailable): the chain phase. */
function describePhase(
  n: HsdName | undefined,
  value: string | null,
  inAuction: boolean,
): Described {
  const t = n ? nextTransition(n.state, n.stats) : null;
  return {
    status: auctionPhase(n?.state).label,
    detail: t ? `${t.label.toLowerCase()} ${formatCountdown(t)}` : null,
    tone: "muted",
    value,
    action: null,
    inAuction,
  };
}

const WATCH_PHASE: Record<string, string> = {
  OPENING: "Opening",
  BIDDING: "Bidding",
  REVEAL: "In reveal",
  CLOSED: "Registered",
  REVOKED: "Revoked",
  AVAILABLE: "Available",
};

export function buildNameRows(input: NameRowsInput): NameRow[] {
  const clock = input.wallClock;
  const watchedBy = new Map(input.watched.map((w) => [w.name, w]));
  const rows: NameRow[] = [];
  const seen = new Set<string>();

  const withWatch = (r: Omit<NameRow, "watch" | "cats"> & { cats: NameCat[] }): NameRow => {
    const w = watchedBy.get(r.name);
    const cats = [...r.cats];
    if (w && !cats.includes("watching")) cats.push("watching");
    if (input.needs.includes(r.name) && !cats.includes("needs")) cats.unshift("needs");
    return { ...r, cats, watch: w ? { tags: w.tags, addedAt: w.addedAt } : null };
  };

  for (const n of input.owned) {
    seen.add(n.name);
    const base = {
      name: n.name,
      pending: null as string | null,
      height: n.height,
      renewal: n.renewal,
      expires: null as NameRow["expires"],
    };
    if (n.shakedex) {
      const s = n.shakedex;
      const sent = input.finalizeSent.has(s.purchaseId);
      const ready = s.state === "awaitingFinalize" && s.blocksRemaining === 0;
      rows.push(
        withWatch({
          ...base,
          kind: "purchase",
          cats: ["owned"],
          status: "Bought",
          detail:
            s.state === "unconfirmed"
              ? "waiting for the purchase to confirm"
              : sent
                ? "finalize sent"
                : ready
                  ? "ready to finalize"
                  : s.blocksRemaining != null
                    ? `awaiting finalize · ${s.blocksRemaining} ${s.blocksRemaining === 1 ? "block" : "blocks"} left`
                    : "awaiting finalize",
          tone: ready && !sent ? "warn" : "info",
          pending: sent ? "Finalize · waiting for a block" : null,
          statusTitle: "",
          value: `${short(s.priceDoos)} paid`,
          action: sent
            ? null
            : ready
              ? { label: "Finalize", kind: "finalizePurchase", signs: true }
              : { label: "View", kind: "viewPurchase", signs: false },
          selectable: false,
          purchaseId: s.purchaseId,
        }),
      );
      continue;
    }
    if (n.listing) {
      rows.push(
        withWatch({
          ...base,
          kind: "listing",
          cats: ["owned"],
          status: "Listed for sale",
          detail: listingStateLabel(n.listing.state, n.listing.blocksUntilFinalize),
          tone: "info",
          statusTitle: "",
          value: n.value != null ? short(n.value) : null,
          action: { label: MY_LISTINGS_TAB, kind: "myListings", signs: false },
          selectable: true,
          expires: expiresCell(n, clock, watchedBy.has(n.name)),
        }),
      );
      continue;
    }
    const c = input.capsByName.get(n.name);
    // "Opening the auction" speaks for a position with an open in flight; a
    // name of ours that merely could be opened reads its chain phase.
    const d =
      c && c.taskState !== "availableToOpen"
        ? describeCaps(n, c, clock)
        : describePhase(n, n.value != null ? short(n.value) : null, false);
    const pending = pendingBroadcastBadge(c?.pendingBroadcastAction);
    const { phase } = auctionPhase(c?.phase ?? n.state);
    const ownedNow = phase === "CLOSED" || phase === "TRANSFER" || phase === "REVOKED";
    const pendingTask =
      c?.taskState === "wonNeedsRegister" ||
      (c?.taskState === "lostNeedsRedeem" && phase !== "REVOKED");
    const cats: NameCat[] = [];
    if (d.inAuction) cats.push("auction");
    if (ownedNow && !pendingTask) cats.push("owned");
    rows.push(
      withWatch({
        ...base,
        kind: "owned",
        cats,
        status: d.status,
        detail: d.detail,
        tone: d.tone,
        pending,
        statusTitle: c?.countdownLabel && countdown(c) ? `${c.countdownLabel} ${countdown(c)}` : "",
        value: d.value,
        action: pending ? null : d.action,
        selectable: true,
        expires: pendingTask || d.inAuction ? null : expiresCell(n, clock, watchedBy.has(n.name)),
      }),
    );
  }

  for (const name of input.positions) {
    if (seen.has(name)) continue;
    const c = input.capsByName.get(name);
    if (!c || !ACTIVE_POSITION_TASK_STATES.has(c.taskState)) continue;
    seen.add(name);
    const d = describeCaps(undefined, c, clock);
    const pending = pendingBroadcastBadge(c.pendingBroadcastAction);
    rows.push(
      withWatch({
        name,
        kind: "auction",
        cats: ["auction"],
        status: d.status,
        detail: d.detail,
        tone: d.tone,
        pending,
        statusTitle: c.countdownLabel && countdown(c) ? `${c.countdownLabel} ${countdown(c)}` : "",
        value: d.value,
        expires: null,
        action: pending ? null : d.action,
        selectable: false,
        height: null,
        renewal: null,
      }),
    );
  }

  for (const w of input.watched) {
    if (seen.has(w.name)) continue;
    seen.add(w.name);
    const info = input.watchInfo[w.name];
    const state = input.watchStatus.find((s) => s.name === w.name)?.state ?? info?.state ?? null;
    const key = (state ?? "").toUpperCase();
    const status = state ? (WATCH_PHASE[key] ?? "Unknown state") : "Loading…";
    const t = info ? nextTransition(state, info.stats) : null;
    const detail =
      key === "CLOSED"
        ? info?.stats?.daysUntilExpire != null
          ? `expires in ${Math.floor(info.stats.daysUntilExpire)}d`
          : null
        : t
          ? `${t.label.toLowerCase()} ${formatCountdown(t)}`
          : null;
    const auctionNow = key === "OPENING" || key === "BIDDING" || key === "REVEAL";
    rows.push(
      withWatch({
        name: w.name,
        kind: "watch",
        cats: auctionNow ? ["watching", "auction"] : ["watching"],
        status,
        detail,
        tone: key === "BIDDING" || key === "REVEAL" ? "info" : "muted",
        pending: null,
        statusTitle: t?.label ?? "",
        value: info?.highest != null && info.highest > 0 ? `${short(info.highest)} high` : null,
        expires: key === "CLOSED" ? expiresCell(info, clock, true) : null,
        action: key === "BIDDING" ? open("Bid") : null,
        selectable: false,
        height: info?.height ?? null,
        renewal: info?.renewal ?? null,
      }),
    );
  }

  return sortRows(rows, input);
}

function group(r: NameRow): number {
  if (r.cats.includes("needs")) return 0;
  if (r.kind === "watch") return 4;
  if (r.cats.includes("auction")) return 1;
  if (r.kind === "listing" || r.kind === "purchase") return 3;
  return 2;
}

function sortRows(rows: NameRow[], input: NameRowsInput): NameRow[] {
  const needIndex = (n: string) => input.needs.indexOf(n);
  return rows.sort((a, b) => {
    const ga = group(a);
    const gb = group(b);
    if (ga !== gb) return ga - gb;
    if (ga === 0) return needIndex(a.name) - needIndex(b.name);
    if (ga === 1) {
      const ca = input.capsByName.get(a.name);
      const cb = input.capsByName.get(b.name);
      const ra = ca ? taskStateUrgencyRank(ca.taskState) : 4;
      const rb = cb ? taskStateUrgencyRank(cb.taskState) : 4;
      if (ra !== rb) return ra - rb;
      const ba = ca?.countdownBlocks ?? Number.POSITIVE_INFINITY;
      const bb = cb?.countdownBlocks ?? Number.POSITIVE_INFINITY;
      if (ba !== bb) return ba - bb;
    }
    return a.name.localeCompare(b.name);
  });
}

/** Rows per chip; `all` is every row. */
export function chipCounts(rows: NameRow[]): Record<"all" | NameCat, number> {
  const c = { all: rows.length, needs: 0, auction: 0, owned: 0, watching: 0 };
  for (const r of rows) for (const cat of r.cats) c[cat] += 1;
  return c;
}
