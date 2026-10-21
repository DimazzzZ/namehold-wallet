/**
 * Web-QA mock lifecycle-engine tests.
 *
 * The mock now drives auction phases from a virtual block height. These tests
 * walk a name through the full lifecycle (available -> opening -> bidding ->
 * reveal -> closed -> won/lost) via the invoke() surface, plus check the
 * reveal sub-lifecycle (broadcasted -> confirmed) that PR2 relies on.
 */
import { beforeEach, describe, expect, it } from "vitest";
import { mockInvoke, __resetChainForTests, __mineForTests } from "./webqa-mock";
import fixture from "../test/fixtures/listing-actions.json";
import type {
  Settings,
  ShakedexListingState,
  ShakedexListingSummary,
  ShakedexMyListing,
  ShakedexNameListing,
} from "../types";

type Caps = {
  name: string;
  phase: string;
  taskState: string;
  hasBidCommitment: boolean;
  hasBidCoin: boolean;
  hasRevealCoin: boolean;
  revealTxid: string | null;
  bidValueDoos: number | null;
  canReveal: { allowed: boolean };
  countdownLabel: string | null;
  countdownBlocks: number | null;
};

function caps(name: string): Caps {
  return mockInvoke<Caps>("get_name_action_capabilities", { name });
}

describe("webqa-mock lifecycle engine", () => {
  beforeEach(() => {
    __resetChainForTests();
  });

  it("starts unopened names in AVAILABLE / availableToOpen", () => {
    const c = caps("brandnew");
    expect(c.phase).toBe("AVAILABLE");
    expect(c.taskState).toBe("availableToOpen");
    expect(c.hasBidCommitment).toBe(false);
    expect(c.revealTxid).toBeNull();
  });

  it("walks OPEN -> BIDDING -> REVEAL as blocks are mined", () => {
    mockInvoke("build_open_draft", { name: "foo" });
    expect(caps("foo").phase).toBe("OPENING");
    expect(caps("foo").taskState).toBe("waitingForBidding");
    __mineForTests(3);
    expect(caps("foo").phase).toBe("BIDDING");
    expect(caps("foo").taskState).toBe("readyToBid");
    mockInvoke("build_bid_draft", {
      name: "foo",
      bidValueDoos: 5_000_000,
      lockupDoos: 8_000_000,
    });
    expect(caps("foo").hasBidCommitment).toBe(true);
    __mineForTests(5);
    expect(caps("foo").phase).toBe("REVEAL");
    expect(caps("foo").taskState).toBe("readyToReveal");
    expect(caps("foo").canReveal.allowed).toBe(true);
  });

  it("surfaces the local bid value on capabilities", () => {
    mockInvoke("build_open_draft", { name: "bar" });
    __mineForTests(3);
    mockInvoke("build_bid_draft", {
      name: "bar",
      bidValueDoos: 7_500_000,
      lockupDoos: 10_000_000,
    });
    __mineForTests(5);
    expect(caps("bar").bidValueDoos).toBe(7_500_000);
  });

  it("moves reveal ready -> pending -> done through build/broadcast/mine", () => {
    mockInvoke("build_open_draft", { name: "baz" });
    __mineForTests(3);
    mockInvoke("build_bid_draft", {
      name: "baz",
      bidValueDoos: 5_000_000,
      lockupDoos: 8_000_000,
    });
    __mineForTests(5);
    let c = caps("baz");
    expect(c.taskState).toBe("readyToReveal");
    expect(c.revealTxid).toBeNull();

    const draft = mockInvoke<{ id: string }>("build_reveal_draft", {
      name: "baz",
    });
    const bcast = mockInvoke<{ txid: string; status: string }>("broadcast_tx_draft", {
      draftId: draft.id,
    });
    expect(bcast.status).toBe("broadcasted");
    expect(bcast.txid).toMatch(/^revealtxid/);
    c = caps("baz");
    expect(c.taskState).toBe("revealBroadcastPending");
    expect(c.revealTxid).toBe(bcast.txid);
    expect(c.canReveal.allowed).toBe(false);

    __mineForTests(1);
    c = caps("baz");
    expect(c.taskState).toBe("revealDoneWaitingForClose");
    expect(c.hasBidCoin).toBe(false);
    expect(c.hasRevealCoin).toBe(true);
  });

  it("resolves CLOSED to wonNeedsRegister when our bid beats the rival", () => {
    mockInvoke("build_open_draft", { name: "winme" });
    __mineForTests(3);
    mockInvoke("build_bid_draft", {
      name: "winme",
      bidValueDoos: 10_000_000,
      lockupDoos: 15_000_000,
    });
    __mineForTests(5);
    const draft = mockInvoke<{ id: string }>("build_reveal_draft", {
      name: "winme",
    });
    mockInvoke("broadcast_tx_draft", { draftId: draft.id });
    __mineForTests(6);
    const c = caps("winme");
    expect(c.phase).toBe("CLOSED");
    expect(c.taskState).toBe("wonNeedsRegister");
  });

  it("resolves CLOSED to lostNeedsRedeem when the rival outbids us", () => {
    mockInvoke("build_open_draft", { name: "lose" });
    __mineForTests(3);
    mockInvoke("build_bid_draft", {
      name: "lose",
      bidValueDoos: 1_000_000,
      lockupDoos: 2_000_000,
    });
    __mineForTests(5);
    const draft = mockInvoke<{ id: string }>("build_reveal_draft", {
      name: "lose",
    });
    mockInvoke("broadcast_tx_draft", { draftId: draft.id });
    __mineForTests(6);
    expect(caps("lose").taskState).toBe("lostNeedsRedeem");
  });

  it("lists a broadcasted reveal draft and flips it to confirmed on the next block", () => {
    mockInvoke("build_open_draft", { name: "draftname" });
    __mineForTests(3);
    mockInvoke("build_bid_draft", {
      name: "draftname",
      bidValueDoos: 5_000_000,
      lockupDoos: 8_000_000,
    });
    __mineForTests(5);
    const draft = mockInvoke<{ id: string }>("build_reveal_draft", {
      name: "draftname",
    });
    mockInvoke("broadcast_tx_draft", { draftId: draft.id });

    type Draft = { action: string; status: string; summary: { name: string } };
    let drafts = mockInvoke<Draft[]>("list_tx_drafts");
    expect(drafts).toHaveLength(1);
    expect(drafts[0]?.action).toBe("reveal");
    expect(drafts[0]?.status).toBe("broadcasted");
    expect(drafts[0]?.summary?.name).toBe("draftname");

    __mineForTests(1);
    drafts = mockInvoke<Draft[]>("list_tx_drafts");
    expect(drafts[0]?.status).toBe("confirmed");
  });

  it("computes an auction-close countdown while in REVEAL", () => {
    mockInvoke("build_open_draft", { name: "clocked" });
    __mineForTests(3);
    mockInvoke("build_bid_draft", {
      name: "clocked",
      bidValueDoos: 5_000_000,
      lockupDoos: 8_000_000,
    });
    __mineForTests(5);
    const c = caps("clocked");
    expect(c.phase).toBe("REVEAL");
    expect(c.countdownLabel).toBe("Auction closes in");
    expect(c.countdownBlocks).toBeGreaterThan(0);
  });

  it("advances the mock chain via the webqa_mine_blocks invoke command", () => {
    const before = mockInvoke<{ height: number }>("node_status").height;
    const after = mockInvoke<number>("webqa_mine_blocks", { count: 4 });
    expect(after).toBe(before + 4);
    expect(mockInvoke<{ height: number }>("node_status").height).toBe(after);
  });

  it("read_auction_position_names lists engine-tracked names once opened", () => {
    expect(mockInvoke("read_auction_position_names")).toEqual([]);
    mockInvoke("build_open_draft", { name: "listme" });
    expect(mockInvoke("read_auction_position_names")).toEqual(["listme"]);
  });
});

