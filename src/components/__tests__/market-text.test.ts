import { describe, it, expect } from "vitest";
import cases from "../../test/fixtures/sell-price-cases.json";
import { makeListing } from "../../test/fixtures/listings";
import {
  listingStatus,
  nodeReason,
  MAINNET_EXPERIMENTAL,
  NEEDS_SENDING_NODE,
  RECOVERY_PHRASE_ONLY,
  listedUntilText,
  purchaseRefusal,
  parseStepPrice,
  scheduleProblem,
  sellRefusal,
  canSellShakedex,
  listingStateLabel,
  listingPricesText,
  marketStatusText,
  modeText,
  stepTimeText,
  scheduleWarning,
  LISTING_BUTTONS,
  MAINNET_SELLING_EXPERIMENTAL,
  RESTORED_LABEL,
  SCHEDULE_PAST_A_YEAR,
} from "../market/marketText";
import type { ShakedexListingPrices, ShakedexListingState } from "../../types";

describe("listedUntilText", () => {
  it("dates the seller's expiresAt in UTC, and says the date does not end the listing", () => {
    expect(listedUntilText(1815232480)).toBe(
      "Listed until July 10, 2027 · stays buyable until sold or cancelled",
    );
  });

  it("gives no text for an expiresAt no date can hold, rather than throwing", () => {
    // expiresAt comes from a foreign listing file as any u64.
    expect(listedUntilText(Number.MAX_SAFE_INTEGER)).toBeNull();
    expect(listedUntilText(18446744073709552000)).toBeNull();
  });
});

describe("purchaseRefusal", () => {
  const send = { signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null };
  const noSend = { ...send, broadcasterAvailable: false, canWrite: false };

  it("refuses a mainnet purchase until the experimental flag is on, as the backend does", () => {
    expect(purchaseRefusal("mnemonic_hot", send, "mainnet", "false")).toBe(MAINNET_EXPERIMENTAL);
    expect(purchaseRefusal("mnemonic_hot", send, "mainnet", undefined)).toBe(MAINNET_EXPERIMENTAL);
    expect(purchaseRefusal("mnemonic_hot", send, "mainnet", "true")).toBeNull();
    expect(purchaseRefusal("mnemonic_hot", send, "regtest", "false")).toBeNull();
  });

  it("gives the profile and node refusals first, in the backend's order", () => {
    expect(purchaseRefusal("ledger_hardware", send, "mainnet", "false")).toBe(RECOVERY_PHRASE_ONLY);
    expect(purchaseRefusal("mnemonic_hot", noSend, "mainnet", "false")).toBe(NEEDS_SENDING_NODE);
  });
});

describe("nodeReason", () => {
  it("drops the layer's name from a node failure", () => {
    expect(
      nodeReason(
        "Node RPC error: the node refused the API key (HTTP 401): set the node's API key in Settings → Connections",
      ),
    ).toBe(
      "the node refused the API key (HTTP 401): set the node's API key in Settings → Connections",
    );
  });

  it("says a node that did not answer is not answering", () => {
    expect(
      nodeReason("HTTP error: error sending request for url (http://127.0.0.1:14037/)"),
    ).toMatch(/your node did not answer/);
    expect(nodeReason("HTTP error: operation timed out")).toMatch(/took too long/);
  });
});

describe("listingStatus", () => {
  const base = {
    listingJson: "{}",
    name: "n",
    kind: "buyNow",
    currentPrice: 1,
    nextPrice: null,
    nextValidInSecs: null,
    floorPrice: 1,
    steps: [],
    expiresAt: null,
  } as const;

  it("reads a buyable listing as ready, and warns when it expires soon", () => {
    const buyable = { verdict: "buyable", warnExpiry: false } as never;
    expect(listingStatus({ ...base, verdict: buyable } as never)).toEqual({
      tone: "ok",
      text: "Ready to buy",
    });
    const soon = { verdict: "buyable", warnExpiry: true } as never;
    expect(listingStatus({ ...base, verdict: soon } as never).tone).toBe("warn");
  });

  it("gives a listing that could not be checked the node's reason in plain words", () => {
    const v = {
      verdict: "hidden",
      kind: "couldNotCheck",
      reason: "Node RPC error: error sending request for url (http://127.0.0.1:14037/)",
    } as never;
    const s = listingStatus({ ...base, verdict: v } as never);
    expect(s.tone).toBe("warn");
    expect(s.text).toMatch(/^Could not be checked: your node did not answer/);
  });

  it("marks a listing that fails verification as bad", () => {
    const v = { verdict: "hidden", kind: "failedVerification", reason: "x" } as never;
    expect(listingStatus({ ...base, verdict: v } as never).tone).toBe("bad");
  });
});

