import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Guards the "built-but-not-broadcast draft cleanup" behaviour of the name
// page. build_*_draft persists a row with reserved coins eagerly, so an
// orphaned draft trips the backend double-action guard on retry ("an auction
// for '…' is already being opened"). The page must discard the row when the
// user cancels the Confirm & Sign overlay — and must NOT discard once a
// broadcast may be in flight, nor when the user leaves mid-run (the run itself
// owns the draft until it settles).

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NamePageAt } from "../../test/namePage";
import { useUiStore } from "../../stores/ui";

const profile = {
  id: "p1",
  label: "Primary",
  network: "regtest",
  receiveAddress: "rs1qwallet",
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

function route(overrides: Record<string, () => Promise<unknown>> = {}) {
  return (cmd: string) => {
    if (overrides[cmd]) return overrides[cmd]!();
    if (cmd === "get_name_action_capabilities") {
      return Promise.resolve({
        name: "bidname",
        phase: "BIDDING",
        taskState: "readyToBid",
        ownsName: false,
        nameIsRegistered: false,
        transferPending: false,
        redeemableRevealCount: 0,
        redeemableValueDoos: 0,
        hasBidCommitment: false,
        hasRevealCoin: false,
        hasOwnerCoin: false,
        canOpen: { allowed: false, reason: "Phase is BIDDING" },
        canBid: { allowed: true, reason: null },
        canReveal: { allowed: false, reason: null },
        canRedeem: { allowed: false, reason: null },
        canRegister: { allowed: false, reason: null },
        canUpdate: { allowed: false, reason: null },
        canTransfer: { allowed: false, reason: null },
        canFinalize: { allowed: false, reason: null },
        canCancelTransfer: { allowed: false, reason: null },
        canRenew: { allowed: false, reason: null },
        canRevoke: { allowed: false, reason: null },
        nextActionKey: "BID",
        nextActionLabel: "Bid",
        nextActionReason: null,
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
          unlocked: true,
          unlockedUntilEpochMs: Date.now() + 60000,
        });
      case "get_write_capability":
        return Promise.resolve({
          signerUnlocked: true,
          broadcasterAvailable: true,
          canWrite: true,
          reason: null,
        });
      case "read_name_info":
        return Promise.resolve({
          name: "bidname",
          state: "BIDDING",
          registered: false,
          height: 100,
          renewal: 200,
          owner: null,
          value: null,
          highest: 5_000_000,
          stats: { blocksUntilReveal: 50, hoursUntilReveal: 8 },
        });
      case "build_bid_draft":
        return Promise.resolve({ id: "draft-1" });
      case "sign_tx_draft":
        return Promise.resolve({ id: "draft-1" });
      case "broadcast_tx_draft":
        return Promise.resolve({ txid: "abcdef0123456789" });
      case "delete_tx_draft":
        return Promise.resolve(null);
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  useUiStore.setState({ toastQueue: [], toastMessage: null });
});

async function placeBid() {
  const utils = render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });
  await screen.findByText("Place a Bid");
  fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "10" } });
  fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "12" } });
  const submitBtn = await screen.findByRole("button", { name: /Placing bid…|^Bid$/i });
  fireEvent.click(submitBtn);
  return utils;
}

function invokedCommands(): string[] {
  return invokeMock.mock.calls.map((c) => c[0] as string);
}

function deleteDraftCalls(): unknown[] {
  return invokeMock.mock.calls.filter((c) => c[0] === "delete_tx_draft").map((c) => c[1]);
}

describe("Name page — built-but-not-broadcast draft cleanup", () => {
  // A sign-stage failure includes Cancel on the Confirm & Sign overlay
  // (useExecuteDraft tags unlock/sign rejections stage="sign"). The just-built
  // draft must be discarded so a retry isn't blocked by the double-action guard.
  it("discards the draft when the sign stage fails (Confirm & Sign cancel)", async () => {
    invokeMock.mockImplementation(
      route({ sign_tx_draft: () => Promise.reject(new Error("user cancelled")) }),
    );
    await placeBid();

    await waitFor(() => {
      expect(deleteDraftCalls()).toEqual([{ draftId: "draft-1" }]);
    });
  });

  // A broadcast-stage failure means the tx may be in flight; deleting here
  // would be unsafe (and the backend delete_tx_draft refuses it anyway).
  it("does NOT discard the draft when the broadcast stage fails (tx may be in flight)", async () => {
    invokeMock.mockImplementation(
      route({ broadcast_tx_draft: () => Promise.reject(new Error("connection reset")) }),
    );
    await placeBid();

    await waitFor(() => {
      const toast = ((q) => q[q.length - 1])(useUiStore.getState().toastQueue);
      expect(toast?.type).toBe("error");
    });
    expect(deleteDraftCalls()).toHaveLength(0);
  });

  // Build-stage failures never persist a draft, so no cleanup should fire.
  it("does not call delete_tx_draft when the build stage fails", async () => {
    invokeMock.mockImplementation(
      route({ build_bid_draft: () => Promise.reject(new Error("insufficient funds")) }),
    );
    await placeBid();

    await waitFor(() => {
      const toast = ((q) => q[q.length - 1])(useUiStore.getState().toastQueue);
      expect(toast?.type).toBe("error");
    });
    expect(deleteDraftCalls()).toHaveLength(0);
  });

  // Happy path: broadcast succeeds → nothing discarded, a toast, and the page
  // stays (no close after broadcast).
  it("does not discard on the successful build → sign → broadcast path", async () => {
    invokeMock.mockImplementation(route());
    await placeBid();

    await waitFor(() => {
      const toast = ((q) => q[q.length - 1])(useUiStore.getState().toastQueue);
      expect(toast?.message).toMatch(/^BID broadcast — abcdef012345…/);
    });
    expect(deleteDraftCalls()).toHaveLength(0);
    expect(invokedCommands()).toContain("broadcast_tx_draft");
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(".bidname");
    // The bid inputs are cleared for the next bid.
    expect(screen.getByLabelText(/^Bid \(HNS\)/i)).toHaveValue(null);
  });

  it("makes no delete call when the page is left idle", async () => {
    invokeMock.mockImplementation(route());
    const utils = render(<NamePageAt path="/names/bidname" />, { wrapper: wrapper() });
    await screen.findByText("Place a Bid");
    utils.unmount();
    await new Promise((r) => setTimeout(r, 0));
    expect(deleteDraftCalls()).toHaveLength(0);
  });

  // Leaving the page while a run is in flight (sign still hanging) must not
  // discard: the run owns the draft until it settles, and discards it itself
  // if the sign stage then fails.
  it("does not discard when the page is left mid-sign; the failing run discards once", async () => {
    let rejectSign: (e: unknown) => void = () => {};
    const hangingSign = () =>
      new Promise<unknown>((_resolve, reject) => {
        rejectSign = reject;
      });
    invokeMock.mockImplementation(route({ sign_tx_draft: hangingSign }));

    const utils = await placeBid();
    await waitFor(() => {
      expect(invokedCommands()).toContain("build_bid_draft");
      expect(invokedCommands()).toContain("sign_tx_draft");
    });

    utils.unmount();
    await new Promise((r) => setTimeout(r, 0));
    expect(deleteDraftCalls()).toHaveLength(0);

    rejectSign(new Error("user cancelled"));
    await waitFor(() => {
      expect(deleteDraftCalls()).toEqual([{ draftId: "draft-1" }]);
    });
  });
});
