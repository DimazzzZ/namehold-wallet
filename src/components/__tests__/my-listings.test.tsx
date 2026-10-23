import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within, cleanup } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
const saveMock = vi.fn();
const openMock = vi.fn();
vi.mock("../../lib/dialog", () => ({
  save: (...a: unknown[]) => saveMock(...a),
  open: (...a: unknown[]) => openMock(...a),
}));

import { MyListings } from "../market/MyListings";
import {
  CANCEL_MARKET_TOLD,
  CANCEL_MEMPOOL_PURCHASE,
  CANCEL_STILL_BUYABLE,
  CANCEL_STILL_BUYABLE_AS_IT_FALLS,
  CANCEL_THEN_FINALIZE,
  CANCELLED_ELSEWHERE,
  EXPORT_THIS_DEVICE_ONLY,
  LISTING_BUTTONS,
  LOWER_DURING_CANCEL,
  LOWER_MANAGED_ELSEWHERE,
  MANAGED_ELSEWHERE,
  MANAGE_HERE_LABEL,
  MANAGE_HERE_TEXT,
  ON_THIS_DEVICE,
  SOLD_ELSEWHERE,
  TAKE_OVER_OTHER_ADDRESS,
  cancelElsewhereText,
  managedElsewherePriceText,
  LOWER_PRICE_PERMANENCE,
  MAINNET_SELLING_EXPERIMENTAL,
  NEEDS_SENDING_NODE,
  PUBLISH_PRIVACY,
  RECOVERY_PHRASE_ONLY,
  RESTORED_LABEL,
  SEND_AGAIN_KEPT,
  UNSENT_KEPT,
} from "../market/marketText";
import { makeActions, makeListing, makeMyListing } from "../../test/fixtures/listings";
import { loadSettings } from "../../test/fixtures/settings";
import { makeProfile, makeSession } from "../../test/fixtures/wallet";
import { useUiStore } from "../../stores/ui";
import type {
  ShakedexListingState,
  ShakedexMyListing,
  ShakedexPendingSend,
  WalletProfileKind,
} from "../../types";

/** A row's draft as `pending_send` gives it. */
const pending = (
  draftId: string,
  over: Partial<ShakedexPendingSend> = {},
): ShakedexPendingSend => ({
  draftId,
  action: "shakedex_lock",
  signed: true,
  discardable: true,
  ...over,
});
/** The node did not take it (`tx_evidence::NOT_TAKEN`): the draft is `broadcast_pending`. */
const NOT_TAKEN =
  "The node did not take the transaction. hsd does not say why; often its coins are already spent elsewhere. You can try again; the coins stay held until the node is checked again.";
const toasts = () => useUiStore.getState().toastQueue.map((t) => t.message);

const STATES: ShakedexListingState[] = [
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
/** The buttons each state shows, written out so a change to `LISTING_BUTTONS` fails here. */
const EXPECTED_BUTTONS: Record<ShakedexListingState, string[]> = {
  locking: ["finalizeAndSign"],
  readyToFinalize: ["finalizeAndSign"],
  finalizing: [],
  listed: ["lowerPrice", "cancel", "export", "publish"],
  salePending: ["export"],
  cancelling: ["lowerPrice", "export"],
  cancelAwaitingFinalize: ["finalizeCancel", "export"],
  cancelFinalizing: ["export"],
  sold: [],
  cancelled: [],
  aborted: [],
  expired: [],
  restored: ["cancel", "importFile"],
};
const BUTTON_ID: Record<string, string> = {
  finalizeAndSign: "listing-finalize-and-sign",
  lowerPrice: "listing-lower-price",
  cancel: "listing-cancel",
  finalizeCancel: "listing-finalize-cancel",
  export: "listing-export",
  publish: "listing-publish",
  importFile: "listing-import-file",
};

interface Opts {
  kind?: WalletProfileKind;
  network?: "mainnet" | "regtest";
  /** False: a node that cannot send. */
  canSend?: boolean;
  /** The signer as the backend holds it: signing commands refuse "Wallet locked" until unlocked. */
  unlocked?: boolean;
  /** Per-command answers over the defaults. */
  answers?: Record<string, (args: unknown) => Promise<unknown>>;
  /** What `shakedex_list_listings` answers from its second call on (default: `rows`). */
  later?: ShakedexMyListing[];
}

function setup(rows: ShakedexMyListing[], o: Opts = {}) {
  let unlocked = o.unlocked ?? true;
  const needsSigner = (v: unknown) =>
    unlocked ? Promise.resolve(v) : Promise.reject("Wallet locked");
  let reads = 0;
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string, args: unknown) => {
    const answer = o.answers?.[cmd];
    if (answer) return answer(args);
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([
          makeProfile({ kind: o.kind ?? "mnemonic_hot", network: o.network ?? "mainnet" }),
        ]);
      case "get_signer_session":
        return Promise.resolve(makeSession({ unlocked }));
      case "unlock_local_signer":
        unlocked = true;
        return Promise.resolve(null);
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: unlocked,
          broadcasterAvailable: o.canSend ?? true,
          canWrite: unlocked && (o.canSend ?? true),
          reason: null,
        });
      case "shakedex_list_listings":
        reads += 1;
        return Promise.resolve(reads > 1 && o.later ? o.later : rows);
      case "shakedex_lock_search_status":
        return Promise.resolve({ done: true, addressIndex: true, candidates: 0 });
      case "shakedex_cancel_listing":
        return needsSigner({ id: "cancel-1" });
      case "shakedex_finalize_cancel":
        return needsSigner({ id: "home-1" });
      case "shakedex_lower_price":
        return needsSigner(rows[0]!.listing);
      case "shakedex_publish_listing":
        return needsSigner(rows[0]!.listing);
      case "sign_tx_draft":
        return needsSigner({ id: (args as { draftId: string }).draftId });
      case "broadcast_tx_draft":
        return Promise.resolve({
          draftId: (args as { draftId: string }).draftId,
          txid: "cc",
          status: "broadcasted",
        });
      default:
        return Promise.resolve(null);
    }
  });
}

function renderIt() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(<MyListings />, {
    wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    ),
  });
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);
const cmds = () => invokeMock.mock.calls.map((c) => c[0] as string);

/** The signing commands, in the order they ran (reads left out). */
const signingOrder = () =>
  cmds().filter((c) =>
    [
      "unlock_local_signer",
      "shakedex_cancel_listing",
      "shakedex_finalize_cancel",
      "shakedex_lower_price",
      "shakedex_publish_listing",
      "sign_tx_draft",
      "broadcast_tx_draft",
      "delete_tx_draft",
    ].includes(c),
  );

beforeEach(() => {
  loadSettings({ shakedex_experimental: "true" });
  useUiStore.setState({ toastQueue: [], toastMessage: null });
  saveMock.mockReset();
  openMock.mockReset();
});