describe("parseStepPrice / scheduleProblem", () => {
  it.each(cases.prices)("price $text reads as the backend reads it", ({ text, error }) => {
    const r = parseStepPrice(text);
    expect("error" in r ? r.error : null).toBe(error);
  });

  it("gives the doos of a price, never rounded", () => {
    expect(parseStepPrice(" 0.001 ")).toEqual({ doos: 1000n });
    expect(parseStepPrice("2040000000")).toEqual({ doos: 2_040_000_000_000_000n });
    expect(parseStepPrice(".5")).toEqual({ doos: 500_000n });
  });

  it.each(cases.schedules)("schedule %j is refused as the backend refuses it", (c) => {
    expect(scheduleProblem(c)).toBe(c.error);
  });

  // TS only: the backend's u32/u64 arguments cannot hold these, so its
  // fixture cannot either; Tauri refuses them before the command runs.
  it("refuses a step count or a time between steps that is not a whole number", () => {
    const base = { startPrice: "5", floorPrice: "3", steps: 3, intervalSecs: 900 };
    expect(scheduleProblem({ ...base, steps: 2.5 })).toBe(
      "reverse auction: 2.5 steps: a schedule has 2 to 1000 steps",
    );
    expect(scheduleProblem({ ...base, steps: NaN })).toBe(
      "reverse auction: NaN steps: a schedule has 2 to 1000 steps",
    );
    expect(scheduleProblem({ ...base, intervalSecs: NaN })).toBe(
      "reverse auction: the time between steps must be a whole number of seconds",
    );
    expect(scheduleProblem({ ...base, intervalSecs: 1500.5 })).toBe(
      "reverse auction: the time between steps must be a whole number of seconds",
    );
    expect(scheduleProblem({ ...base, intervalSecs: -900 })).toBe(
      "reverse auction: the time between steps must be a whole number of seconds",
    );
    // Refused before the prices are read, as the backend refuses the arguments first.
    expect(scheduleProblem({ ...base, startPrice: "x", steps: 2.5 })).toBe(
      "reverse auction: 2.5 steps: a schedule has 2 to 1000 steps",
    );
  });
});

describe("sellRefusal", () => {
  const cap = (send: boolean) => ({
    signerUnlocked: true,
    broadcasterAvailable: send,
    canWrite: send,
    reason: null,
  });

  it("refuses Ledger, watch-only and xpriv profiles with the R16 sentence", () => {
    for (const kind of ["ledger_hardware", "watch_only_xpub", "xpriv_hot"] as const)
      expect(sellRefusal(kind, cap(true), "regtest", undefined)).toBe(RECOVERY_PHRASE_ONLY);
  });

  it("needs the experimental flag on mainnet only", () => {
    expect(sellRefusal("mnemonic_hot", cap(true), "mainnet", "false")).toBe(
      MAINNET_SELLING_EXPERIMENTAL,
    );
    expect(sellRefusal("mnemonic_hot", cap(true), "mainnet", undefined)).toBe(
      MAINNET_SELLING_EXPERIMENTAL,
    );
    expect(sellRefusal("mnemonic_hot", cap(true), "mainnet", "true")).toBeNull();
    expect(sellRefusal("mnemonic_hot", cap(true), "regtest", undefined)).toBeNull();
    expect(canSellShakedex("mnemonic_hot", cap(true), "mainnet", "false")).toBe(false);
    expect(canSellShakedex("mnemonic_hot", cap(true), "mainnet", "true")).toBe(true);
    expect(canSellShakedex("mnemonic_hot", cap(true), undefined, "true")).toBe(false);
  });

  it("needs a node that can send", () => {
    expect(sellRefusal("mnemonic_hot", cap(false), "regtest", undefined)).toBe(NEEDS_SENDING_NODE);
    expect(canSellShakedex("mnemonic_hot", cap(false), "regtest", undefined)).toBe(false);
  });
});

