import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NameActionsModal } from "../NameActionsModal";
import { allowed, makeCapabilities, refused } from "../../test/fixtures/capabilities";
import { makeProfile, makeSession } from "../../test/fixtures/wallet";
import { loadSettings } from "../../test/fixtures/settings";
import {
  MAINNET_SELLING_EXPERIMENTAL,
  NEEDS_SENDING_NODE,
  RECOVERY_PHRASE_ONLY,
} from "../market/marketText";
import type { NameActionCapabilities, WalletNetwork, WalletProfileKind } from "../../types";

/** `shakedex::NAME_LOCKED_FOR_SALE`, word for word. */
const NAME_LOCKED_FOR_SALE = "this name is locked for sale through Shakedex";

const profile = {
  id: "p1",
  label: "Primary",
  network: "mainnet",
  receiveAddress: "hs1qwallet",
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

/**
 * `reason` is what the capability rows carry; `writeReason` is what
 * `get_write_capability` reports. They default to the same string — a test that
 * needs to tell the two notices apart passes them separately.
 */
function route(
  canWrite: boolean,
  reason: string | null,
  signerUnlocked = true,
  writeReason: string | null = reason,
) {
  return (cmd: string) => {
    if (cmd === "get_name_action_capabilities") {
      return Promise.resolve({
        name: "examplename",
        phase: "CLOSED",
        taskState: "wonNeedsRegister",
        ownsName: true,
        nameIsRegistered: false,
        transferPending: false,
        redeemableRevealCount: 0,
        redeemableValueDoos: 0,
        hasBidCommitment: false,
        hasRevealCoin: false,
        hasOwnerCoin: true,
        canOpen: { allowed: false, reason: null },
        canBid: { allowed: false, reason: "Phase is CLOSED" },
        canReveal: { allowed: false, reason: "No commitment" },
        canRedeem: { allowed: false, reason: null },
        canRegister: { allowed: canWrite, reason: canWrite ? null : reason },
        canUpdate: { allowed: canWrite, reason: canWrite ? null : reason },
        canTransfer: { allowed: canWrite, reason: canWrite ? null : reason },
        canFinalize: { allowed: false, reason: null },
        canCancelTransfer: { allowed: false, reason: null },
        canRenew: { allowed: canWrite, reason: canWrite ? null : reason },
        canRevoke: { allowed: canWrite, reason: canWrite ? null : reason },
        nextActionKey: "REGISTER",
        nextActionLabel: "Register Name",
        nextActionReason: canWrite ? null : reason,
        countdownLabel: null,
        countdownBlocks: null,
        countdownHours: null,
      });
    }
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "get_signer_session":
        return Promise.resolve({
          walletProfileId: "p1",
          unlocked: signerUnlocked,
          unlockedUntilEpochMs: Date.now() + 60000,
        });
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked,
          broadcasterAvailable: true,
          canWrite,
          reason: writeReason,
        });
      case "read_name_info":
        return Promise.resolve({
          name: "examplename",
          state: "CLOSED",
          registered: true,
          height: 5040,
          renewal: 329999,
          owner: { hash: "deadbeef", address: "hs1qwallet" },
          value: 100000,
          highest: 100000,
          stats: { openPeriodStart: 5000, biddingPeriodEnd: 5040, revealPeriodEnd: 5540 },
        });
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("NameActionsModal — node-readiness gating", () => {
  it("states the reason once and offers no menu when the node can't write", async () => {
    invokeMock.mockImplementation(
      route(
        false,
        "Your local node is still syncing (40%). On-chain sends and transfers need a fully-synced node.",
      ),
    );
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, { wrapper: wrapper() });

    // Wait for write-capability gating to settle, then assert on the banner itself.
    const blocked = await screen.findByTestId("name-actions-blocked");
    expect(blocked).toHaveTextContent(/name actions unavailable/i);

    // The mocked name is CLOSED/owned, so the guided action is Register and it
    // should be disabled.
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /^Register$/i })).toBeDisabled();
    });

    // A node that cannot write refuses every capability, so there is nothing
    // behind the advanced toggle and no toggle to open. The reason is already
    // stated twice — on the banner above and on the guided action — and a menu
    // of six buttons all carrying that same reason is the wall of dead
    // controls this modal no longer renders.
    expect(screen.queryByTestId("all-actions-toggle")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Reveal$/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Redeem$/i })).toBeNull();
    // No meaningful "Bid" action either, for a name whose auction is over.
    expect(screen.queryByRole("button", { name: /^Bid$/i })).toBeNull();
    expect(screen.queryByLabelText("Bid (HNS)")).not.toBeInTheDocument();
    // Close stays available.
    expect(screen.getByRole("button", { name: /^Close$/i })).not.toBeDisabled();
  });

  it("states the locked-wallet reason once, with an Unlock button", async () => {
    // Regression: the write-capability reason was printed twice — once by the
    // modal-wide gate (which has an Unlock button) and again, bare, inside the
    // guided action panel. One notice, one button, and it must be actionable.
    const reason = "Unlock your wallet to sign transactions.";
    invokeMock.mockImplementation(route(false, "Needs a synced owner coin.", false, reason));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, { wrapper: wrapper() });

    // Re-query on every tick: React swaps the banner's subtree once the
    // write-capability query resolves, so a node captured earlier goes stale.
    await waitFor(() =>
      expect(screen.getByTestId("name-actions-blocked")).toHaveTextContent(reason),
    );

    const occurrences = (document.body.textContent ?? "").split(reason).length - 1;
    expect(occurrences).toBe(1);
    expect(screen.getAllByTestId("unlock-now")).toHaveLength(1);
    expect(screen.getByTestId("name-actions-blocked")).toContainElement(
      screen.getByTestId("unlock-now"),
    );
  });

  it("enables actions once the node is write-capable", async () => {
    invokeMock.mockImplementation(route(true, null));
    render(<NameActionsModal name="examplename" open onClose={() => {}} />, { wrapper: wrapper() });

    // No blocked banner.
    await waitForWritable();
    expect(screen.queryByTestId("name-actions-blocked")).toBeNull();
    // Register should be enabled since canWrite is true and canRegister is allowed.
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /^Register$/i })).not.toBeDisabled();
    });
  });
});

