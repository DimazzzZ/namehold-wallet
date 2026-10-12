import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { LockDialog } from "../market/LockDialog";
import {
  LOCK_COSTS,
  MAINNET_SELLING_EXPERIMENTAL,
  NEEDS_SENDING_NODE,
  PUBLISH_PRIVACY,
  RECOVERY_PHRASE_ONLY,
  SEND_AGAIN_KEPT,
  SHARED_AS_FILE_ONLY,
  UNSENT_KEPT,
} from "../market/marketText";
import { makeMyListing } from "../../test/fixtures/listings";
import { loadSettings } from "../../test/fixtures/settings";
import { makeProfile, makeSession } from "../../test/fixtures/wallet";
import { useUiStore } from "../../stores/ui";
import type {
  ShakedexMyListing,
  TxDraftSummary,
  WalletNetwork,
  WalletProfileKind,
} from "../../types";

/** `tx_evidence::NOT_TAKEN`: the draft is held `broadcast_pending`. */
const NOT_TAKEN =
  "The node did not take the transaction. hsd does not say why; often its coins are already spent elsewhere. You can try again; the coins stay held until the node is checked again.";

/** `sell.rs::near_expiry_warning(2000)`: the dialog shows the backend's warning as written. */
const NEAR =
  "The name expires in 2000 blocks: do Finalize & sign before then. The FINALIZE into the lock renews the name, so this expiry does not cut the listing short.";
/** `sell.rs::expires_before_the_lock` as `AppError::InvalidInput` writes it. */
const REFUSED =
  "Invalid input: 'dexsale' expires at block 6000, before its transfer into the lock could be finalized: renew it first";

function draft(warnings: string[]): TxDraftSummary {
  return {
    id: "lock-1",
    walletProfileId: "p1",
    action: "shakedex_lock",
    status: "draft",
    summary: {
      action: "shakedex_lock",
      sendTotalDoos: 0,
      feeDoos: 2_800,
      changeDoos: 0,
      inputTotalDoos: 2_800,
      numInputs: 1,
      recipientAddress: "rs1qlock",
      txid: null,
      warnings,
    },
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: "2026-10-12",
    purchaseLostReason: null,
  };
}

interface Opts {
  kind?: WalletProfileKind;
  network?: WalletNetwork;
  unlocked?: boolean;
  build?: () => Promise<unknown>;
  sign?: () => Promise<unknown>;
  broadcast?: () => Promise<unknown>;
  /** False: a node that cannot send (`get_write_capability`). */
  broadcasterAvailable?: boolean;
  /** The signer session lapses right after the lock is built. */
  lapseAfterBuild?: boolean;
  /** My listings as the backend gives it after the send (default: no rows). */
  listings?: ShakedexMyListing[];
}

function setup(o: Opts = {}) {
  // The signer as the backend holds it: the lock refuses "Wallet locked"
  // (`authorize_signer`) until the profile's signer is unlocked.
  let unlocked = o.unlocked ?? true;
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([
          makeProfile({ kind: o.kind ?? "mnemonic_hot", network: o.network ?? "regtest" }),
        ]);
      case "get_signer_session":
        return Promise.resolve(makeSession({ unlocked }));
      case "unlock_local_signer":
        unlocked = true;
        return Promise.resolve(null);
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: unlocked,
          broadcasterAvailable: o.broadcasterAvailable ?? true,
          canWrite: o.broadcasterAvailable ?? true,
          reason: null,
        });
      case "shakedex_build_lock_draft":
        if (!unlocked) return Promise.reject("Wallet locked");
        if (o.lapseAfterBuild) unlocked = false;
        return o.build ? o.build() : Promise.resolve(draft([LOCK_COSTS]));
      case "sign_tx_draft":
        if (!unlocked) return Promise.reject("Wallet locked");
        return o.sign ? o.sign() : Promise.resolve(draft([LOCK_COSTS]));
      case "broadcast_tx_draft":
        return o.broadcast
          ? o.broadcast()
          : Promise.resolve({ draftId: "lock-1", txid: "ab", status: "broadcasted" });
      case "shakedex_list_listings":
        return Promise.resolve(o.listings ?? []);
      default:
        return Promise.resolve(null);
    }
  });
}

/** The Locking row of the lock draft `lock-1`, as `pending_send` gives it. */
function lockingRow(discardable: boolean): ShakedexMyListing {
  return makeMyListing("locking", {
    pendingSend: { draftId: "lock-1", action: "shakedex_lock", signed: true, discardable },
  });
}

function renderIt(onClose = vi.fn()) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  render(<LockDialog open name="dexsale" onClose={onClose} />, { wrapper });
  return onClose;
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);

/** The Lock button once the profile and the node are known (disabled until then). */
async function lockButton() {
  const b = await screen.findByTestId("lock-build");
  await waitFor(() => expect(b).toBeEnabled());
  return b;
}

beforeEach(() => {
  loadSettings();
  useUiStore.setState({ toastQueue: [], toastMessage: null });
});

