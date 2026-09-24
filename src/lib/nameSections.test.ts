import { describe, it, expect } from "vitest";
import { resolveSections } from "./nameSections";
import {
  allowed as yes,
  makeCapabilities as caps,
  refused as no,
} from "../test/fixtures/capabilities";

describe("resolveSections — leading your own auction is not owning the name", () => {
  // The case the whole three-state split exists for. During REVEAL hsd reports
  // the highest revealer as the owner, so `ownsName` is true while the wallet
  // holds nothing but a REVEAL coin. Gating on it put DNS records, Ownership
  // and Sign message on screen — and auto-expanded them — for a name that had
  // not been won.
  const revealLeader = caps({
    phase: "REVEAL",
    taskState: "revealDoneWaitingForClose",
    ownsName: true,
    transferPending: false,
    redeemableRevealCount: 0,
    redeemableValueDoos: 0,
    nameIsRegistered: false,
    hasRevealCoin: true,
  });

  it("holds records and ownership back, naming what unlocks them", () => {
    const s = resolveSections(revealLeader);
    expect(s.records.kind).toBe("upcoming");
    expect(s.ownership.kind).toBe("upcoming");
    if (s.records.kind === "upcoming") expect(s.records.when).toMatch(/register/i);
    if (s.ownership.kind === "upcoming") expect(s.ownership.when).toMatch(/register/i);
  });

  it("reports nothing live, so the modal has no advanced menu to open", () => {
    expect(resolveSections(revealLeader).anyLive).toBe(false);
  });

  it("opens the auction section as soon as there is a reveal to send", () => {
    const s = resolveSections(caps({ ...revealLeader, canReveal: yes }));
    expect(s.auction.kind).toBe("live");
    expect(s.anyLive).toBe(true);
  });
});

describe("resolveSections — while a transaction is waiting for a block", () => {
  // Between broadcast and the block the chain still reports the previous
  // state, and the honest answer is "nothing to do". A second menu offering
  // alternatives invites the user to send a competing transaction.
  it("offers nothing at all, whatever the phase says", () => {
    const s = resolveSections(
      caps({
        phase: "CLOSED",
        taskState: "ownedNoUrgentAction",
        ownsName: true,
        transferPending: false,
        redeemableRevealCount: 0,
        redeemableValueDoos: 0,
        nameIsRegistered: true,
        canUpdate: yes,
        canRenew: yes,
        pendingBroadcastAction: "update",
      }),
    );
    expect(s.auction.kind).toBe("absent");
    expect(s.records.kind).toBe("absent");
    expect(s.ownership.kind).toBe("absent");
    expect(s.anyLive).toBe(false);
  });
});

describe("resolveSections — records", () => {
  // Register lives inside the records section: it publishes the first
  // resource. Gating the section on "already registered" would take away the
  // only button the just-won stage has.
  it("is live on a won name that still needs registering", () => {
    const s = resolveSections(
      caps({
        taskState: "wonNeedsRegister",
        ownsName: true,
        transferPending: false,
        redeemableRevealCount: 0,
        redeemableValueDoos: 0,
        nameIsRegistered: false,
        canRegister: yes,
      }),
    );
    expect(s.records.kind).toBe("live");
    expect(s.anyLive).toBe(true);
    // Transfer, Renew and the rest still need a REGISTER to have happened.
    expect(s.ownership.kind).toBe("upcoming");
  });

  // hsd accepts TRANSFER -> UPDATE and that transition IS the cancel, so the
  // Update button would end the transfer while saying nothing about it.
  it("steps aside while a transfer is pending, and points at the button that means it", () => {
    const s = resolveSections(
      caps({
        phase: "TRANSFER",
        taskState: "transferPendingFinalize",
        ownsName: true,
        transferPending: true,
        redeemableRevealCount: 0,
        redeemableValueDoos: 0,
        nameIsRegistered: true,
        canFinalize: yes,
        canCancelTransfer: yes,
      }),
    );
    expect(s.records.kind).toBe("upcoming");
    if (s.records.kind === "upcoming") expect(s.records.when).toMatch(/cancel/i);
    expect(s.ownership.kind).toBe("live");
  });

  // The gate this mirrors is `can_update`, which keys on the transfer's items.
  // Deriving it from the task state instead is a second source of truth: the
  // two can disagree, and then the section says one thing and the button
  // inside it does another.
  it("follows the backend's transfer flag, not the phase-derived task state", () => {
    const s = resolveSections(
      caps({
        phase: "CLOSED",
        taskState: "ownedNoUrgentAction",
        ownsName: true,
        nameIsRegistered: true,
        transferPending: true,
        redeemableRevealCount: 0,
        redeemableValueDoos: 0,
        canUpdate: no("a transfer is pending — updating records would cancel it"),
      }),
    );
    expect(s.records.kind).toBe("upcoming");
  });

  it("is absent on a name this wallet has nothing to do with", () => {
    const s = resolveSections(caps({ phase: "BIDDING", ownsName: false }));
    expect(s.records.kind).toBe("absent");
    expect(s.ownership.kind).toBe("absent");
  });
});

describe("resolveSections — auction", () => {
  // On a registered name with nothing left to redeem the auction is history.
  // Keeping a fallback for Open/Reveal/Redeem there is noise on every managed
  // name the wallet holds.
  it("is gone once the name is registered and no auction step remains", () => {
    const s = resolveSections(
      caps({
        taskState: "ownedNoUrgentAction",
        ownsName: true,
        transferPending: false,
        redeemableRevealCount: 0,
        redeemableValueDoos: 0,
        nameIsRegistered: true,
        canUpdate: yes,
      }),
    );
    expect(s.auction.kind).toBe("absent");
    expect(s.anyLive).toBe(true); // records + ownership carry the name now
  });

  // A losing bidder still has a redeem to make on a name that is not theirs.
  it("stays live for a redeem on a name the wallet does not own", () => {
    const s = resolveSections(
      caps({ taskState: "lostNeedsRedeem", ownsName: false, canRedeem: yes }),
    );
    expect(s.auction.kind).toBe("live");
    expect(s.records.kind).toBe("absent");
    expect(s.anyLive).toBe(true);
  });
});