describe("MyListings", () => {
  it.each(STATES)("a %s listing shows exactly the buttons its state allows", async (state) => {
    // Enabled or not is the backend's (`shakedex_my_listings_tests`); which
    // buttons a state shows is this screen's.
    setup([makeMyListing(state)]);
    renderIt();
    const row = await screen.findByTestId("my-listing-row");
    const shown = Object.entries(BUTTON_ID)
      .filter(([, id]) => within(row).queryByTestId(id) !== null)
      .map(([k]) => k)
      .sort();
    expect(shown).toEqual([...EXPECTED_BUTTONS[state]].sort());
    expect([...LISTING_BUTTONS[state]].sort()).toEqual(shown);
  });

  it("shows no row before the profile is known: its gates are not", async () => {
    setup([makeMyListing("listed")], {
      answers: { list_wallet_profiles: () => new Promise(() => {}) },
    });
    renderIt();
    expect(await screen.findByText("Loading your listings…")).toBeInTheDocument();
    await new Promise((r) => setTimeout(r, 20));
    // The listings are read for a profile (R7): none is asked for before it is known.
    expect(calls("shakedex_list_listings")).toHaveLength(0);
    expect(screen.queryByTestId("my-listing-row")).not.toBeInTheDocument();
  });

  it("says when there are no listings", async () => {
    setup([]);
    renderIt();
    expect(
      await screen.findByText("No listings yet. Sell a name from its actions in Owned Names."),
    ).toBeInTheDocument();
  });

  it.each(["list_wallet_profiles", "get_write_capability"])(
    "shows no row, and says why, when %s fails: the gates are not known",
    async (cmd) => {
      setup([makeMyListing("listed")], {
        answers: { [cmd]: () => Promise.reject("database is locked") },
      });
      renderIt();
      expect(
        await screen.findByText("Could not check what this wallet can sign"),
      ).toBeInTheDocument();
      expect(screen.queryByTestId("my-listing-row")).not.toBeInTheDocument();
    },
  );

  it("says why the list could not be read", async () => {
    setup([], { answers: { shakedex_list_listings: () => Promise.reject("database is locked") } });
    renderIt();
    expect(await screen.findByText("Could not load your listings")).toBeInTheDocument();
    expect(screen.getByText("database is locked")).toBeInTheDocument();
  });

  it("disables Lower price while cancelling, with the backend's reason", async () => {
    setup([
      makeMyListing("cancelling", {
        actions: makeActions({ lowerPrice: { allowed: false, reason: LOWER_DURING_CANCEL } }),
      }),
    ]);
    renderIt();
    expect(await screen.findByTestId("listing-lower-price")).toBeDisabled();
    expect(screen.getByText(LOWER_DURING_CANCEL)).toBeInTheDocument();
  });

  it("names the lockup left on a Locking row, as the Owned Names row does", async () => {
    const row = makeMyListing("locking");
    setup([{ ...row, listing: { ...row.listing, blocksUntilFinalize: 9 } }]);
    renderIt();
    expect(await screen.findByText("Locking · 9 blocks")).toBeInTheDocument();
  });

  describe("R35: when buyers stop seeing the listing", () => {
    /** A row whose name stops being buyable in `blocks`, on mainnet unless told. */
    const expiring = (
      blocks: number | null,
      state: ShakedexListingState = "listed",
      over: Partial<ShakedexMyListing> = {},
    ): ShakedexMyListing => {
      const row = makeMyListing(state, over);
      return { ...row, listing: { ...row.listing, blocksUntilUnbuyable: blocks } };
    };
    const line = async () =>
      (await screen.findByTestId("my-listing-row")).querySelector(
        "[data-testid='listing-unbuyable']",
      );

    it("says in days, within 30 days on mainnet, when buyers stop seeing it", async () => {
      setup([expiring(1_440)]);
      renderIt();
      expect(await screen.findByTestId("listing-unbuyable")).toHaveTextContent(
        "Buyers stop seeing this listing in ~10 days, when the name gets too close to expiring to finalize a purchase. Cancel it and bring the name home to renew it, then list it again.",
      );
    });

    it("says it on a restored lock too", async () => {
      setup([expiring(4_320, "restored")]);
      renderIt();
      expect(await screen.findByTestId("listing-unbuyable")).toHaveTextContent(
        "Buyers stop seeing this listing in ~30 days,",
      );
    });

    it("says it in blocks where blocks are mined on demand", async () => {
      setup([expiring(42, "listed", { unbuyableWarnBlocks: 300, wallClockTiming: false })], {
        network: "regtest",
      });
      renderIt();
      expect(await screen.findByTestId("listing-unbuyable")).toHaveTextContent(
        "Buyers stop seeing this listing in 42 blocks, when the name gets too close to expiring to finalize a purchase. Cancel it and bring the name home to renew it, then list it again.",
      );
    });

    it("says buyers no longer see it once past that point", async () => {
      setup([expiring(0)]);
      renderIt();
      expect(await screen.findByTestId("listing-unbuyable")).toHaveTextContent(
        "Buyers no longer see this listing: the name is too close to expiring to finalize a purchase. Cancel it and bring the name home to renew it.",
      );
    });

    it.each([
      ["outside 30 days", 4_321],
      ["when not known", null],
    ])("says nothing %s", async (_, blocks) => {
      setup([expiring(blocks)]);
      renderIt();
      expect(await line()).toBeNull();
    });
  });

  it("shows a refused action disabled with its reason, never hides it", async () => {
    setup([
      makeMyListing("locking", {
        actions: makeActions({
          finalizeAndSign: {
            allowed: false,
            reason: "Finalize & sign opens after the transfer lockup",
          },
        }),
      }),
    ]);
    renderIt();
    expect(await screen.findByTestId("listing-finalize-and-sign")).toBeDisabled();
    expect(screen.getByText("Finalize & sign opens after the transfer lockup")).toBeInTheDocument();
  });

  it.each([
    [{ publish: true, marketStatus: "pending" }, "On LearnHNS Market as a pending listing"],
    [{ publish: true, marketStatus: "listed" }, /^On LearnHNS Market$/],
    [
      { publish: true, marketStatus: "replacedReuploaded" },
      "Replaced on the market by someone else's copy — re-uploaded",
    ],
    [
      {
        publish: true,
        marketStatus: "retrying",
        marketError: "no answer",
        marketRetryAt: "2026-10-12T10:00:00Z",
      },
      "Not on market — retrying",
    ],
    [
      { publish: true, marketStatus: "refused", marketError: "lockTime too far" },
      "LearnHNS Market refused the listing",
    ],
    [{ publish: true, marketStatus: "stepsUnverified" }, "does not verify over the lock coin"],
    [{ publish: true, marketStatus: "reported" }, "LearnHNS Market was told this listing ended"],
    [{ publish: false }, "Not published on LearnHNS Market — shared as a file only"],
  ] as const)("market status %j reads %s", async (over, text) => {
    setup([makeMyListing("listed", { listing: makeListing(over as never) })]);
    renderIt();
    expect(await screen.findByTestId("listing-market-status")).toHaveTextContent(text);
  });

  it("saves the stored listing file at the path chosen", async () => {
    saveMock.mockResolvedValue("/tmp/dexsale-listing.json");
    setup([makeMyListing("listed")]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-export"));
    await waitFor(() =>
      expect(calls("shakedex_save_listing_file")).toEqual([
        ["shakedex_save_listing_file", { listingId: "l1", path: "/tmp/dexsale-listing.json" }],
      ]),
    );
    expect(saveMock.mock.calls[0]![0]).toMatchObject({ defaultPath: "dexsale-listing.json" });
  });

  it("export never asks to unlock the signer: saving needs none", async () => {
    saveMock.mockResolvedValue("/tmp/x.json");
    setup([makeMyListing("listed")], {
      answers: { shakedex_save_listing_file: () => Promise.reject("Wallet locked") },
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-export"));
    expect(await screen.findByTestId("listing-error")).toBeInTheDocument();
    expect(calls("unlock_local_signer")).toHaveLength(0);
    expect(calls("shakedex_save_listing_file")).toHaveLength(1);
  });

  it("saves nothing when the save dialog is closed", async () => {
    saveMock.mockResolvedValue(null);
    setup([makeMyListing("listed")]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-export"));
    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    expect(calls("shakedex_save_listing_file")).toHaveLength(0);
  });

  it("shows a reverse auction's now, next and floor, and no mode for a restored lock", async () => {
    setup([
      makeMyListing("listed", {
        listing: makeListing({ mode: "reverseAuction" }),
        prices: {
          kind: "at",
          currentPrice: 5_000_000,
          nextPrice: 4_000_000,
          nextValidInSecs: 1800,
          floorPrice: 3_000_000,
        },
      }),
      makeMyListing("restored", {
        listing: makeListing({ id: "l2", name: "other", state: "restored", steps: [] }),
      }),
    ]);
    renderIt();
    const [auction, restored] = await screen.findAllByTestId("my-listing-row");
    expect(within(auction!).getByTestId("listing-prices")).toHaveTextContent(
      "Now 5.000000 HNS · next 4.000000 HNS in ~30 min · floor 3.000000 HNS",
    );
    expect(within(restored!).getByText("Locked — listing details unknown")).toBeInTheDocument();
    expect(within(restored!).queryByText(/Buy now|Reverse auction/)).not.toBeInTheDocument();
  });

  it("shows no current price on a row that is not Listed, whatever its prices say", async () => {
    setup([
      makeMyListing("cancelling", {
        prices: {
          kind: "at",
          currentPrice: 5_000_000,
          nextPrice: null,
          nextValidInSecs: null,
          floorPrice: 5_000_000,
        },
      }),
    ]);
    renderIt();
    const row = await screen.findByTestId("my-listing-row");
    expect(within(row).queryByTestId("listing-prices")).not.toBeInTheDocument();
    expect(within(row).queryByText(/5\.000000 HNS/)).not.toBeInTheDocument();
  });

  it("says why a corrupted row has no prices", async () => {
    setup([
      makeMyListing("listed", { prices: { kind: "unreadable", reason: "step 2 is not hex" } }),
    ]);
    renderIt();
    expect(await screen.findByTestId("listing-prices")).toHaveTextContent(
      "The stored prices cannot be read: step 2 is not hex",
    );
  });

  it("cancel says what the cancel cannot stop, then signs through the backend and sends it", async () => {
    setup([makeMyListing("listed", { listing: makeListing({ publish: true }) })]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-cancel"));
    expect(screen.getByText(CANCEL_STILL_BUYABLE)).toBeInTheDocument();
    expect(screen.getByText(CANCEL_MEMPOOL_PURCHASE)).toBeInTheDocument();
    expect(screen.getByText(CANCEL_THEN_FINALIZE)).toBeInTheDocument();
    expect(screen.getByText(CANCEL_MARKET_TOLD)).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("cancel-sign"));
    await waitFor(() =>
      expect(calls("broadcast_tx_draft")).toEqual([
        ["broadcast_tx_draft", { draftId: "cancel-1" }],
      ]),
    );
    expect(calls("shakedex_cancel_listing")[0]![1]).toEqual({ listingId: "l1", feeRate: null });
  });

  // `commands/shakedex.rs`'s `cheaper_ahead`, from the row's prices: a
  // cheaper step ahead of the current one, or, with none valid yet, any step
  // ahead of a reverse auction; nothing without the node's median time.
  it.each([
    ["a reverse auction with a cheaper step ahead", "reverseAuction", 5_000_000, 4_000_000, true],
    ["a reverse auction at its floor", "reverseAuction", 3_000_000, null, false],
    ["a reverse auction with no step valid yet", "reverseAuction", null, 5_000_000, true],
    ["a Buy Now lowered by a step not valid yet", "buyNow", 5_000_000, 4_000_000, true],
    ["a Buy Now whose one step is not valid yet", "buyNow", null, 5_000_000, false],
  ] as const)(
    "cancel of %s says the cheaper steps stay buyable only then",
    async (_, mode, now, next, falls) => {
      setup([
        makeMyListing("listed", {
          listing: makeListing({ mode }),
          prices: {
            kind: "at",
            currentPrice: now,
            nextPrice: next,
            nextValidInSecs: next == null ? null : 900,
            floorPrice: 3_000_000,
          },
        }),
      ]);
      renderIt();
      fireEvent.click(await screen.findByTestId("listing-cancel"));
      expect(screen.queryByText(CANCEL_STILL_BUYABLE_AS_IT_FALLS) !== null).toBe(falls);
      expect(screen.queryByText(CANCEL_STILL_BUYABLE) !== null).toBe(!falls);
    },
  );

  it("cancel without the node's median time says only the current price stays buyable, and an unpublished one says nothing of the market", async () => {
    setup([
      makeMyListing("listed", {
        listing: makeListing({
          mode: "reverseAuction",
          steps: [
            { price: 5_000_000, lockTime: 1, signature: "00" },
            { price: 4_000_000, lockTime: 2, signature: "00" },
          ],
        }),
        prices: { kind: "unverified" },
      }),
    ]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-cancel"));
    expect(screen.getByText(CANCEL_STILL_BUYABLE)).toBeInTheDocument();
    expect(screen.queryByText(CANCEL_MARKET_TOLD)).not.toBeInTheDocument();
  });

  it("cancel on a locked wallet unlocks this profile's signer once and signs", async () => {
    setup([makeMyListing("listed")], { unlocked: false });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-cancel"));
    fireEvent.click(screen.getByTestId("cancel-sign"));
    await waitFor(() =>
      expect(signingOrder()).toEqual([
        "shakedex_cancel_listing",
        "unlock_local_signer",
        "shakedex_cancel_listing",
        "broadcast_tx_draft",
      ]),
    );
    expect(calls("unlock_local_signer")[0]![1]).toEqual({ walletProfileId: "p1" });
  });

  it("a cancel refused at the gates says the signed cancel stays in My listings, and offers no second cancel", async () => {
    setup([makeMyListing("listed")], {
      answers: { broadcast_tx_draft: () => Promise.reject("node is not synced") },
      later: [makeMyListing("cancelling", { pendingSend: pending("cancel-1") })],
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-cancel"));
    fireEvent.click(screen.getByTestId("cancel-sign"));
    await waitFor(() => expect(screen.getByTestId("cancel-error")).toHaveTextContent(UNSENT_KEPT));
    expect(screen.getByTestId("cancel-error")).toHaveTextContent("node is not synced");
    expect(screen.queryByTestId("cancel-sign")).not.toBeInTheDocument();
    expect(calls("delete_tx_draft")).toHaveLength(0);
    expect(calls("shakedex_cancel_listing")).toHaveLength(1);
  });

  it("a cancel the node did not take says it can be sent again, never discarded", async () => {
    setup([makeMyListing("listed")], {
      answers: { broadcast_tx_draft: () => Promise.reject(NOT_TAKEN) },
      later: [
        makeMyListing("cancelling", { pendingSend: pending("cancel-1", { discardable: false }) }),
      ],
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-cancel"));
    fireEvent.click(screen.getByTestId("cancel-sign"));
    await waitFor(() =>
      expect(screen.getByTestId("cancel-error")).toHaveTextContent(SEND_AGAIN_KEPT),
    );
    expect(screen.getByTestId("cancel-error")).not.toHaveTextContent(UNSENT_KEPT);
  });

  it("a cancel the node refused says nothing kept when no row keeps it", async () => {
    setup([makeMyListing("listed")], {
      answers: { broadcast_tx_draft: () => Promise.reject("insufficient fee") },
      later: [makeMyListing("cancelling")],
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-cancel"));
    fireEvent.click(screen.getByTestId("cancel-sign"));
    const err = await screen.findByTestId("cancel-error");
    await waitFor(() => expect(calls("shakedex_list_listings").length).toBeGreaterThan(1));
    expect(err).toHaveTextContent("insufficient fee");
    expect(err).not.toHaveTextContent(/stays in My listings/);
  });

  it("refuses a price that is not below the current one before asking the backend", async () => {
    setup([
      makeMyListing("listed", {
        prices: {
          kind: "at",
          currentPrice: 5_000_000,
          nextPrice: null,
          nextValidInSecs: null,
          floorPrice: 5_000_000,
        },
      }),
    ]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-lower-price"));
    expect(screen.getByText(LOWER_PRICE_PERMANENCE)).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("lower-price-input"), { target: { value: "5" } });
    expect(screen.getByTestId("lower-price-submit")).toBeDisabled();
    expect(
      screen.getByText(
        "5.000000 HNS is not below the current price 5.000000 HNS: a price can only be lowered; raising it needs a cancel",
      ),
    ).toBeInTheDocument();
    expect(calls("shakedex_lower_price")).toHaveLength(0);
  });

  it("refuses a price the backend cannot read, in its words", async () => {
    setup([makeMyListing("listed")]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-lower-price"));
    fireEvent.change(screen.getByTestId("lower-price-input"), { target: { value: "4.0000001" } });
    expect(screen.getByTestId("lower-price-submit")).toBeDisabled();
    expect(screen.getByText('price "4.0000001": HNS has at most 6 decimals')).toBeInTheDocument();
  });

  it("lowers the price as typed through the backend", async () => {
    setup([
      makeMyListing("listed", {
        prices: {
          kind: "at",
          currentPrice: 5_000_000,
          nextPrice: null,
          nextValidInSecs: null,
          floorPrice: 5_000_000,
        },
      }),
    ]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-lower-price"));
    fireEvent.change(screen.getByTestId("lower-price-input"), { target: { value: "4.5" } });
    fireEvent.click(screen.getByTestId("lower-price-submit"));
    await waitFor(() =>
      expect(calls("shakedex_lower_price")).toEqual([
        ["shakedex_lower_price", { listingId: "l1", price: "4.5" }],
      ]),
    );
  });

  it("disables every signing action for a Ledger profile with the R16 sentence; export stays", async () => {
    setup([makeMyListing("listed")], { kind: "ledger_hardware" });
    renderIt();
    expect(await screen.findByTestId("listing-cancel")).toBeDisabled();
    expect(screen.getByTestId("listing-lower-price")).toBeDisabled();
    expect(screen.getAllByText(RECOVERY_PHRASE_ONLY).length).toBeGreaterThan(0);
    expect(screen.getByTestId("listing-export")).toBeEnabled();
  });

  it("disables the signing actions with the node sentence when the node cannot send", async () => {
    setup([makeMyListing("listed")], { canSend: false });
    renderIt();
    expect(await screen.findByTestId("listing-cancel")).toBeDisabled();
    expect(screen.getAllByText(NEEDS_SENDING_NODE).length).toBeGreaterThan(0);
  });

  it("offers Publish only for an unpublished listing on mainnet, and sends it", async () => {
    setup([makeMyListing("listed")], { network: "mainnet" });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-publish"));
    expect(screen.getByText(PUBLISH_PRIVACY)).toBeInTheDocument();
    await waitFor(() =>
      expect(calls("shakedex_publish_listing")).toEqual([
        ["shakedex_publish_listing", { listingId: "l1" }],
      ]),
    );
  });

  it("offers no Publish off mainnet", async () => {
    setup([makeMyListing("listed")], { network: "regtest" });
    renderIt();
    await screen.findByTestId("listing-export");
    expect(screen.queryByTestId("listing-publish")).not.toBeInTheDocument();
    expect(screen.queryByText(PUBLISH_PRIVACY)).not.toBeInTheDocument();
  });

  it("offers no Publish for a listing already published", async () => {
    setup([makeMyListing("listed", { listing: makeListing({ publish: true }) })]);
    renderIt();
    await screen.findByTestId("listing-export");
    expect(screen.queryByTestId("listing-publish")).not.toBeInTheDocument();
  });

  it("disables Publish with the profile's gate before the row's own answer", async () => {
    loadSettings({ shakedex_experimental: "false" });
    setup([makeMyListing("listed")]);
    renderIt();
    expect(await screen.findByTestId("listing-publish")).toBeDisabled();
    expect(screen.getByText(MAINNET_SELLING_EXPERIMENTAL)).toBeInTheDocument();
  });

  it("disables Publish with the backend's reason when the row refuses it", async () => {
    setup([
      makeMyListing("listed", {
        actions: makeActions({
          publish: { allowed: false, reason: "this listing file names a market fee" },
        }),
      }),
    ]);
    renderIt();
    expect(await screen.findByTestId("listing-publish")).toBeDisabled();
    expect(screen.getByText("this listing file names a market fee")).toBeInTheDocument();
  });

  it("publish on a locked wallet unlocks this profile's signer once and publishes", async () => {
    setup([makeMyListing("listed")], { unlocked: false });
    renderIt();
    const publish = await screen.findByTestId("listing-publish");
    await waitFor(() => expect(publish).toBeEnabled());
    fireEvent.click(publish);
    await waitFor(() =>
      expect(signingOrder()).toEqual([
        "shakedex_publish_listing",
        "unlock_local_signer",
        "shakedex_publish_listing",
      ]),
    );
  });

  it("sends the listing's signed, unsent draft from its row", async () => {
    setup([
      makeMyListing("finalizing", {
        pendingSend: pending("fin-1", { action: "shakedex_lock_finalize" }),
      }),
    ]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-send"));
    await waitFor(() =>
      expect(calls("broadcast_tx_draft")).toEqual([["broadcast_tx_draft", { draftId: "fin-1" }]]),
    );
    expect(calls("sign_tx_draft")).toHaveLength(0);
    // The row follows the send.
    await waitFor(() => expect(calls("shakedex_list_listings").length).toBeGreaterThan(1));
  });

  it("a Send the node did not take says so, with the stage, and that it can be sent again", async () => {
    const row = makeMyListing("finalizing", {
      pendingSend: pending("fin-1", { action: "shakedex_lock_finalize" }),
    });
    setup([row], {
      answers: { broadcast_tx_draft: () => Promise.reject(NOT_TAKEN) },
      later: [{ ...row, pendingSend: pending("fin-1", { discardable: false }) }],
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-send"));
    await waitFor(() =>
      expect(screen.getByTestId("listing-error")).toHaveTextContent(SEND_AGAIN_KEPT),
    );
    expect(screen.getByTestId("listing-error")).toHaveTextContent(
      "Broadcast failed: The node did not take the transaction.",
    );
  });

  it("a Send whose sign fails names the stage and keeps the draft", async () => {
    setup(
      [
        makeMyListing("cancelling", {
          pendingSend: pending("c-1", { action: "shakedex_cancel", signed: false }),
        }),
      ],
      { answers: { sign_tx_draft: () => Promise.reject("the secure window was closed") } },
    );
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-send"));
    expect(await screen.findByTestId("listing-error")).toHaveTextContent(
      "Sign failed: the secure window was closed",
    );
    expect(calls("delete_tx_draft")).toHaveLength(0);
  });

  it("a row whose send the node may hold offers Send only: it cannot be discarded", async () => {
    setup([makeMyListing("locking", { pendingSend: pending("lock-1", { discardable: false }) })]);
    renderIt();
    expect(await screen.findByTestId("listing-send")).toBeEnabled();
    expect(screen.queryByTestId("listing-discard")).not.toBeInTheDocument();
  });

  it("signs and sends an unsigned draft, unlocking this profile's signer once", async () => {
    setup(
      [
        makeMyListing("cancelling", {
          pendingSend: pending("c-1", { action: "shakedex_cancel", signed: false }),
        }),
      ],
      { unlocked: false },
    );
    renderIt();
    const send = await screen.findByTestId("listing-send");
    fireEvent.click(send);
    await waitFor(() =>
      expect(signingOrder()).toEqual([
        "sign_tx_draft",
        "unlock_local_signer",
        "sign_tx_draft",
        "broadcast_tx_draft",
      ]),
    );
  });

  it("a Locking row with an unsent lock offers Send and Discard; Discard drops it", async () => {
    setup([
      makeMyListing("locking", {
        pendingSend: pending("lock-1"),
      }),
    ]);
    renderIt();
    expect(await screen.findByTestId("listing-send")).toBeEnabled();
    fireEvent.click(screen.getByTestId("listing-discard"));
    await waitFor(() =>
      expect(calls("delete_tx_draft")).toEqual([["delete_tx_draft", { draftId: "lock-1" }]]),
    );
    expect(calls("broadcast_tx_draft")).toHaveLength(0);
    // The row follows the discard.
    await waitFor(() => expect(calls("shakedex_list_listings").length).toBeGreaterThan(1));
  });

  it.each([
    ["locking", "Lock discarded. Nothing was sent: the name stays yours."],
    ["finalizing", "Finalize discarded. The listing is back to Ready to Finalize & sign."],
    ["cancelling", "Cancel discarded. The listing is back as it was before the cancel."],
    [
      "cancelFinalizing",
      "Finalize discarded. The listing is back to Cancelled · finalize to bring it home.",
    ],
  ] as const)("discarding a %s row's draft says where the listing went", async (state, text) => {
    setup([makeMyListing(state, { pendingSend: pending("d-1") })]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-discard"));
    await waitFor(() => expect(toasts()).toEqual([text]));
  });

  it("offers no Send or Discard when nothing is left to send", async () => {
    setup([makeMyListing("locking")]);
    renderIt();
    await screen.findByTestId("listing-finalize-and-sign");
    expect(screen.queryByTestId("listing-send")).not.toBeInTheDocument();
    expect(screen.queryByTestId("listing-discard")).not.toBeInTheDocument();
  });

  it("finalizes a mined cancel home: builds, signs and sends it", async () => {
    setup([makeMyListing("cancelAwaitingFinalize")]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-finalize-cancel"));
    await waitFor(() =>
      expect(signingOrder()).toEqual([
        "shakedex_finalize_cancel",
        "sign_tx_draft",
        "broadcast_tx_draft",
      ]),
    );
    expect(calls("shakedex_finalize_cancel")[0]![1]).toEqual({ listingId: "l1", feeRate: null });
    expect(calls("broadcast_tx_draft")[0]![1]).toEqual({ draftId: "home-1" });
  });

  it("a cancel's finalize that fails to sign shows why and drops its unsent draft", async () => {
    setup([makeMyListing("cancelAwaitingFinalize")], {
      answers: { sign_tx_draft: () => Promise.reject("the secure window was closed") },
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-finalize-cancel"));
    expect(await screen.findByTestId("listing-error")).toHaveTextContent(
      "the secure window was closed",
    );
    expect(calls("delete_tx_draft")).toEqual([["delete_tx_draft", { draftId: "home-1" }]]);
  });

  it("a cancel's finalize whose broadcast fails keeps its draft and says where it is", async () => {
    setup([makeMyListing("cancelAwaitingFinalize")], {
      answers: { broadcast_tx_draft: () => Promise.reject("node is not synced") },
      later: [makeMyListing("cancelFinalizing", { pendingSend: pending("home-1") })],
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-finalize-cancel"));
    await waitFor(() => expect(screen.getByTestId("listing-error")).toHaveTextContent(UNSENT_KEPT));
    expect(calls("delete_tx_draft")).toHaveLength(0);
  });

  it("a cancel's finalize whose broadcast fails and is kept nowhere says only why", async () => {
    setup([makeMyListing("cancelAwaitingFinalize")], {
      answers: { broadcast_tx_draft: () => Promise.reject("insufficient fee") },
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-finalize-cancel"));
    const err = await screen.findByTestId("listing-error");
    await waitFor(() => expect(calls("shakedex_list_listings").length).toBeGreaterThan(1));
    expect(err).toHaveTextContent("Broadcast failed: insufficient fee");
    expect(err).not.toHaveTextContent(/stays in My listings/);
  });

  it("Finalize & sign opens its form for the row's listing", async () => {
    setup([
      makeMyListing("readyToFinalize", {
        listing: makeListing({ state: "readyToFinalize", steps: [] }),
      }),
    ]);
    renderIt();
    const button = await screen.findByTestId("listing-finalize-and-sign");
    // The label the Lock dialog's toast names (`LISTING_BUTTON_LABEL`).
    expect(button).toHaveTextContent(/^Finalize & sign$/);
    fireEvent.click(button);
    expect(await screen.findByText("Finalize & sign .dexsale")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("fas-price"), { target: { value: "5" } });
    const submit = screen.getByTestId("fas-submit");
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() => expect(calls("shakedex_finalize_and_sign")).toHaveLength(1));
    expect(calls("shakedex_finalize_and_sign")[0]![1]).toMatchObject({ listingId: "l1" });
  });

  it("Import its listing file opens its dialog for the restored lock", async () => {
    setup([makeMyListing("restored", { listing: makeListing({ state: "restored", steps: [] }) })]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-import-file"));
    expect(await screen.findByText("Import the listing file of .dexsale")).toBeInTheDocument();
    expect(screen.getByTestId("own-file-text")).toBeInTheDocument();
  });

  it("disables Import its listing file with the backend's reason when the row refuses it", async () => {
    const why =
      "only a lock restored by name, or a listing managed from another device, takes its saved listing file";
    setup([
      makeMyListing("restored", {
        actions: makeActions({ importFile: { allowed: false, reason: why } }),
      }),
    ]);
    renderIt();
    expect(await screen.findByTestId("listing-import-file")).toBeDisabled();
    expect(screen.getByText(why)).toBeInTheDocument();
  });

  it("an unreadable Restored lock still offers its file import, as the import's own refusal allows", async () => {
    const corrupted = "corrupted listing l1: unreadable steps";
    const refused = { allowed: false, reason: corrupted };
    setup([
      makeMyListing("restored", {
        listing: makeListing({ state: "restored", steps: [] }),
        prices: { kind: "unreadable", reason: corrupted },
        actions: {
          finalizeAndSign: refused,
          lowerPrice: refused,
          cancel: refused,
          finalizeCancel: refused,
          export: refused,
          publish: refused,
          importFile: { allowed: true, reason: null },
          manageHere: refused,
        },
      }),
    ]);
    renderIt();
    expect(await screen.findByTestId("listing-cancel")).toBeDisabled();
    expect(screen.getByTestId("listing-import-file")).toBeEnabled();
  });

  it("restore by name shows the lock as 'Locked — listing details unknown'", async () => {
    let restored = false;
    const restoredRow = makeMyListing("restored", {
      listing: makeListing({ state: "restored", steps: [] }),
    });
    setup([], {
      answers: {
        shakedex_restore_lock: () => {
          restored = true;
          return Promise.resolve(restoredRow.listing);
        },
        shakedex_list_listings: () => Promise.resolve(restored ? [restoredRow] : []),
      },
    });
    renderIt();
    expect(await screen.findByText("Restore a lock by name")).toBeInTheDocument();
    const submit = screen.getByTestId("restore-submit");
    expect(submit).toBeDisabled();
    fireEvent.change(screen.getByTestId("restore-name"), { target: { value: " dexsale " } });
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    expect(await screen.findByText(RESTORED_LABEL)).toBeInTheDocument();
    expect(calls("shakedex_restore_lock")).toEqual([
      ["shakedex_restore_lock", { name: "dexsale" }],
    ]);
    expect(toasts()).toContain(`Restored .dexsale: ${RESTORED_LABEL}`);
  });

  it("Find my locks runs a forced search and says how many it restored", async () => {
    setup([], {
      answers: {
        shakedex_find_locks: () =>
          Promise.resolve({ ran: true, adopted: ["a", "b"], done: true, complete: true }),
      },
    });
    renderIt();
    const button = await screen.findByRole("button", { name: "Find my locks" });
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    await waitFor(() =>
      expect(toasts()).toContain("Found 2 names locked for sale · restored in My listings"),
    );
    expect(calls("shakedex_find_locks")).toEqual([["shakedex_find_locks", { force: true }]]);
  });

  it("Find my locks says when it found none, and a single name in the singular", async () => {
    let report = { ran: true, adopted: [] as string[], done: true, complete: true };
    setup([], { answers: { shakedex_find_locks: () => Promise.resolve(report) } });
    renderIt();
    const button = await screen.findByRole("button", { name: "Find my locks" });
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    await waitFor(() => expect(toasts()).toContain("No locked names found"));
    report = { ran: true, adopted: ["a"], done: true, complete: true };
    fireEvent.click(button);
    await waitFor(() =>
      expect(toasts()).toContain("Found 1 name locked for sale · restored in My listings"),
    );
  });

  it("Find my locks does not say none were found after a run that could not read everything", async () => {
    // `done` is stamped by an earlier complete run; this one read partially.
    setup([], {
      answers: {
        shakedex_find_locks: () =>
          Promise.resolve({ ran: true, adopted: [], done: true, complete: false }),
      },
    });
    renderIt();
    const button = await screen.findByRole("button", { name: "Find my locks" });
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    await waitFor(() =>
      expect(toasts()).toContain(
        "Some addresses or names could not be read from your node. Try again later.",
      ),
    );
    expect(toasts()).not.toContain("No locked names found");
  });

  it("shows the unlock line only while candidates wait and the search is not done", async () => {
    const UNLOCK_LINE =
      "Names this recovery phrase may have locked for sale were found. Unlock the wallet to restore them.";
    let status = { done: false, addressIndex: true, candidates: 2 };
    setup([], {
      unlocked: false,
      answers: { shakedex_lock_search_status: () => Promise.resolve(status) },
    });
    renderIt();
    expect(await screen.findByText(UNLOCK_LINE)).toBeInTheDocument();
    expect(screen.getByTestId("unlock-now")).toBeInTheDocument();
    cleanup();

    status = { done: false, addressIndex: true, candidates: 0 };
    setup([], {
      unlocked: false,
      answers: { shakedex_lock_search_status: () => Promise.resolve(status) },
    });
    renderIt();
    await screen.findByText("Restore a lock by name");
    await waitFor(() => expect(calls("shakedex_lock_search_status").length).toBeGreaterThan(0));
    expect(screen.queryByText(UNLOCK_LINE)).not.toBeInTheDocument();
    cleanup();

    status = { done: true, addressIndex: true, candidates: 2 };
    setup([], {
      unlocked: false,
      answers: { shakedex_lock_search_status: () => Promise.resolve(status) },
    });
    renderIt();
    await screen.findByText("Restore a lock by name");
    await waitFor(() => expect(calls("shakedex_lock_search_status").length).toBeGreaterThan(0));
    expect(screen.queryByText(UNLOCK_LINE)).not.toBeInTheDocument();
    cleanup();

    // Unlocked: the search runs by itself, no line.
    status = { done: false, addressIndex: true, candidates: 2 };
    setup([], {
      unlocked: true,
      answers: { shakedex_lock_search_status: () => Promise.resolve(status) },
    });
    renderIt();
    await screen.findByText("Restore a lock by name");
    await waitFor(() => expect(calls("shakedex_lock_search_status").length).toBeGreaterThan(0));
    expect(screen.queryByText(UNLOCK_LINE)).not.toBeInTheDocument();
  });

  it("renders nothing for the search when its status cannot be read", async () => {
    setup([], { answers: { shakedex_lock_search_status: () => Promise.reject("node down") } });
    renderIt();
    expect(await screen.findByText("Restore a lock by name")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Find my locks" })).toBeInTheDocument();
    expect(screen.queryByText(/node down/)).not.toBeInTheDocument();
  });

  it("says the search needs the address index", async () => {
    const INDEX_LINE =
      "Finding your locks needs a node run with --index-tx and --index-address; Restore a lock by name works without them.";
    setup([], {
      answers: {
        shakedex_lock_search_status: () =>
          Promise.resolve({ done: false, addressIndex: false, candidates: 0 }),
      },
    });
    renderIt();
    expect(await screen.findByText(INDEX_LINE)).toBeInTheDocument();
    cleanup();

    // The forced search's own refusal, as Tauri rejects it: a plain string.
    setup([], {
      answers: {
        shakedex_find_locks: () =>
          Promise.reject(
            "Invalid input: finding your locks needs a node run with --index-tx and --index-address; Restore a lock by name works without them",
          ),
      },
    });
    renderIt();
    const button = await screen.findByRole("button", { name: "Find my locks" });
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
    expect(await screen.findByText(INDEX_LINE)).toBeInTheDocument();
  });

  it.each([
    [" .DexSale ", "dexsale"],
    ["сбер", "xn--90ai7ab"],
  ])("restore sends %j as the on-chain name %s", async (typed, sent) => {
    setup([], {
      answers: {
        shakedex_restore_lock: () => Promise.resolve(makeListing({ state: "restored", steps: [] })),
      },
    });
    renderIt();
    fireEvent.change(await screen.findByTestId("restore-name"), { target: { value: typed } });
    const submit = screen.getByTestId("restore-submit");
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() =>
      expect(calls("shakedex_restore_lock")).toEqual([["shakedex_restore_lock", { name: sent }]]),
    );
  });

  it("restore is disabled while the typed name is no name at all", async () => {
    setup([]);
    renderIt();
    fireEvent.change(await screen.findByTestId("restore-name"), { target: { value: " ... " } });
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.getByTestId("restore-submit")).toBeDisabled();
  });

  it("shows the name it restores when that is not the name as typed", async () => {
    setup([]);
    renderIt();
    const field = await screen.findByTestId("restore-name");
    fireEvent.change(field, { target: { value: "dex sale" } });
    expect(await screen.findByTestId("restore-target")).toHaveTextContent("Restores .dexsale");
    // Case, a leading dot and the Unicode form of an IDN are the name as typed.
    for (const same of [" .DexSale ", "сбер"]) {
      fireEvent.change(field, { target: { value: same } });
      await waitFor(() => expect(screen.queryByTestId("restore-target")).toBeNull());
    }
  });

  it("restore says why the backend refused it", async () => {
    setup([], {
      answers: {
        shakedex_restore_lock: () =>
          Promise.reject("Invalid input: 'dexsale' is not at this wallet's lock address"),
      },
    });
    renderIt();
    fireEvent.change(await screen.findByTestId("restore-name"), { target: { value: "dexsale" } });
    const submit = screen.getByTestId("restore-submit");
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "'dexsale' is not at this wallet's lock address",
    );
  });

  it("restore on a locked wallet unlocks this profile's signer once and restores", async () => {
    setup([], {
      unlocked: false,
      answers: {},
    });
    const base = invokeMock.getMockImplementation()!;
    let unlocked = false;
    invokeMock.mockImplementation((cmd: string, args: unknown) => {
      if (cmd === "unlock_local_signer") unlocked = true;
      if (cmd === "shakedex_restore_lock")
        return unlocked
          ? Promise.resolve(makeListing({ state: "restored", steps: [] }))
          : Promise.reject("Wallet locked");
      return base(cmd, args);
    });
    renderIt();
    fireEvent.change(await screen.findByTestId("restore-name"), { target: { value: "dexsale" } });
    const submit = screen.getByTestId("restore-submit");
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() => expect(calls("shakedex_restore_lock")).toHaveLength(2));
    expect(calls("unlock_local_signer")).toEqual([
      ["unlock_local_signer", { walletProfileId: "p1" }],
    ]);
  });

  it("restore is disabled for a Ledger profile with the R16 sentence", async () => {
    setup([], { kind: "ledger_hardware" });
    renderIt();
    fireEvent.change(await screen.findByTestId("restore-name"), { target: { value: "dexsale" } });
    expect(screen.getByTestId("restore-submit")).toBeDisabled();
    expect(screen.getAllByText(RECOVERY_PHRASE_ONLY).length).toBeGreaterThan(0);
  });

  it("imports a restored lock's own file, pasted or chosen on disk", async () => {
    openMock.mockResolvedValue("/tmp/f.json");
    const listed = makeListing({ state: "listed" });
    setup([makeMyListing("restored", { listing: makeListing({ state: "restored", steps: [] }) })], {
      answers: {
        shakedex_import_own_listing_file: () => Promise.resolve(listed),
        shakedex_import_own_listing_from_file: () => Promise.resolve(listed),
      },
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-import-file"));
    const importButton = await screen.findByTestId("own-file-import");
    expect(importButton).toBeDisabled();
    fireEvent.change(screen.getByTestId("own-file-text"), { target: { value: '{"version":2}' } });
    fireEvent.click(importButton);
    await waitFor(() =>
      expect(calls("shakedex_import_own_listing_file")).toEqual([
        ["shakedex_import_own_listing_file", { text: '{"version":2}' }],
      ]),
    );
    expect(toasts()).toContain("Listing file imported: the listing is Listed for sale.");
    await waitFor(() => expect(screen.queryByTestId("own-file-text")).not.toBeInTheDocument());

    fireEvent.click(screen.getByTestId("listing-import-file"));
    fireEvent.click(await screen.findByTestId("own-file-pick"));
    await waitFor(() =>
      expect(calls("shakedex_import_own_listing_from_file")).toEqual([
        ["shakedex_import_own_listing_from_file", { path: "/tmp/f.json" }],
      ]),
    );
    expect(openMock.mock.calls[0]![0]).toMatchObject({
      filters: [{ name: "Listing file", extensions: ["json"] }],
    });
  });

  it("says why the file picker could not open", async () => {
    openMock.mockRejectedValue("dialog plugin unavailable");
    setup([makeMyListing("restored", { listing: makeListing({ state: "restored", steps: [] }) })]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-import-file"));
    fireEvent.click(await screen.findByTestId("own-file-pick"));
    expect(await screen.findByTestId("own-file-error")).toHaveTextContent(
      "dialog plugin unavailable",
    );
    expect(calls("shakedex_import_own_listing_from_file")).toHaveLength(0);
  });

  it("the own-file import is disabled with the node sentence when the node cannot send", async () => {
    // The row's button is gated too; the dialog says why on its own once open.
    setup([makeMyListing("restored", { listing: makeListing({ state: "restored", steps: [] }) })]);
    const { OwnFileImport } = await import("../market/RestoreLock");
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([makeProfile()]);
      if (cmd === "get_write_capability")
        return Promise.resolve({
          signerUnlocked: true,
          broadcasterAvailable: false,
          canWrite: false,
          reason: null,
        });
      return Promise.resolve(null);
    });
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={qc}>
        <OwnFileImport listing={makeListing({ state: "restored", steps: [] })} onClose={vi.fn()} />
      </QueryClientProvider>,
    );
    expect(await screen.findByText(NEEDS_SENDING_NODE)).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("own-file-text"), { target: { value: "{}" } });
    expect(screen.getByTestId("own-file-import")).toBeDisabled();
    expect(screen.getByTestId("own-file-pick")).toBeDisabled();
  });

  it("the own-file import on a locked wallet unlocks this profile's signer once and imports", async () => {
    setup([makeMyListing("restored", { listing: makeListing({ state: "restored", steps: [] }) })], {
      unlocked: false,
    });
    const base = invokeMock.getMockImplementation()!;
    let unlocked = false;
    invokeMock.mockImplementation((cmd: string, args: unknown) => {
      if (cmd === "unlock_local_signer") unlocked = true;
      if (cmd === "shakedex_import_own_listing_file")
        return unlocked ? Promise.resolve(makeListing()) : Promise.reject("Wallet locked");
      return base(cmd, args);
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-import-file"));
    fireEvent.change(await screen.findByTestId("own-file-text"), { target: { value: "{}" } });
    fireEvent.click(screen.getByTestId("own-file-import"));
    await waitFor(() => expect(calls("shakedex_import_own_listing_file")).toHaveLength(2));
    expect(calls("unlock_local_signer")).toEqual([
      ["unlock_local_signer", { walletProfileId: "p1" }],
    ]);
  });

  it("imports nothing when the open dialog is closed, and shows the backend's refusal", async () => {
    openMock.mockResolvedValue(null);
    setup([makeMyListing("restored", { listing: makeListing({ state: "restored", steps: [] }) })], {
      answers: {
        shakedex_import_own_listing_file: () =>
          Promise.reject("Invalid input: this listing file is for another lock coin"),
      },
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-import-file"));
    fireEvent.click(await screen.findByTestId("own-file-pick"));
    await waitFor(() => expect(openMock).toHaveBeenCalled());
    expect(calls("shakedex_import_own_listing_from_file")).toHaveLength(0);
    fireEvent.change(screen.getByTestId("own-file-text"), { target: { value: "{}" } });
    fireEvent.click(screen.getByTestId("own-file-import"));
    expect(await screen.findByTestId("own-file-error")).toHaveTextContent(
      "this listing file is for another lock coin",
    );
  });
});

describe("MyListings: a listing another device manages (R40–R45)", () => {
  /** A Listed, published row another device manages, its price there 3 HNS. */
  const elsewhereRow = (over: Partial<ShakedexMyListing> = {}) =>
    makeMyListing("listed", {
      listing: makeListing({ state: "listed", publish: true, marketStatus: "listed" }),
      prices: {
        kind: "at",
        currentPrice: 5,
        nextPrice: null,
        nextValidInSecs: null,
        floorPrice: 5,
      },
      actions: makeActions({
        lowerPrice: { allowed: false, reason: LOWER_MANAGED_ELSEWHERE },
      }),
      managedElsewhere: true,
      foreignStepPriceDoos: 3,
      ...over,
    });

  it("says so in place of the market status, with the other device's price, and labels this device's own", async () => {
    setup([elsewhereRow()]);
    renderIt();
    expect(await screen.findByTestId("listing-market-status")).toHaveTextContent(MANAGED_ELSEWHERE);
    expect(screen.getByTestId("listing-elsewhere-price")).toHaveTextContent(
      managedElsewherePriceText(3),
    );
    expect(screen.getByTestId("listing-prices")).toHaveTextContent(`— ${ON_THIS_DEVICE}`);
    expect(screen.getByText(EXPORT_THIS_DEVICE_ONLY)).toBeInTheDocument();
    // Lower price is refused in the backend's words; Cancel and Export stay.
    expect(screen.getByTestId("listing-lower-price")).toBeDisabled();
    expect(screen.getByText(LOWER_MANAGED_ELSEWHERE)).toBeInTheDocument();
    expect(screen.getByTestId("listing-cancel")).toBeEnabled();
    expect(screen.getByTestId("listing-export")).toBeEnabled();
    expect(screen.getByTestId("listing-manage-here")).toHaveTextContent(MANAGE_HERE_LABEL);
    expect(screen.getByTestId("listing-import-file")).toBeEnabled();
  });

  it("offers neither the take-over nor the import on a listing this device manages", async () => {
    setup([makeMyListing("listed")]);
    renderIt();
    await screen.findByTestId("listing-market-status");
    expect(screen.queryByTestId("listing-manage-here")).toBeNull();
    expect(screen.queryByTestId("listing-import-file")).toBeNull();
    expect(screen.queryByTestId("listing-elsewhere-price")).toBeNull();
  });

  it("asks before taking over, then calls the backend once with the listing", async () => {
    setup([elsewhereRow()], {
      answers: {
        shakedex_manage_listing_here: () => Promise.resolve(makeListing({ state: "listed" })),
      },
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-manage-here"));
    expect(screen.getByTestId("manage-here-text")).toHaveTextContent(MANAGE_HERE_TEXT(3));
    expect(calls("shakedex_manage_listing_here")).toHaveLength(0);
    fireEvent.click(screen.getByTestId("manage-here-confirm"));
    await waitFor(() =>
      expect(calls("shakedex_manage_listing_here")).toEqual([
        ["shakedex_manage_listing_here", { listingId: "l1" }],
      ]),
    );
    await waitFor(() => expect(toasts().some((t) => t.includes("manages the listing"))).toBe(true));
  });

  it("shows the backend's refusal of the take-over in its words", async () => {
    setup([elsewhereRow()], {
      answers: {
        shakedex_manage_listing_here: () =>
          Promise.reject(`Invalid input: ${TAKE_OVER_OTHER_ADDRESS}`),
      },
    });
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-manage-here"));
    fireEvent.click(screen.getByTestId("manage-here-confirm"));
    expect(await screen.findByTestId("manage-here-error")).toHaveTextContent(
      TAKE_OVER_OTHER_ADDRESS,
    );
  });

  it("the cancel dialog names the other device's price as buyable until the cancel is mined", async () => {
    setup([elsewhereRow()]);
    renderIt();
    fireEvent.click(await screen.findByTestId("listing-cancel"));
    expect(await screen.findByTestId("cancel-elsewhere")).toHaveTextContent(cancelElsewhereText(3));
  });

  it("words a sale at another device's price and a cancel from another device (R43)", async () => {
    setup([
      makeMyListing("sold", {
        listing: makeListing({ id: "s1", state: "sold" }),
        soldElsewhere: true,
      }),
      makeMyListing("cancelled", {
        listing: makeListing({ id: "c1", state: "cancelled" }),
        cancelledElsewhere: true,
      }),
      makeMyListing("sold", { listing: makeListing({ id: "s2", state: "sold" }) }),
    ]);
    renderIt();
    const lines = await screen.findAllByTestId("listing-from-elsewhere");
    expect(lines.map((l) => l.textContent)).toEqual([SOLD_ELSEWHERE, CANCELLED_ELSEWHERE]);
  });
});
