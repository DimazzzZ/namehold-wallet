import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// The send flow's asset-safety guarantees, exercised at the UI level:
//   * the confirm screen shows the FULL recipient address (never truncated);
//   * a successful broadcast is reported with the node txid and closes the flow;
//   * a FAILED broadcast is surfaced as a persistent error, keeps the dialog
//     open, flips the action to "Retry", and is never mistaken for a send;
//   * invalid input never builds a draft.

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { WalletView } from "../WalletView";

// A full-length recipient address; the confirm screen must render it verbatim.
const RECIPIENT = "rs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2";
const NODE_TXID = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

const baseProfile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "regtest",
  accountXpub: "xpubFAKE000000000000",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: "rs1qexamplereceiveaddr",
  lastSyncedHeight: 10,
  lastSyncedAt: null,
  watchOnly: false,
  hasPassphrase: true,
  active: true,
};

function routeInvoke(opts: { broadcast: "ok" | "fail" } = { broadcast: "ok" }) {
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([baseProfile]);
      case "get_signer_session":
        return Promise.resolve({
          walletProfileId: baseProfile.id,
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
      case "get_wallet_balances":
        return Promise.resolve({
          liquidDoos: 5_000_000,
          nameControlDoos: 0,
          nameLockupDoos: 0,
          totalDoos: 5_000_000,
        });
      case "list_tx_drafts":
        return Promise.resolve([]);
      case "read_names":
        return Promise.resolve([]);
      case "build_send_hns_draft":
        return Promise.resolve({
          id: "d1",
          walletProfileId: baseProfile.id,
          action: "send_hns",
          status: "draft",
          summary: {
            action: "send_hns",
            sendTotalDoos: 1_000_000,
            feeDoos: 1410,
            changeDoos: 3_998_590,
            inputTotalDoos: 5_000_000,
            numInputs: 1,
            recipientAddress: RECIPIENT,
            txid: null,
            warnings: [],
          },
          errorMessage: null,
          txid: null,
          createdAt: "2026-01-01",
        });
      case "sign_tx_draft":
        return Promise.resolve({
          id: "d1",
          walletProfileId: baseProfile.id,
          action: "send_hns",
          status: "signed",
          summary: { recipientAddress: RECIPIENT, txid: "localtxid" },
          errorMessage: null,
          txid: null,
          createdAt: "2026-01-01",
        });
      case "broadcast_tx_draft":
        if (opts.broadcast === "fail") {
          return Promise.reject(
            new Error("Node RPC error: TX rejected: bad-txns-inputs-missingorspent"),
          );
        }
        return Promise.resolve({ draftId: "d1", txid: NODE_TXID, status: "broadcasted" });
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter initialEntries={["/wallet"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

async function openReview() {
  await screen.findByText("Primary");
  fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));
  fireEvent.change(screen.getByPlaceholderText(/rs1q/i), { target: { value: RECIPIENT } });
  fireEvent.change(screen.getByPlaceholderText("1.0"), { target: { value: "1" } });
  fireEvent.click(screen.getByRole("button", { name: /Review/i }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: /Sign & Broadcast/i })).toBeInTheDocument(),
  );
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("WalletView send flow (asset safety)", () => {
  it("shows the FULL recipient address on the confirm screen, untruncated", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await openReview();

    const recipient = screen.getByTestId("send-recipient");
    expect(recipient).toHaveTextContent(RECIPIENT);
    // The fix: no CSS truncation that could hide a changed address.
    expect(recipient.className).toContain("break-all");
    expect(recipient.className).not.toContain("truncate");
  });

  it("shows the amount in HNS (doos → HNS), not raw dollarydoos", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await openReview();

    // The mocked draft sends 1_000_000 doos; the review must show 1 HNS, never
    // the raw doos value (a wrong denominator would misstate the amount by 1e6).
    expect(screen.getByText("1.000000 HNS")).toBeInTheDocument();
    expect(screen.queryByText(/1000000(\.0+)? HNS/)).toBeNull();
  });

  it("on successful broadcast, signs then broadcasts and closes the flow", async () => {
    invokeMock.mockImplementation(routeInvoke({ broadcast: "ok" }));
    render(<WalletView />, { wrapper: wrapper() });
    await openReview();

    fireEvent.click(screen.getByRole("button", { name: /Sign & Broadcast/i }));

    // Sign then broadcast were both invoked, in that order.
    await waitFor(() => {
      const calls = invokeMock.mock.calls.map((c) => c[0]);
      expect(calls).toContain("sign_tx_draft");
      expect(calls).toContain("broadcast_tx_draft");
    });
    // The confirm dialog resets (send total no longer shown).
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /Sign & Broadcast/i })).toBeNull(),
    );
    expect(screen.queryByTestId("send-error")).toBeNull();
  });

  it("on broadcast failure: shows a persistent error, keeps the dialog open, and never reports success", async () => {
    invokeMock.mockImplementation(routeInvoke({ broadcast: "fail" }));
    render(<WalletView />, { wrapper: wrapper() });
    await openReview();

    fireEvent.click(screen.getByRole("button", { name: /Sign & Broadcast/i }));

    // The failure is surfaced inline and states the coins were not moved.
    const err = await screen.findByTestId("send-error");
    expect(err).toHaveTextContent(/not sent/i);
    expect(err).toHaveTextContent(/not moved/i);

    // The dialog stays open (recipient still visible) and offers a retry.
    expect(screen.getByTestId("send-recipient")).toHaveTextContent(RECIPIENT);
    expect(
      screen.getByRole("button", { name: /Retry Sign & Broadcast/i }),
    ).toBeInTheDocument();

    // Broadcast was attempted (so the failure is real, not a pre-check).
    expect(invokeMock.mock.calls.map((c) => c[0])).toContain("broadcast_tx_draft");
  });

  it("does not build a draft for an empty address", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));
    // Leave address empty, set an amount.
    fireEvent.change(screen.getByPlaceholderText("1.0"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: /Review/i }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.map((c) => c[0])).not.toContain("build_send_hns_draft");
    });
    expect(screen.queryByRole("button", { name: /Sign & Broadcast/i })).toBeNull();
  });

  it("does not build a draft for a non-positive amount", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));
    fireEvent.change(screen.getByPlaceholderText(/rs1q/i), { target: { value: RECIPIENT } });
    fireEvent.change(screen.getByPlaceholderText("1.0"), { target: { value: "0" } });
    fireEvent.click(screen.getByRole("button", { name: /Review/i }));

    await waitFor(() => {
      expect(invokeMock.mock.calls.map((c) => c[0])).not.toContain("build_send_hns_draft");
    });
  });

  it("shows an inline error for a wrong-network address and disables Review", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));
    // A mainnet hs1… address on a regtest wallet.
    fireEvent.change(screen.getByPlaceholderText(/rs1q/i), {
      target: { value: "hs1qkc9l7ykllufaxa6yfq47krr5xlcunyqv3svqj2" },
    });
    fireEvent.change(screen.getByPlaceholderText("1.0"), { target: { value: "1" } });

    expect(await screen.findByTestId("send-address-error")).toHaveTextContent(/valid regtest address/i);
    expect(screen.getByRole("button", { name: /Review/i })).toBeDisabled();
  });

  it("shows an inline error when the amount exceeds the spendable balance", async () => {
    invokeMock.mockImplementation(routeInvoke()); // spendable = 5,000,000 doos (5 HNS)
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));
    fireEvent.change(screen.getByPlaceholderText(/rs1q/i), { target: { value: RECIPIENT } });
    fireEvent.change(screen.getByPlaceholderText("1.0"), { target: { value: "10" } }); // > 5 HNS

    expect(await screen.findByTestId("send-amount-error")).toHaveTextContent(/exceeds your spendable/i);
    expect(screen.getByRole("button", { name: /Review/i })).toBeDisabled();
  });

  it("Max builds a sweep draft (max:true)", async () => {
    invokeMock.mockImplementation(routeInvoke());
    render(<WalletView />, { wrapper: wrapper() });
    await screen.findByText("Primary");
    fireEvent.click(screen.getByRole("button", { name: /Send HNS/i }));
    fireEvent.change(screen.getByPlaceholderText(/rs1q/i), { target: { value: RECIPIENT } });
    fireEvent.click(screen.getByRole("button", { name: /^Max$/i }));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "build_send_hns_draft");
      expect(call?.[1]).toMatchObject({ toAddress: RECIPIENT, max: true });
    });
  });
});
