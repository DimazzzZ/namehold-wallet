import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { FinalizeAndSign } from "../market/FinalizeAndSign";
import {
  RECOVERY_PHRASE_ONLY,
  REVERSE_NEEDS_SCHEDULE,
  SCHEDULE_PAST_A_YEAR,
  STEP_SIGNATURE_PERMANENCE,
  UNSENT_KEPT,
} from "../market/marketText";
import { makeListing, makeMyListing } from "../../test/fixtures/listings";
import { loadSettings } from "../../test/fixtures/settings";
import { makeProfile, makeSession } from "../../test/fixtures/wallet";
import { useUiStore } from "../../stores/ui";
import type { ShakedexListingMode, ShakedexMyListing, WalletProfileKind } from "../../types";

interface Opts {
  unlocked?: boolean;
  kind?: WalletProfileKind;
  finalize?: () => Promise<unknown>;
  broadcast?: () => Promise<unknown>;
  /** My listings as the backend gives it after a failed broadcast (default: no rows). */
  listings?: ShakedexMyListing[];
}

function setup(o: Opts = {}) {
  // The signer as the backend holds it: Finalize & sign refuses "Wallet
  // locked" (`authorize_signer`) until the profile's signer is unlocked.
  let unlocked = o.unlocked ?? true;
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([
          makeProfile({ kind: o.kind ?? "mnemonic_hot", network: "regtest" }),
        ]);
      case "get_signer_session":
        return Promise.resolve(makeSession({ unlocked }));
      case "unlock_local_signer":
        unlocked = true;
        return Promise.resolve(null);
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: unlocked,
          broadcasterAvailable: true,
          canWrite: unlocked,
          reason: null,
        });
      case "shakedex_finalize_and_sign":
        if (!unlocked) return Promise.reject("Wallet locked");
        return o.finalize
          ? o.finalize()
          : Promise.resolve(makeListing({ state: "finalizing", finalizeDraftId: "fin-1" }));
      case "broadcast_tx_draft":
        return o.broadcast
          ? o.broadcast()
          : Promise.resolve({ draftId: "fin-1", txid: "ff", status: "broadcasted" });
      case "shakedex_list_listings":
        return Promise.resolve(o.listings ?? []);
      default:
        return Promise.resolve(null);
    }
  });
}

function renderIt(mode: ShakedexListingMode, onClose = vi.fn()) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <FinalizeAndSign
      open
      listing={makeListing({ state: "readyToFinalize", mode, steps: [] })}
      onClose={onClose}
    />,
    {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    },
  );
  return onClose;
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);
const toasts = () => useUiStore.getState().toastQueue.map((t) => t.message);

/** The submit button (disabled until the profile and the node are known). */
const submitButton = () => screen.findByTestId("fas-submit");

/** A complete schedule, typed field by field. */
async function typeSchedule(start: string, floor: string, steps: string, intervalMin: string) {
  fireEvent.change(await screen.findByTestId("fas-start"), { target: { value: start } });
  fireEvent.change(screen.getByTestId("fas-floor"), { target: { value: floor } });
  fireEvent.change(screen.getByTestId("fas-steps"), { target: { value: steps } });
  fireEvent.change(screen.getByTestId("fas-interval"), { target: { value: intervalMin } });
}

beforeEach(() => {
  loadSettings();
  useUiStore.setState({ toastQueue: [], toastMessage: null });
});