describe("LockDialog", () => {
  it("states what locking costs before anything is built (R27)", async () => {
    setup();
    renderIt();
    expect(await screen.findByTestId("lock-costs")).toHaveTextContent(LOCK_COSTS);
    expect(calls("shakedex_build_lock_draft")).toHaveLength(0);
  });

  it("says the price or schedule is set at Finalize & sign, not here (R19)", async () => {
    setup();
    renderIt();
    expect(
      await screen.findByText(
        "You set the price, or the schedule, at Finalize & sign, once the transfer lockup is over.",
      ),
    ).toBeInTheDocument();
  });

  it("shows the near-expiry warning from the lock draft before anything is signed (R31)", async () => {
    setup({ build: () => Promise.resolve(draft([LOCK_COSTS, NEAR])) });
    renderIt();
    const lock = await lockButton();
    fireEvent.click(lock);
    expect(await screen.findByText(NEAR)).toBeInTheDocument();
    // The costs are on screen once, not again as a draft warning.
    expect(screen.getAllByText(LOCK_COSTS)).toHaveLength(1);
    expect(calls("sign_tx_draft")).toHaveLength(0);
    expect(screen.getByTestId("lock-send")).toBeEnabled();
  });

  it("shows the backend's refusal of a name that would expire first, and nothing to send (R31)", async () => {
    setup({ build: () => Promise.reject(REFUSED) });
    renderIt();
    const lock = await lockButton();
    fireEvent.click(lock);
    expect(await screen.findByTestId("lock-error")).toHaveTextContent(
      "before its transfer into the lock could be finalized",
    );
    expect(screen.queryByTestId("lock-send")).not.toBeInTheDocument();
  });

  it("has the publish box on by default on mainnet, with the privacy note (R23)", async () => {
    loadSettings({ shakedex_experimental: "true" });
    setup({ network: "mainnet" });
    renderIt();
    const box = await screen.findByTestId("lock-publish");
    expect(box).toBeChecked();
    expect(screen.getByText(PUBLISH_PRIVACY)).toBeInTheDocument();
    const lock = await lockButton();
    fireEvent.click(lock);
    await waitFor(() => expect(calls("shakedex_build_lock_draft")).toHaveLength(1));
    expect(calls("shakedex_build_lock_draft")[0]![1]).toMatchObject({
      name: "dexsale",
      publish: true,
      mode: "buyNow",
      feeRate: null,
    });
  });

  it("locks unpublished on mainnet when the box is unticked", async () => {
    loadSettings({ shakedex_experimental: "true" });
    setup({ network: "mainnet" });
    renderIt();
    fireEvent.click(await screen.findByTestId("lock-publish"));
    const lock = await lockButton();
    fireEvent.click(lock);
    await waitFor(() => expect(calls("shakedex_build_lock_draft")).toHaveLength(1));
    expect(calls("shakedex_build_lock_draft")[0]![1]).toMatchObject({ publish: false });
  });

  it("has no publish box on regtest, says the listing is a file only, and locks unpublished (R29)", async () => {
    setup({ network: "regtest" });
    renderIt();
    const lock = await lockButton();
    expect(screen.queryByTestId("lock-publish")).not.toBeInTheDocument();
    expect(screen.getByText(SHARED_AS_FILE_ONLY)).toBeInTheDocument();
    fireEvent.click(lock);
    await waitFor(() => expect(calls("shakedex_build_lock_draft")).toHaveLength(1));
    expect(calls("shakedex_build_lock_draft")[0]![1]).toMatchObject({ publish: false });
  });

  it.each(["ledger_hardware", "watch_only_xpub", "xpriv_hot"] as const)(
    "is disabled with the R16 reason for %s (R29)",
    async (kind) => {
      setup({ kind });
      renderIt();
      expect(await screen.findByText(RECOVERY_PHRASE_ONLY)).toBeInTheDocument();
      expect(screen.getByTestId("lock-build")).toBeDisabled();
    },
  );

  it("is disabled with the node sentence when the node cannot send (R6)", async () => {
    setup({ broadcasterAvailable: false });
    renderIt();
    expect(await screen.findByText(NEEDS_SENDING_NODE)).toBeInTheDocument();
    expect(screen.getByTestId("lock-build")).toBeDisabled();
  });

  it("is disabled on mainnet without the experimental flag (R29)", async () => {
    loadSettings({ shakedex_experimental: "false" });
    setup({ network: "mainnet" });
    renderIt();
    expect(await screen.findByText(MAINNET_SELLING_EXPERIMENTAL)).toBeInTheDocument();
    expect(screen.getByTestId("lock-build")).toBeDisabled();
  });

  it("a sent lock's toast names the tab and the button where the price is set", async () => {
    setup();
    const onClose = renderIt();
    fireEvent.click(await lockButton());
    fireEvent.click(await screen.findByTestId("lock-send"));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(useUiStore.getState().toastQueue.map((t) => t.message)).toEqual([
      "Lock sent. Once the transfer lockup is over, set the price with Finalize & sign in Market › My listings.",
    ]);
  });

  it("on a locked wallet unlocks this profile's signer, then builds, signs and sends", async () => {
    setup({ unlocked: false });
    const onClose = renderIt();
    fireEvent.click(await screen.findByTestId("lock-mode-reverse"));
    const lock = await lockButton();
    fireEvent.click(lock);
    fireEvent.click(await screen.findByTestId("lock-send"));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    const order = invokeMock.mock.calls
      .map((c) => c[0])
      .filter((c) =>
        [
          "unlock_local_signer",
          "shakedex_build_lock_draft",
          "sign_tx_draft",
          "broadcast_tx_draft",
        ].includes(c),
      );
    expect(order).toEqual([
      "shakedex_build_lock_draft",
      "unlock_local_signer",
      "shakedex_build_lock_draft",
      "sign_tx_draft",
      "broadcast_tx_draft",
    ]);
    expect(calls("unlock_local_signer")).toEqual([
      ["unlock_local_signer", { walletProfileId: "p1" }],
    ]);
    expect(calls("shakedex_build_lock_draft")[1]![1]).toMatchObject({ mode: "reverseAuction" });
  });

  it("a signer session that lapses before the send is unlocked once, and the lock is sent", async () => {
    setup({ lapseAfterBuild: true });
    const onClose = renderIt();
    fireEvent.click(await lockButton());
    fireEvent.click(await screen.findByTestId("lock-send"));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(calls("unlock_local_signer")).toEqual([
      ["unlock_local_signer", { walletProfileId: "p1" }],
    ]);
    expect(calls("sign_tx_draft")).toHaveLength(2);
    expect(calls("broadcast_tx_draft")).toHaveLength(1);
    expect(calls("delete_tx_draft")).toHaveLength(0);
  });

  it("a sign that fails shows why and deletes the unsent draft", async () => {
    setup({ sign: () => Promise.reject("Confirmation was cancelled") });
    const onClose = renderIt();
    fireEvent.click(await lockButton());
    fireEvent.click(await screen.findByTestId("lock-send"));
    expect(await screen.findByTestId("lock-error")).toHaveTextContent("Confirmation was cancelled");
    await waitFor(() =>
      expect(calls("delete_tx_draft")).toEqual([["delete_tx_draft", { draftId: "lock-1" }]]),
    );
    expect(calls("broadcast_tx_draft")).toHaveLength(0);
    expect(await screen.findByTestId("lock-build")).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });

  // `broadcast_tx_draft` after the sign: refused at its gates, the signed
  // lock stays with its Locking listing; not taken, it is held
  // `broadcast_pending`; refused by R31's recheck, it is deleted with its
  // listing. The dialog says what the refreshed My listings shows.
  it.each([
    ["refused at the gates", "remote broadcasting is not allowed", [lockingRow(true)], UNSENT_KEPT],
    ["not taken by the node", NOT_TAKEN, [lockingRow(false)], SEND_AGAIN_KEPT],
    [
      "refused by the expiry recheck",
      "Invalid input: 'dexsale' expires at block 6000, before its transfer into the lock could be finalized: renew it first",
      [],
      null,
    ],
  ] as const)(
    "a broadcast %s says where the lock is, deletes nothing and offers no second send",
    async (_, why, listings, kept) => {
      setup({ broadcast: () => Promise.reject(why), listings: [...listings] });
      const onClose = renderIt();
      fireEvent.click(await lockButton());
      fireEvent.click(await screen.findByTestId("lock-send"));
      await waitFor(() => expect(calls("shakedex_list_listings")).toHaveLength(1));
      const err = await screen.findByTestId("lock-error");
      await waitFor(() => expect(err).toHaveTextContent(why.slice(0, 40)));
      if (kept) expect(err).toHaveTextContent(kept);
      else expect(err).not.toHaveTextContent(/stays in My listings/);
      expect(screen.queryByTestId("lock-send")).not.toBeInTheDocument();
      expect(calls("delete_tx_draft")).toHaveLength(0);
      expect(calls("broadcast_tx_draft")).toHaveLength(1);
      expect(onClose).not.toHaveBeenCalled();
    },
  );

  it("Back after the build deletes the unsent draft, and its Locking listing with it", async () => {
    setup();
    const onClose = renderIt();
    const lock = await lockButton();
    fireEvent.click(lock);
    await screen.findByTestId("lock-send");
    fireEvent.click(screen.getByTestId("lock-back"));
    await waitFor(() =>
      expect(calls("delete_tx_draft")).toEqual([["delete_tx_draft", { draftId: "lock-1" }]]),
    );
    expect(await screen.findByTestId("lock-build")).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("closing after the build deletes the unsent draft too", async () => {
    setup();
    const onClose = renderIt();
    const lock = await lockButton();
    fireEvent.click(lock);
    await screen.findByTestId("lock-send");
    fireEvent.click(screen.getByText("×"));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(calls("delete_tx_draft")).toEqual([["delete_tx_draft", { draftId: "lock-1" }]]);
  });
});
