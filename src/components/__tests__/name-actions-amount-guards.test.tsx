import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// The name page's Bid and Recover buttons are disabled while their amount is
// refused, so a page test cannot press them then. These tests call the
// handlers directly: whatever calls them, a refused amount reaches no backend
// command. Each refusal has a twin that the same handler does call the backend
// with exact doos, so a refusal cannot pass for a harness that reaches nothing.

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useNameActions } from "../name-page/useNameActions";

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "regtest",
  receiveAddress: "rs1qwallet",
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

function route(cmd: string) {
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
    case "build_bid_draft":
      return Promise.resolve({ id: "draft-1" });
    case "sign_tx_draft":
      return Promise.resolve({ id: "draft-1" });
    case "broadcast_tx_draft":
      return Promise.resolve({ txid: "abcdef0123456789" });
    case "recover_bid_commitment":
      return Promise.resolve({
        name: "bidname",
        address: "rs1qrecovered",
        bidValueDoos: 1_234_567_891,
        lockupValueDoos: 2_000_000_000,
      });
    default:
      return Promise.resolve(null);
  }
}

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);

async function mounted() {
  const hook = renderHook(() => useNameActions("bidname"), { wrapper: wrapper() });
  await waitFor(() => expect(hook.result.current.profile?.id).toBe("p1"));
  return hook;
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(route);
});

describe("useNameActions — a refused amount reaches no backend command", () => {
  it("submitBid builds nothing for a bid with a 7th decimal", async () => {
    const { result } = await mounted();
    act(() => {
      result.current.setBidHns("10.0000005");
      result.current.setLockupHns("12");
    });
    await act(async () => result.current.submitBid());
    expect(calls("build_bid_draft")).toHaveLength(0);
  });

  it("submitBid builds the bid in exact doos once the amounts are valid", async () => {
    const { result } = await mounted();
    act(() => {
      result.current.setBidHns("1234.567891");
      result.current.setLockupHns("2000.000001");
    });
    await act(async () => result.current.submitBid());
    await waitFor(() => expect(calls("build_bid_draft")).toHaveLength(1));
    expect(calls("build_bid_draft")[0]![1]).toMatchObject({
      bidValue: 1_234_567_891,
      lockup: 2_000_000_001,
    });
  });

  it("handleRecoverBid recovers nothing for an amount with a 7th decimal", async () => {
    const { result } = await mounted();
    act(() => result.current.setRecoverHns("10.0000005"));
    expect(result.current.recoverError).toBe("HNS has at most 6 decimals");
    await act(async () => result.current.handleRecoverBid());
    expect(calls("recover_bid_commitment")).toHaveLength(0);
  });

  it("handleRecoverBid sends exact doos once the amount is valid", async () => {
    const { result } = await mounted();
    act(() => result.current.setRecoverHns("1234.567891"));
    await act(async () => result.current.handleRecoverBid());
    expect(calls("recover_bid_commitment")).toHaveLength(1);
    expect(calls("recover_bid_commitment")[0]![1]).toMatchObject({ bidValueDoos: 1_234_567_891 });
  });
});