describe("webqa-mock selling", () => {
  beforeEach(() => __resetChainForTests());

  const rows = () => mockInvoke<ShakedexMyListing[]>("shakedex_list_listings");
  const row = () => rows()[0]!;
  // The actions a row allows, as the backend's `ListingActions` would
  // (`shakedex_my_listings_tests::listing_actions_follow_the_commands_refusals_in_every_state`).
  const allowed = () =>
    Object.entries(row().actions)
      .filter(([, a]) => a.allowed)
      .map(([k]) => k)
      .sort();
  const nameRow = (name: string) =>
    mockInvoke<{ name: string; listing?: ShakedexNameListing }[]>("read_names").find(
      (n) => n.name === name,
    );

  it("walks lock -> ready -> finalize & sign -> listed -> cancel -> home", () => {
    const d = mockInvoke<{ id: string; action: string }>("shakedex_build_lock_draft", {
      name: "example",
      mode: "buyNow",
      publish: false,
      feeRate: null,
    });
    expect(d.action).toBe("shakedex_lock");
    expect(row().listing.state).toBe("locking");
    expect(row().pendingSend).toEqual({
      draftId: d.id,
      action: "shakedex_lock",
      signed: false,
      discardable: true,
    });
    expect(nameRow("example")?.listing).toEqual({
      listingId: "mock-listing-example",
      state: "locking",
      blocksUntilFinalize: null,
    });
    expect(allowed()).toEqual([]);

    mockInvoke("sign_tx_draft", { draftId: d.id });
    expect(row().pendingSend?.signed).toBe(true);
    mockInvoke("broadcast_tx_draft", { draftId: d.id });
    expect(row().pendingSend).toBeNull();
    __mineForTests(1);
    // The regtest transfer lockup, counted from the lock's block as hsd
    // judges a FINALIZE at tip + 1.
    expect(nameRow("example")?.listing?.blocksUntilFinalize).toBe(9);
    expect(row().listing.blocksUntilFinalize).toBe(9);
    __mineForTests(9);
    expect(row().listing.state).toBe("readyToFinalize");
    expect(allowed()).toEqual(["finalizeAndSign"]);

    const l = mockInvoke<ShakedexListingSummary>("shakedex_finalize_and_sign", {
      listingId: "mock-listing-example",
      prices: [{ price: "5" }],
      schedule: null,
      feeRate: null,
    });
    expect(l.state).toBe("finalizing");
    expect(l.steps.map((s) => s.price)).toEqual([5_000_000]);
    expect(row().pendingSend).toEqual({
      draftId: l.finalizeDraftId,
      action: "shakedex_lock_finalize",
      signed: true,
      discardable: true,
    });
    mockInvoke("broadcast_tx_draft", { draftId: l.finalizeDraftId });
    __mineForTests(1);
    expect(row().listing.state).toBe("listed");
    expect(row().prices).toEqual({
      kind: "at",
      currentPrice: 5_000_000,
      nextPrice: null,
      nextValidInSecs: null,
      floorPrice: 5_000_000,
    });
    // The mock profile is mainnet: a Listed listing not published can be.
    expect(allowed()).toEqual(["cancel", "export", "lowerPrice", "publish"]);
    expect(nameRow("example")?.listing?.state).toBe("listed");

    const c = mockInvoke<{ id: string }>("shakedex_cancel_listing", {
      listingId: l.id,
      feeRate: null,
    });
    expect(row().listing.state).toBe("cancelling");
    expect(allowed()).toEqual(["export"]);
    mockInvoke("broadcast_tx_draft", { draftId: c.id });
    __mineForTests(1);
    expect(row().listing.state).toBe("cancelAwaitingFinalize");
    expect(row().listing.cancelBlocksRemaining).toBe(9);
    expect(row().actions.finalizeCancel).toEqual({
      allowed: false,
      reason: "the cancel can be finalized in 9 blocks",
    });
    __mineForTests(9);
    expect(allowed()).toEqual(["export", "finalizeCancel"]);
    const f = mockInvoke<{ id: string }>("shakedex_finalize_cancel", {
      listingId: l.id,
      feeRate: null,
    });
    expect(row().listing.state).toBe("cancelFinalizing");
    mockInvoke("sign_tx_draft", { draftId: f.id });
    mockInvoke("broadcast_tx_draft", { draftId: f.id });
    __mineForTests(1);
    expect(row().listing.state).toBe("cancelled");
    expect(allowed()).toEqual([]);
    // Home: the name's row no longer shows the listing.
    expect(nameRow("example")?.listing).toBeUndefined();
  });

  it("refuses a step the state does not allow, in the backend's words", () => {
    mockInvoke("shakedex_build_lock_draft", { name: "example", mode: "buyNow", publish: false });
    expect(() =>
      mockInvoke("shakedex_finalize_and_sign", {
        listingId: "mock-listing-example",
        prices: [{ price: "5" }],
        schedule: null,
      }),
    ).toThrow("Finalize & sign opens after the transfer lockup");
    expect(() =>
      mockInvoke("shakedex_build_lock_draft", { name: "example", mode: "buyNow", publish: false }),
    ).toThrow("'example' is already locked for sale");
  });

  it("discarding an unsent draft puts the listing back as the backend does", () => {
    const d = mockInvoke<{ id: string }>("shakedex_build_lock_draft", {
      name: "example",
      mode: "reverseAuction",
      publish: true,
    });
    mockInvoke("delete_tx_draft", { draftId: d.id });
    expect(rows()).toEqual([]);

    const d2 = mockInvoke<{ id: string }>("shakedex_build_lock_draft", {
      name: "example",
      mode: "reverseAuction",
      publish: true,
    });
    mockInvoke("broadcast_tx_draft", { draftId: d2.id });
    __mineForTests(10);
    const l = mockInvoke<ShakedexListingSummary>("shakedex_finalize_and_sign", {
      listingId: row().listing.id,
      prices: [],
      schedule: { startPrice: "10", floorPrice: "4", steps: 4, intervalSecs: 3600 },
    });
    expect(l.steps.map((s) => s.price)).toEqual([10_000_000, 8_000_000, 6_000_000, 4_000_000]);
    mockInvoke("delete_tx_draft", { draftId: l.finalizeDraftId });
    expect(row().listing.state).toBe("readyToFinalize");
    expect(row().listing.steps).toEqual([]);
  });

  it("a reverse auction's row gives its next price and when", () => {
    const d = mockInvoke<{ id: string }>("shakedex_build_lock_draft", {
      name: "example",
      mode: "reverseAuction",
      publish: false,
    });
    mockInvoke("broadcast_tx_draft", { draftId: d.id });
    __mineForTests(10);
    const l = mockInvoke<ShakedexListingSummary>("shakedex_finalize_and_sign", {
      listingId: row().listing.id,
      prices: [],
      schedule: { startPrice: "10", floorPrice: "4", steps: 4, intervalSecs: 3600 },
    });
    mockInvoke("broadcast_tx_draft", { draftId: l.finalizeDraftId });
    __mineForTests(1);
    const p = row().prices;
    expect(p.kind).toBe("at");
    if (p.kind !== "at") return;
    expect(p.currentPrice).toBe(10_000_000);
    expect(p.nextPrice).toBe(8_000_000);
    expect(p.nextValidInSecs).toBeGreaterThan(0);
    expect(p.floorPrice).toBe(4_000_000);
  });

  it("lowers a Listed price, publishes it, and exports its file", () => {
    const d = mockInvoke<{ id: string }>("shakedex_build_lock_draft", {
      name: "example",
      mode: "buyNow",
      publish: false,
    });
    mockInvoke("broadcast_tx_draft", { draftId: d.id });
    __mineForTests(10);
    const l = mockInvoke<ShakedexListingSummary>("shakedex_finalize_and_sign", {
      listingId: row().listing.id,
      prices: [{ price: "5" }],
      schedule: null,
    });
    mockInvoke("broadcast_tx_draft", { draftId: l.finalizeDraftId });
    __mineForTests(1);
    const lowered = mockInvoke<ShakedexListingSummary>("shakedex_lower_price", {
      listingId: l.id,
      price: "3",
    });
    expect(lowered.steps.map((s) => s.price)).toEqual([5_000_000, 3_000_000]);
    expect(row().prices).toMatchObject({ kind: "at", currentPrice: 3_000_000 });
    const published = mockInvoke<ShakedexListingSummary>("shakedex_publish_listing", {
      listingId: l.id,
    });
    expect(published.publish).toBe(true);
    expect(row().actions.publish).toEqual({
      allowed: false,
      reason: "this listing is already published on LearnHNS Market",
    });
    expect(
      JSON.parse(mockInvoke<string>("shakedex_export_listing_file", { listingId: l.id })),
    ).toMatchObject({ name: "example" });
    expect(mockInvoke("shakedex_save_listing_file", { listingId: l.id, path: "/tmp/x" })).toBe(
      null,
    );
  });

  it("a name locked for sale has no owner actions, as the backend's R27", () => {
    mockInvoke("build_open_draft", { name: "won" });
    __mineForTests(3);
    mockInvoke("build_bid_draft", { name: "won", bidValueDoos: 5_000_000, lockupDoos: 8_000_000 });
    __mineForTests(5);
    const reveal = mockInvoke<{ id: string }>("build_reveal_draft", { name: "won" });
    mockInvoke("broadcast_tx_draft", { draftId: reveal.id });
    __mineForTests(5);
    expect(caps("won").taskState).toBe("wonNeedsRegister");
    const transfer = () =>
      mockInvoke<{ canTransfer: { allowed: boolean; reason: string | null } }>(
        "get_name_action_capabilities",
        { name: "won" },
      ).canTransfer;
    expect(transfer()).toEqual({ allowed: true, reason: null });
    mockInvoke("shakedex_build_lock_draft", { name: "won", mode: "buyNow", publish: false });
    expect(transfer()).toEqual({
      allowed: false,
      reason: "this name is locked for sale through Shakedex",
    });
  });

  it("lowering a reverse auction mid-schedule makes the new price current", () => {
    const d = mockInvoke<{ id: string }>("shakedex_build_lock_draft", {
      name: "example",
      mode: "reverseAuction",
      publish: false,
    });
    mockInvoke("broadcast_tx_draft", { draftId: d.id });
    __mineForTests(10);
    const l = mockInvoke<ShakedexListingSummary>("shakedex_finalize_and_sign", {
      listingId: row().listing.id,
      prices: [],
      schedule: { startPrice: "10", floorPrice: "4", steps: 4, intervalSecs: 3600 },
    });
    mockInvoke("broadcast_tx_draft", { draftId: l.finalizeDraftId });
    // Six blocks an hour: past the second step, before the third.
    __mineForTests(7);
    expect(row().prices).toMatchObject({ currentPrice: 8_000_000, nextPrice: 6_000_000 });
    mockInvoke("shakedex_lower_price", { listingId: l.id, price: "7" });
    expect(row().prices).toMatchObject({
      kind: "at",
      currentPrice: 7_000_000,
      nextPrice: 6_000_000,
      floorPrice: 4_000_000,
    });
    // The next price is the soonest step cheaper than the current one: the
    // dearer 6 HNS step that comes first is passed over.
    mockInvoke("shakedex_lower_price", { listingId: l.id, price: "5" });
    expect(row().prices).toMatchObject({ currentPrice: 5_000_000, nextPrice: 4_000_000 });
  });

  it("a Cancel transfer of a name still locking aborts its listing once mined", () => {
    mockInvoke("build_open_draft", { name: "won" });
    __mineForTests(3);
    mockInvoke("build_bid_draft", { name: "won", bidValueDoos: 5_000_000, lockupDoos: 8_000_000 });
    __mineForTests(5);
    const reveal = mockInvoke<{ id: string }>("build_reveal_draft", { name: "won" });
    mockInvoke("broadcast_tx_draft", { draftId: reveal.id });
    __mineForTests(5);
    type Cap = { allowed: boolean; reason: string | null };
    const capsOf = () =>
      mockInvoke<{ canCancelTransfer: Cap; canTransfer: Cap }>("get_name_action_capabilities", {
        name: "won",
      });
    const lock = mockInvoke<{ id: string }>("shakedex_build_lock_draft", {
      name: "won",
      mode: "buyNow",
      publish: false,
    });
    expect(capsOf().canCancelTransfer.allowed).toBe(false);
    mockInvoke("broadcast_tx_draft", { draftId: lock.id });
    __mineForTests(1);
    expect(row().listing.state).toBe("locking");
    // During the lockup the pending abort is the reason, not the lockup
    // (`finalize_and_sign_refusal`).
    const lockupReason = row().actions.finalizeAndSign.reason;
    const early = mockInvoke<{ id: string }>("build_cancel_draft", { name: "won" });
    expect(row().actions.finalizeAndSign).toEqual({
      allowed: false,
      reason: fixture.cancelTransferPending,
    });
    mockInvoke("delete_tx_draft", { draftId: early.id });
    expect(row().actions.finalizeAndSign).toEqual({ allowed: false, reason: lockupReason });
    __mineForTests(9);
    expect(row().listing.state).toBe("readyToFinalize");
    expect(capsOf().canCancelTransfer).toEqual({ allowed: true, reason: null });

    // An unsent Cancel transfer, discarded, aborts nothing.
    const first = mockInvoke<{ id: string; action: string }>("build_cancel_draft", { name: "won" });
    expect(first.action).toBe("cancel");
    expect(row().actions.finalizeAndSign).toEqual({
      allowed: false,
      reason: fixture.cancelTransferPending,
    });
    mockInvoke("delete_tx_draft", { draftId: first.id });
    expect(row().actions.finalizeAndSign.allowed).toBe(true);

    const c = mockInvoke<{ id: string }>("build_cancel_draft", { name: "won" });
    mockInvoke("broadcast_tx_draft", { draftId: c.id });
    expect(row().listing.state).toBe("readyToFinalize");
    __mineForTests(1);
    expect(row().listing.state).toBe("aborted");
    expect(allowed()).toEqual([]);
    expect(nameRow("won")?.listing).toBeUndefined();
    expect(capsOf().canTransfer.allowed).toBe(true);
  });

  it("the backend's listing-actions fixture has a case for every state the mock looks up", () => {
    const states: ShakedexListingState[] = [
      "locking",
      "readyToFinalize",
      "finalizing",
      "listed",
      "salePending",
      "sold",
      "cancelling",
      "cancelAwaitingFinalize",
      "cancelFinalizing",
      "cancelled",
      "aborted",
      "restored",
      "expired",
    ];
    for (const state of states) {
      for (const withFile of [false, true]) {
        for (const published of [false, true]) {
          expect(
            fixture.cases.filter(
              (c) => c.state === state && c.withFile === withFile && c.published === published,
            ),
            `${state} ${withFile} ${published}`,
          ).toHaveLength(1);
        }
      }
    }
  });

  it("keeps a changed setting, so mainnet selling can be turned on in browser QA", () => {
    expect(mockInvoke<Settings>("get_settings").shakedex_experimental).toBe("false");
    mockInvoke("update_setting", { key: "shakedex_experimental", value: "true" });
    expect(mockInvoke<Settings>("get_settings").shakedex_experimental).toBe("true");
    __resetChainForTests();
    expect(mockInvoke<Settings>("get_settings").shakedex_experimental).toBe("false");
  });

  it("restores a lock by name and makes it Listed with its own file", () => {
    const r = mockInvoke<ShakedexListingSummary>("shakedex_restore_lock", { name: "dexsale" });
    expect(r.state).toBe("restored");
    expect(allowed()).toEqual(["cancel", "importFile"]);
    // A name in our lock has a row of its own at the lock address.
    expect(nameRow("dexsale")?.listing?.state).toBe("restored");
    const listed = mockInvoke<ShakedexListingSummary>("shakedex_import_own_listing_file", {
      text: JSON.stringify({ name: "dexsale" }),
    });
    expect(listed.state).toBe("listed");
    expect(allowed()).toEqual(["cancel", "export", "lowerPrice", "publish"]);
    expect(() =>
      mockInvoke("shakedex_import_own_listing_from_file", { path: "/tmp/dexsale.json" }),
    ).toThrow(
      "only a lock restored by name, or a listing managed from another device, takes its saved listing file",
    );
  });
});