describe("listingStateLabel / modeText", () => {
  it("counts the lockup while Locking and names a restored lock as R32 does", () => {
    expect(listingStateLabel("locking", 4)).toBe("Locking · 4 blocks");
    expect(listingStateLabel("locking", 1)).toBe("Locking · 1 block");
    expect(listingStateLabel("locking", null)).toBe("Locking");
    expect(listingStateLabel("restored", null)).toBe(RESTORED_LABEL);
    expect(RESTORED_LABEL).toBe("Locked — listing details unknown");
  });

  it("shows no mode for a restored lock (its mode is not known)", () => {
    expect(modeText(makeListing({ state: "restored", mode: "buyNow" }))).toBeNull();
    expect(modeText(makeListing({ mode: "reverseAuction" }))).toBe("Reverse auction");
    expect(modeText(makeListing({ mode: "buyNow" }))).toBe("Buy now");
  });
});

describe("marketStatusText", () => {
  it("says a refused pending listing is uploaded once the lock finalize is mined", () => {
    const t = marketStatusText(
      makeListing({
        state: "finalizing",
        publish: true,
        marketStatus: "refused",
        marketError: "pending listing already exists",
      }),
      "mainnet",
    );
    expect(t.text).toContain("refused the pending listing");
    expect(t.text).toContain("pending listing already exists");
    expect(t.text).toContain("once the finalize into the lock is mined");
  });

  it("says a refused listing once Listed is sent again only on a change", () => {
    const t = marketStatusText(
      makeListing({ publish: true, marketStatus: "refused", marketError: "bad" }),
      "mainnet",
    );
    expect(t.tone).toBe("bad");
    expect(t.text).toContain("sent again only when its price or expiry changes");
  });

  it("keeps R25's words for a replaced copy", () => {
    expect(
      marketStatusText(
        makeListing({ publish: true, marketStatus: "replacedReuploaded" }),
        "mainnet",
      ).text,
    ).toBe("Replaced on the market by someone else's copy — re-uploaded");
  });

  it("never says a published listing with no status yet goes on the market once it has ended or is restored", () => {
    const states = Object.keys(LISTING_BUTTONS) as ShakedexListingState[];
    expect(states).toHaveLength(13);
    const off = ["sold", "cancelled", "aborted", "expired", "restored"];
    const before = ["locking", "readyToFinalize", "finalizing"];
    for (const state of states) {
      const t = marketStatusText(
        makeListing({ state, publish: true, marketStatus: null }),
        "mainnet",
      );
      if (off.includes(state))
        expect(t, state).toEqual({ tone: "muted", text: "Not on LearnHNS Market" });
      else if (before.includes(state))
        expect(t.text, state).toBe("Announced on LearnHNS Market once the lock transfer is sent");
      else expect(t.text, state).toBe("Goes on LearnHNS Market at the next sync");
    }
  });

  it("says an unpublished listing is shared as a file only, and why off mainnet", () => {
    expect(marketStatusText(makeListing(), "mainnet").text).toBe(
      "Not published on LearnHNS Market — shared as a file only",
    );
    expect(marketStatusText(makeListing(), "regtest").text).toContain(
      "LearnHNS Market lists mainnet names only",
    );
  });
});

