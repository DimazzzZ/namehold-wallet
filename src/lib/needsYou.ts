import type { HsdName, NameActionCapabilities, RenewalRow, WalletNetwork } from "../types";
import type { StatusTone } from "../components/market/marketText";
import { formatHnsShort } from "./utils";

/**
 * "What needs you": the single list of tasks the wallet owes the chain, shared
 * by the Wallet's Needs you card, the nav count and the Names filter. Built
 * from the backend's capability verdicts (`taskState`), chain renewals and
 * Shakedex purchases — never from raw auction phases.
 */
export type NeedsYouKind =
  "reveal" | "renew" | "finalizeTransfer" | "finalizePurchase" | "register" | "redeem";

export interface NeedsYouItem {
  kind: NeedsYouKind;
  /** ACE name; render with displayName(). */
  name: string;
  text: string;
  when: string;
  tone: StatusTone;
  actionLabel: "Reveal" | "Renew" | "Finalize" | "Register" | "Redeem";
  /** For sorting within a kind; null sorts last. */
  blocks: number | null;
  /** finalizePurchase only. */
  purchaseId?: string;
}

export interface NeedsYouInput {
  isWatchOnly: boolean;
  network: WalletNetwork | null;
  names: HsdName[];
  caps: NameActionCapabilities[];
  renewals: RenewalRow[];
  finalizeSent: Set<string | undefined>;
}

const KIND_ORDER: NeedsYouKind[] = [
  "reveal",
  "renew",
  "finalizeTransfer",
  "finalizePurchase",
  "register",
  "redeem",
];

/** `~50m`, `~6h`, `~3 days`, or `Mar 2027` once it is months away. */
function humanHours(hours: number, now: number): string {
  if (hours < 1.5) return `~${Math.max(1, Math.round(hours * 60))}m`;
  if (hours < 36) return `~${Math.round(hours)}h`;
  const days = Math.round(hours / 24);
  if (days < 60) return `~${days} ${days === 1 ? "day" : "days"}`;
  return new Date(now + hours * 3_600_000).toLocaleString("en-US", {
    month: "short",
    year: "numeric",
  });
}

/**
 * Time left on a task. Hours come from the backend only (DESIGN_SYSTEM §1.9):
 * blocks are never multiplied out here, and a chain that mines on demand
 * (regtest) shows blocks alone.
 */
export function needsYouWhen(
  blocks: number | null,
  hours: number | null,
  network: WalletNetwork | null,
  now: number = Date.now(),
): string {
  const blockText =
    blocks == null
      ? null
      : `${blocks.toLocaleString("en-US")} ${blocks === 1 ? "block" : "blocks"}`;
  const wallClock = network === "mainnet" || network === "testnet";
  if (wallClock && hours != null) {
    const human = humanHours(hours, now);
    return blockText ? `${human} · ${blockText}` : human;
  }
  return blockText ?? "soon";
}

export function needsYouItems(input: NeedsYouInput, now: number = Date.now()): NeedsYouItem[] {
  if (input.isWatchOnly) return [];
  const { network } = input;
  const items: NeedsYouItem[] = [];

  // The renewals row comes first so it wins the name+kind dedupe below.
  for (const r of input.renewals) {
    // CSV-imported expiry is stale by definition: never a "renew now".
    if (!r.expiringSoon || r.source !== "chain") continue;
    items.push({
      kind: "renew",
      name: r.name,
      text: "Renew before it expires — an expired name is lost for good",
      when: needsYouWhen(
        r.blocksUntilExpire,
        r.daysUntilExpire == null ? null : r.daysUntilExpire * 24,
        network,
        now,
      ),
      tone: "bad",
      actionLabel: "Renew",
      blocks: r.blocksUntilExpire,
    });
  }

  for (const c of input.caps) {
    const when = needsYouWhen(c.countdownBlocks, c.countdownHours, network, now);
    switch (c.taskState) {
      case "readyToReveal":
        items.push({
          kind: "reveal",
          name: c.name,
          text:
            c.lockupValueDoos == null
              ? "Reveal your bid, or its lockup can't be reclaimed"
              : `Reveal your bid, or the ${formatHnsShort(c.lockupValueDoos)} HNS lockup can't be reclaimed`,
          when,
          tone: "warn",
          actionLabel: "Reveal",
          blocks: c.countdownBlocks,
        });
        break;
      case "expiringSoon":
        items.push({
          kind: "renew",
          name: c.name,
          text: "Renew before it expires — an expired name is lost for good",
          when,
          tone: "bad",
          actionLabel: "Renew",
          blocks: c.countdownBlocks,
        });
        break;
      case "transferPendingFinalize":
        // While the transfer lockup runs this is a wait, not a task.
        if (!c.canFinalize.allowed) break;
        items.push({
          kind: "finalizeTransfer",
          name: c.name,
          text: "Transfer is ready — finalize it to complete",
          when: "Ready now",
          tone: "ok",
          actionLabel: "Finalize",
          blocks: null,
        });
        break;
      case "wonNeedsRegister":
        items.push({
          kind: "register",
          name: c.name,
          text: "You won — register it to make it yours",
          when: "No rush",
          tone: "ok",
          actionLabel: "Register",
          blocks: null,
        });
        break;
      case "lostNeedsRedeem": {
        const known = c.redeemableValueDoos > 0;
        const value = formatHnsShort(c.redeemableValueDoos);
        // Owning the name means these are our own outbid reveals, not a loss.
        const text = c.ownsName
          ? known
            ? `Reclaim the ${value} HNS locked in your other bids`
            : "Reclaim your lockup"
          : known
            ? `You lost — reclaim the ${value} HNS lockup`
            : "You lost — reclaim your lockup";
        items.push({
          kind: "redeem",
          name: c.name,
          text,
          when: "No rush",
          tone: "muted",
          actionLabel: "Redeem",
          blocks: null,
        });
        break;
      }
    }
  }

  for (const n of input.names) {
    const s = n.shakedex;
    if (
      s?.state !== "awaitingFinalize" ||
      s.blocksRemaining !== 0 ||
      input.finalizeSent.has(s.purchaseId)
    )
      continue;
    items.push({
      kind: "finalizePurchase",
      name: n.name,
      text: "Your purchase is ready — finalize it to own the name",
      when: "Ready now",
      tone: "ok",
      actionLabel: "Finalize",
      blocks: null,
      purchaseId: s.purchaseId,
    });
  }

  const seen = new Set<string>();
  const unique = items.filter((i) => {
    const key = `${i.kind}\u0000${i.name}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  const rank = (b: number | null) => (b == null ? Number.POSITIVE_INFINITY : b);
  return unique.sort(
    (a, b) =>
      KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) || rank(a.blocks) - rank(b.blocks),
  );
}