// Small helper: the write-capability query resolves async; wait for the
// blocked banner to disappear (i.e., canWrite=true has been applied).
async function waitForWritable() {
  const { waitFor } = await import("@testing-library/react");
  await waitFor(() => expect(screen.queryByTestId("name-actions-blocked")).toBeNull());
}

describe("NameActionsModal — Sell", () => {
  /** A registered name of ours on a node that can write; Sell's gates vary. */
  function sellRoute(
    o: {
      kind?: WalletProfileKind;
      network?: WalletNetwork;
      caps?: Partial<NameActionCapabilities>;
      /** False: a node that cannot send (`get_write_capability`). */
      broadcasterAvailable?: boolean;
      build?: () => Promise<unknown>;
    } = {},
  ) {
    return (cmd: string) => {
      switch (cmd) {
        case "get_name_action_capabilities":
          return Promise.resolve(
            makeCapabilities({
              name: "examplename",
              ownsName: true,
              nameIsRegistered: true,
              hasOwnerCoin: true,
              canTransfer: allowed,
              ...o.caps,
            }),
          );
        case "list_wallet_profiles":
          return Promise.resolve([
            makeProfile({ kind: o.kind ?? "mnemonic_hot", network: o.network ?? "regtest" }),
          ]);
        case "get_signer_session":
          return Promise.resolve(makeSession({ unlocked: true }));
        case "get_write_capability": {
          const sends = o.broadcasterAvailable ?? true;
          return Promise.resolve({
            signerUnlocked: true,
            broadcasterAvailable: sends,
            canWrite: sends,
            reason: sends ? null : "Sending is not allowed through this node",
          });
        }
        case "shakedex_build_lock_draft":
          return o.build ? o.build() : Promise.resolve(null);
        default:
          return Promise.resolve(null);
      }
    };
  }

  async function openSell(onClose: () => void = () => {}) {
    render(<NameActionsModal name="examplename" open onClose={onClose} />, {
      wrapper: wrapper(),
    });
    // A registered name of ours opens with its management section expanded.
    return screen.findByTestId("name-sell");
  }

  async function hint(button: HTMLElement) {
    fireEvent.mouseEnter(button.parentElement!);
    return screen.findByRole("tooltip");
  }

  beforeEach(() => loadSettings());

  it("Sell opens the Lock dialog for the name", async () => {
    invokeMock.mockImplementation(sellRoute());
    const sell = await openSell();
    await waitFor(() => expect(sell).toBeEnabled());
    fireEvent.click(sell);
    expect(await screen.findByText("Sell .examplename")).toBeInTheDocument();
    expect(screen.getByTestId("lock-build")).toBeInTheDocument();
  });

  it("Sell is disabled with the backend's reason when the name cannot be transferred (R27)", async () => {
    // A lock is a TRANSFER to the lock address: `canTransfer` is its covenant gate.
    invokeMock.mockImplementation(
      sellRoute({
        caps: { canTransfer: refused("this name is locked for sale through Shakedex") },
      }),
    );
    const sell = await openSell();
    await waitFor(() => expect(sell).toBeDisabled());
    expect(await hint(sell)).toHaveTextContent("this name is locked for sale through Shakedex");
  });

  it("a Listed name keeps its owner actions, each refused because it is locked for sale (R27)", async () => {
    // What the backend reports once the coin sits at the lock address: still
    // ours and registered, no owner coin, every owner action refused for the
    // sale. The modal must not fall back to a stranger's "already registered".
    const forSale = refused(NAME_LOCKED_FOR_SALE);
    const listed = sellRoute({
      caps: {
        taskState: "ownedNoUrgentAction",
        hasOwnerCoin: false,
        lockedForSale: true,
        canRegister: forSale,
        canUpdate: forSale,
        canTransfer: forSale,
        canFinalize: forSale,
        canCancelTransfer: forSale,
        canRenew: forSale,
        canRevoke: forSale,
      },
    });
    // The name's chain state, so the guided panel renders as in the app.
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "read_name_info"
        ? Promise.resolve({
            name: "examplename",
            state: "CLOSED",
            registered: true,
            height: 5040,
            renewal: 329999,
            owner: { hash: "deadbeef", address: "rs1qlock" },
            value: 100000,
            highest: 100000,
            stats: { openPeriodStart: 5000, biddingPeriodEnd: 5040, revealPeriodEnd: 5540 },
          })
        : listed(cmd),
    );
    const sell = await openSell();
    await screen.findByText(/You own this name/);
    expect(screen.queryByText(/This name is already registered/)).toBeNull();
    expect(screen.getByTestId("ownership-indicator")).toBeInTheDocument();
    for (const label of [/^Transfer$/, /^Renew$/, /^Revoke$/]) {
      const button = screen.getByRole("button", { name: label });
      expect(button).toBeDisabled();
      expect(await hint(button)).toHaveTextContent(NAME_LOCKED_FOR_SALE);
      fireEvent.mouseLeave(button.parentElement!);
      await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
    }
    expect(sell).toBeDisabled();
    expect(await hint(sell)).toHaveTextContent(NAME_LOCKED_FOR_SALE);
    // Its owner coin sits at the lock address by design: not a sync gap.
    expect(screen.queryByTestId("owner-coin-not-synced")).toBeNull();
    expect(screen.getByTestId("owner-coin-in-lock")).toHaveTextContent(
      "Its owner coin is in the Shakedex lock; manage the listing in Market › My listings.",
    );
  });

  it.each(["ledger_hardware", "watch_only_xpub"] as const)(
    "Sell is disabled with the R16 sentence for a %s profile",
    async (kind) => {
      invokeMock.mockImplementation(sellRoute({ kind }));
      const sell = await openSell();
      await waitFor(() => expect(sell).toBeDisabled());
      expect(await hint(sell)).toHaveTextContent(RECOVERY_PHRASE_ONLY);
    },
  );

  it("Sell is disabled with the node sentence when the node cannot send (R6)", async () => {
    invokeMock.mockImplementation(sellRoute({ broadcasterAvailable: false }));
    const sell = await openSell();
    await waitFor(() => expect(sell).toBeDisabled());
    expect(await hint(sell)).toHaveTextContent(NEEDS_SENDING_NODE);
  });

  it("Escape in the Lock dialog leaves the name's actions open, even while a lock is being built", async () => {
    // The build never answers: closing the parent now would unmount the
    // Lock dialog with its draft on the way and nothing left to delete it.
    invokeMock.mockImplementation(sellRoute({ build: () => new Promise(() => {}) }));
    const onClose = vi.fn();
    const sell = await openSell(onClose);
    await waitFor(() => expect(sell).toBeEnabled());
    fireEvent.click(sell);
    const lock = await screen.findByTestId("lock-build");
    await waitFor(() => expect(lock).toBeEnabled());
    fireEvent.click(lock);
    await waitFor(() => expect(lock).toBeDisabled());
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByTestId("lock-build")).toBeInTheDocument();
  });

  it("Escape closes the Lock dialog alone; the next one closes the name's actions", async () => {
    invokeMock.mockImplementation(sellRoute());
    const onClose = vi.fn();
    const sell = await openSell(onClose);
    await waitFor(() => expect(sell).toBeEnabled());
    fireEvent.click(sell);
    await screen.findByTestId("lock-build");
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("lock-build")).not.toBeInTheDocument());
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Sell is disabled on mainnet until Settings allows selling (R29)", async () => {
    loadSettings({ shakedex_experimental: "false" });
    invokeMock.mockImplementation(sellRoute({ network: "mainnet" }));
    const sell = await openSell();
    await waitFor(() => expect(sell).toBeDisabled());
    expect(await hint(sell)).toHaveTextContent(MAINNET_SELLING_EXPERIMENTAL);
  });
});
