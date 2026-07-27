/**
 * Web-QA mock lifecycle-engine tests.
 *
 * The mock now drives auction phases from a virtual block height. These tests
 * walk a name through the full lifecycle (available -> opening -> bidding ->
 * reveal -> closed -> won/lost) via the invoke() surface, plus check the
 * reveal sub-lifecycle (broadcasted -> confirmed) that PR2 relies on.
 */
import { beforeEach, describe, expect, it } from "vitest";
import {
  mockInvoke,
  __resetChainForTests,
  __mineForTests,
} from "./webqa-mock";

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
    const bcast = mockInvoke<{ txid: string; status: string }>(
      "broadcast_tx_draft",
      { draftId: draft.id },
    );
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