describe("stepTimeText / scheduleWarning", () => {
  it("gives no text for a lock time no date can hold", () => {
    expect(stepTimeText(2 ** 64 - 1)).toBeNull();
    expect(stepTimeText(1_700_000_000)).toMatch(/2023/);
  });

  it("warns when the schedule runs past a year", () => {
    expect(scheduleWarning(3, 60 * 60)).toBeNull();
    expect(scheduleWarning(366, 86_400)).toBeNull();
    expect(scheduleWarning(367, 86_400)).toBe(SCHEDULE_PAST_A_YEAR);
    expect(scheduleWarning(1000, 86_400)).toBe(SCHEDULE_PAST_A_YEAR);
    expect(SCHEDULE_PAST_A_YEAR).toBe(
      "The last price comes more than a year after the first, past the listing file's expiry date.",
    );
  });
});

describe("listingPricesText", () => {
  const at: ShakedexListingPrices = {
    kind: "at",
    currentPrice: 5_000_000,
    nextPrice: 4_000_000,
    nextValidInSecs: 1800,
    floorPrice: 3_000_000,
  };

  it("reads a reverse auction as now, next and floor, and a Buy Now as its price", () => {
    expect(listingPricesText(makeListing({ mode: "reverseAuction" }), at)).toBe(
      "Now 5.000000 HNS · next 4.000000 HNS in ~30 min · floor 3.000000 HNS",
    );
    expect(
      listingPricesText(makeListing({ mode: "reverseAuction" }), {
        ...at,
        currentPrice: null,
        nextValidInSecs: 7200,
      }),
    ).toBe("No price step valid yet · next 4.000000 HNS in ~2 h · floor 3.000000 HNS");
    expect(
      listingPricesText(makeListing({ mode: "reverseAuction" }), {
        ...at,
        currentPrice: 3_000_000,
        nextPrice: null,
        nextValidInSecs: null,
      }),
    ).toBe("Now 3.000000 HNS · floor 3.000000 HNS");
    expect(listingPricesText(makeListing({ mode: "buyNow" }), at)).toBe("Price 5.000000 HNS");
  });

  it("gives prices only for a Listed listing, whatever the stored steps say in any other state", () => {
    // LISTING_BUTTONS is a Record over ShakedexListingState: every state of ListingState::ALL.
    const states = Object.keys(LISTING_BUTTONS) as ShakedexListingState[];
    expect(states).toHaveLength(13);
    for (const state of states) {
      for (const mode of ["buyNow", "reverseAuction"] as const) {
        const text = listingPricesText(makeListing({ state, mode }), at);
        if (state === "listed") expect(text, `${state} ${mode}`).not.toBeNull();
        else expect(text, `${state} ${mode}`).toBeNull();
      }
      const others: ShakedexListingPrices[] = [
        { kind: "unverified" },
        { kind: "couldNotCheck", reason: "x" },
        { kind: "unreadable", reason: "x" },
      ];
      for (const p of others) {
        const text = listingPricesText(makeListing({ state }), p);
        if (state === "listed") expect(text, `${state} ${p.kind}`).not.toBeNull();
        else expect(text, `${state} ${p.kind}`).toBeNull();
      }
    }
  });

  it("says why there is no price", () => {
    const l = makeListing();
    expect(listingPricesText(l, { kind: "noSteps" })).toBeNull();
    expect(listingPricesText(l, { kind: "unverified" })).toBe(
      "Current price not checked — needs a full or remote node",
    );
    expect(
      listingPricesText(l, {
        kind: "couldNotCheck",
        reason: "HTTP error: error sending request for url (http://127.0.0.1:14037/)",
      }),
    ).toMatch(/^Current price could not be checked: your node did not answer/);
    expect(
      listingPricesText(l, { kind: "unreadable", reason: "corrupted listing: bad steps" }),
    ).toBe("The stored prices cannot be read: corrupted listing: bad steps");
  });
});

describe("LISTING_BUTTONS", () => {
  it("offers no action on an ended listing and Lower price only while it can be bought", () => {
    for (const state of ["sold", "cancelled", "expired", "aborted"] as const)
      expect(LISTING_BUTTONS[state]).toEqual([]);
    expect(LISTING_BUTTONS.salePending).not.toContain("lowerPrice");
    expect(LISTING_BUTTONS.listed).toEqual(["lowerPrice", "cancel", "export", "publish"]);
    expect(LISTING_BUTTONS.restored).toEqual(["cancel", "importFile"]);
  });
});