describe("FinalizeAndSign", () => {
  it("a Buy Now sends its one price and no schedule, then the signed FINALIZE", async () => {
    setup();
    const onClose = renderIt("buyNow");
    expect(screen.getByText(STEP_SIGNATURE_PERMANENCE)).toBeInTheDocument();
    expect(
      screen.getByText(
        "The secure window lists the finalize fee and every price, with when it becomes valid, before anything is signed.",
      ),
    ).toBeInTheDocument();
    // A Buy Now has one price: no schedule fields.
    expect(screen.queryByTestId("fas-start")).not.toBeInTheDocument();
    fireEvent.change(await screen.findByTestId("fas-price"), { target: { value: "5" } });
    const submit = await submitButton();
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(calls("shakedex_finalize_and_sign")[0]![1]).toEqual({
      listingId: "l1",
      prices: [{ price: "5" }],
      schedule: null,
      feeRate: null,
    });
    expect(calls("broadcast_tx_draft")).toEqual([["broadcast_tx_draft", { draftId: "fin-1" }]]);
    expect(toasts()).toContain(
      "Finalize sent. The listing becomes Listed for sale once the finalize is mined.",
    );
  });

  it("refuses a Buy Now price the backend refuses, in its words, before asking", async () => {
    setup();
    renderIt("buyNow");
    fireEvent.change(await screen.findByTestId("fas-price"), { target: { value: "0.0001" } });
    expect(
      screen.getByText('price "0.0001": below the dust limit of 0.001000 HNS'),
    ).toBeInTheDocument();
    expect(screen.getByTestId("fas-submit")).toBeDisabled();
  });

  it("a reverse auction needs its whole schedule (REVERSE_NEEDS_SCHEDULE), then sends only the schedule", async () => {
    setup();
    renderIt("reverseAuction");
    // A reverse auction has a schedule: no one price.
    expect(screen.queryByTestId("fas-price")).not.toBeInTheDocument();
    fireEvent.change(await screen.findByTestId("fas-start"), { target: { value: "10" } });
    expect(screen.getByTestId("fas-submit")).toBeDisabled();
    expect(screen.getByText(REVERSE_NEEDS_SCHEDULE)).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("fas-floor"), { target: { value: "5" } });
    fireEvent.change(screen.getByTestId("fas-steps"), { target: { value: "6" } });
    expect(screen.getByText(REVERSE_NEEDS_SCHEDULE)).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("fas-interval"), { target: { value: "60" } });
    expect(screen.queryByText(REVERSE_NEEDS_SCHEDULE)).not.toBeInTheDocument();
    expect(screen.getByTestId("fas-summary")).toHaveTextContent(
      "6 prices from 10 to 5 HNS, one every 60 min; the last becomes valid ~5 h after the first.",
    );
    const submit = await submitButton();
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() => expect(calls("shakedex_finalize_and_sign")).toHaveLength(1));
    expect(calls("shakedex_finalize_and_sign")[0]![1]).toEqual({
      listingId: "l1",
      prices: [],
      schedule: { startPrice: "10", floorPrice: "5", steps: 6, intervalSecs: 3600 },
      feeRate: null,
    });
  });

  it("a field that is not a number leaves the schedule incomplete", async () => {
    setup();
    renderIt("reverseAuction");
    await typeSchedule("10", "5", "six", "60");
    expect(screen.getByText(REVERSE_NEEDS_SCHEDULE)).toBeInTheDocument();
    expect(screen.getByTestId("fas-submit")).toBeDisabled();
  });

  it("a step count that is not whole is refused in the backend's words, never cut down", async () => {
    setup();
    renderIt("reverseAuction");
    await typeSchedule("10", "5", "2.5", "60");
    expect(
      screen.getByText("reverse auction: 2.5 steps: a schedule has 2 to 1000 steps"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("fas-submit")).toBeDisabled();
  });

  it.each([
    ["15.1", 906],
    // A float gives 966.0000000000001 and 1997.9999999999998 for these.
    ["16.1", 966],
    ["33.3", 1998],
  ])("%s minutes between prices are sent as exactly %i s", async (minutes, secs) => {
    setup();
    renderIt("reverseAuction");
    await typeSchedule("10", "5", "6", minutes);
    const submit = await submitButton();
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() => expect(calls("shakedex_finalize_and_sign")).toHaveLength(1));
    expect(calls("shakedex_finalize_and_sign")[0]![1]).toMatchObject({
      schedule: { startPrice: "10", floorPrice: "5", steps: 6, intervalSecs: secs },
    });
  });

  it.each(["0x10", "1e3", "0b11"])(
    "a step count written %s is not a number here: the schedule is incomplete",
    async (steps) => {
      setup();
      renderIt("reverseAuction");
      await typeSchedule("10", "5", steps, "60");
      expect(screen.getByText(REVERSE_NEEDS_SCHEDULE)).toBeInTheDocument();
      expect(screen.getByTestId("fas-submit")).toBeDisabled();
    },
  );

  it("a finalize the backend answers without its draft says so instead of sent", async () => {
    setup({
      finalize: () => Promise.resolve(makeListing({ state: "finalizing", finalizeDraftId: null })),
    });
    const onClose = renderIt("buyNow");
    fireEvent.change(await screen.findByTestId("fas-price"), { target: { value: "5" } });
    const submit = await submitButton();
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    expect(await screen.findByTestId("fas-error")).toHaveTextContent(
      "The finalize was signed, but no transaction came back to send: My listings shows what is left to do.",
    );
    expect(calls("broadcast_tx_draft")).toHaveLength(0);
    expect(toasts()).toEqual([]);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("refuses steps closer than 15 minutes in the backend's words", async () => {
    setup();
    renderIt("reverseAuction");
    await typeSchedule("10", "5", "3", "10");
    expect(
      screen.getByText(
        "reverse auction: steps 600 s apart: steps must be at least 15 minutes apart",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("fas-submit")).toBeDisabled();
  });

  it("warns when the schedule runs past a year, and still allows it", async () => {
    setup();
    renderIt("reverseAuction");
    await typeSchedule("1000", "1", "1000", "1440");
    expect(screen.getByText(SCHEDULE_PAST_A_YEAR)).toBeInTheDocument();
    const submit = await submitButton();
    await waitFor(() => expect(submit).toBeEnabled());
    // R20 is the backend's prompt: the form lists no rows of its own.
    expect(screen.queryAllByTestId("fas-step-row")).toHaveLength(0);
  });

  it("gives no year warning for a schedule within a year", async () => {
    setup();
    renderIt("reverseAuction");
    await typeSchedule("10", "5", "6", "60");
    expect(screen.queryByText(SCHEDULE_PAST_A_YEAR)).not.toBeInTheDocument();
  });

  it("unlocks and runs again when the signer is locked", async () => {
    setup({ unlocked: false });
    renderIt("buyNow");
    fireEvent.change(await screen.findByTestId("fas-price"), { target: { value: "5" } });
    const submit = await submitButton();
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() => expect(calls("shakedex_finalize_and_sign")).toHaveLength(2));
    expect(calls("unlock_local_signer")).toEqual([
      ["unlock_local_signer", { walletProfileId: "p1" }],
    ]);
  });

  it("shows a declined confirmation and sends nothing", async () => {
    setup({ finalize: () => Promise.reject("Confirmation declined") });
    const onClose = renderIt("buyNow");
    fireEvent.change(await screen.findByTestId("fas-price"), { target: { value: "5" } });
    const submit = await submitButton();
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    expect(await screen.findByTestId("fas-error")).toHaveTextContent("Confirmation declined");
    expect(calls("broadcast_tx_draft")).toHaveLength(0);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("a FINALIZE whose broadcast fails says where it is kept, and offers no second sign", async () => {
    setup({
      broadcast: () => Promise.reject("node refused: gate"),
      listings: [
        makeMyListing("finalizing", {
          pendingSend: {
            draftId: "fin-1",
            action: "shakedex_finalize",
            signed: true,
            discardable: true,
          },
        }),
      ],
    });
    const onClose = renderIt("buyNow");
    fireEvent.change(await screen.findByTestId("fas-price"), { target: { value: "5" } });
    const submit = await submitButton();
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    const err = await screen.findByTestId("fas-error");
    await waitFor(() => expect(err).toHaveTextContent(UNSENT_KEPT));
    expect(screen.queryByTestId("fas-submit")).not.toBeInTheDocument();
    expect(calls("shakedex_finalize_and_sign")).toHaveLength(1);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("is disabled for a Ledger profile with the R16 sentence", async () => {
    setup({ kind: "ledger_hardware" });
    renderIt("buyNow");
    fireEvent.change(await screen.findByTestId("fas-price"), { target: { value: "5" } });
    expect(await screen.findByText(RECOVERY_PHRASE_ONLY)).toBeInTheDocument();
    expect(screen.getByTestId("fas-submit")).toBeDisabled();
  });
});
