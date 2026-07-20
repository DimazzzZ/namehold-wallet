import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// Task 13 (F6 follow-up): the build → sign → broadcast pipeline run by
// NameActionsModal must (a) attribute a failed mutation's error toast to the
// stage that threw ("Build failed: …" / "Sign failed: …" / "Broadcast
// failed: …"), and (b) never close the modal on error — inputs must survive
// so the user can retry instead of re-typing everything.

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NameActionsModal } from "../NameActionsModal";
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
        return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
      case "get_write_capability":
        return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
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

beforeEach(() => {
  invokeMock.mockReset();
  useUiStore.setState({ toastQueue: [], toastMessage: null });
});

async function placeBid(onClose: () => void) {
  render(<NameActionsModal name="bidname" open onClose={onClose} />, { wrapper: wrapper() });
  await screen.findByText("Place a Bid");
  fireEvent.change(screen.getByLabelText(/^Bid \(HNS\)/i), { target: { value: "10" } });
  fireEvent.change(screen.getByLabelText(/Lockup \(HNS\)/i), { target: { value: "12" } });
  const submitBtn = await screen.findByRole("button", { name: /Placing bid…|^Bid$/i });
  fireEvent.click(submitBtn);
}

describe("NameActionsModal — mutation error handling (Task 13)", () => {
  it("attributes a build-stage failure as 'Build failed: …'", async () => {
    const onClose = vi.fn();
    invokeMock.mockImplementation(
      route({ build_bid_draft: () => Promise.reject(new Error("insufficient funds")) }),
    );
    await placeBid(onClose);

    await waitFor(() => {
      const toast = ((q) => q[q.length - 1])(useUiStore.getState().toastQueue);
      expect(toast?.message).toBe("Build failed: Insufficient HNS balance for this transaction.");
      expect(toast?.type).toBe("error");
    });
  });

  it("attributes a sign-stage failure as 'Sign failed: …'", async () => {
    const onClose = vi.fn();
    invokeMock.mockImplementation(
      route({ sign_tx_draft: () => Promise.reject(new Error("wallet locked")) }),
    );
    await placeBid(onClose);

    await waitFor(() => {
      const toast = ((q) => q[q.length - 1])(useUiStore.getState().toastQueue);
      expect(toast?.message).toBe("Sign failed: Your signer is locked — click Unlock first.");
      expect(toast?.type).toBe("error");
    });
  });

  it("attributes a broadcast-stage failure as 'Broadcast failed: …'", async () => {
    const onClose = vi.fn();
    invokeMock.mockImplementation(
      route({ broadcast_tx_draft: () => Promise.reject(new Error("connection reset")) }),
    );
    await placeBid(onClose);

    await waitFor(() => {
      const toast = ((q) => q[q.length - 1])(useUiStore.getState().toastQueue);
      expect(toast?.message).toBe("Broadcast failed: Connection lost. Please try again.");
      expect(toast?.type).toBe("error");
    });
  });

  it("does not close the modal and keeps typed inputs intact when a mutation fails", async () => {
    const onClose = vi.fn();
    invokeMock.mockImplementation(
      route({ broadcast_tx_draft: () => Promise.reject(new Error("connection reset")) }),
    );
    await placeBid(onClose);

    await waitFor(() => {
      const toast = ((q) => q[q.length - 1])(useUiStore.getState().toastQueue);
      expect(toast?.type).toBe("error");
    });

    expect(onClose).not.toHaveBeenCalled();
    // The modal is still on screen and the typed values weren't wiped.
    expect(screen.getByTestId("name-phase")).toBeInTheDocument();
    expect(screen.getByLabelText(/^Bid \(HNS\)/i)).toHaveValue(10);
    expect(screen.getByLabelText(/Lockup \(HNS\)/i)).toHaveValue(12);
  });

  it("closes the modal on success (unchanged behavior)", async () => {
    const onClose = vi.fn();
    invokeMock.mockImplementation(route());
    await placeBid(onClose);

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledTimes(1);
    });
  });
});
