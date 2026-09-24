import type { NameActionCapabilities, NameActionCapability } from "../../types";

/** A refused capability, carrying the reason a test wants to assert on. */
export const refused = (reason: string | null = null): NameActionCapability => ({
  allowed: false,
  reason,
});

/** An allowed capability. */
export const allowed: NameActionCapability = { allowed: true, reason: null };

/**
 * A complete `NameActionCapabilities` with every action refused; each test
 * overrides only what it is about.
 *
 * Complete on purpose. Tests that built this object by hand either listed
 * every field — and then had to be found and edited whenever the backend grew
 * one — or reached for `as unknown as NameActionCapabilities`, which switches
 * the type check off entirely and lets a test assert against a shape the
 * backend never sends. Adding a field to the type is now a type error here and
 * nowhere else.
 *
 * The defaults are the conservative answer the backend itself falls back to:
 * nothing owned, nothing registered, nothing allowed.
 */
export function makeCapabilities(
  over: Partial<NameActionCapabilities> = {},
): NameActionCapabilities {
  const denied = refused();
  return {
    name: "example",
    phase: "CLOSED",
    taskState: "unavailableOther",
    ownsName: false,
    nameIsRegistered: false,
    transferPending: false,
    hasBidCommitment: false,
    hasBidCoin: false,
    hasRevealCoin: false,
    hasOwnerCoin: false,
    revealTxid: null,
    bidValueDoos: null,
    lockupValueDoos: null,
    myBidCount: 0,
    canOpen: denied,
    canBid: denied,
    canReveal: denied,
    canRedeem: denied,
    canRegister: denied,
    canUpdate: denied,
    canTransfer: denied,
    canFinalize: denied,
    canCancelTransfer: denied,
    canRenew: denied,
    canRevoke: denied,
    nextActionKey: null,
    nextActionLabel: null,
    nextActionReason: null,
    countdownLabel: null,
    countdownBlocks: null,
    countdownHours: null,
    auctionBiddingBlocks: null,
    auctionRevealBlocks: null,
    pendingBroadcastAction: null,
    strandedBidCount: 0,
    strandedLockupDoos: 0,
    redeemableRevealCount: 0,
    redeemableValueDoos: 0,
    ...over,
  };
}
