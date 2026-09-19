/**
 * NameBidsPanel — the bid list inside the name modal.
 *
 * The list is fed by the chain scanner's index, which only holds BID outputs
 * found in BLOCKS. A bid this wallet just sent sits in the mempool, so it used
 * to be absent entirely: place a second bid and the panel still read "1 bids
 * so far", as if the transaction had never happened.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { NameBidsPanel } from "../name-actions/NameBidsPanel";

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

function bid(over: Record<string, unknown> = {}) {
  return {
    txid: "mined",
    index: 0,
    lockup: 189_000_000,
    value: null,
    revealed: false,
    win: null,
    reveal: null,
    time: null,
    mine: true,
    myValue: 11_000_000,
    ...over,
  };
}

beforeEach(() => invokeMock.mockReset());

describe("NameBidsPanel", () => {
  it("lists a bid of ours still waiting for a block, and counts it apart", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "read_name_bids"
        ? Promise.resolve({
            name: "vmp3rt3",
            state: null,
            highest: null,
            value: null,
            bids: [
              bid(),
              bid({
                txid: "inflight",
                lockup: 295_000_000,
                myValue: 5_000_000,
                pending: true,
              }),
            ],
            myBidCount: 2,
          })
        : Promise.resolve(null),
    );

    render(<NameBidsPanel name="vmp3rt3" profileId="p1" phase="BIDDING" />, {
      wrapper: wrapper(),
    });

    const pending = await screen.findByTestId("name-bid-row-pending");
    // Full precision, matching the panel's other rows.
    expect(pending).toHaveTextContent("lockup: 295.000000 HNS");
    expect(pending).toHaveTextContent("your bid: 5.000000 HNS");
    expect(pending).toHaveTextContent(/waiting for a block/i);

    // The headline count is what the CHAIN has — one — with ours called out
    // separately rather than folded in as if it were already on-chain.
    const panel = screen.getByTestId("name-bids");
    expect(panel).toHaveTextContent("1 bids so far");
    expect(panel).toHaveTextContent("1 of yours waiting for a block");
  });

  it("says nothing about waiting when every bid is on-chain", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "read_name_bids"
        ? Promise.resolve({
            name: "vmp3rt3",
            state: null,
            highest: null,
            value: null,
            bids: [bid()],
            myBidCount: 1,
          })
        : Promise.resolve(null),
    );

    render(<NameBidsPanel name="vmp3rt3" profileId="p1" phase="BIDDING" />, {
      wrapper: wrapper(),
    });

    const panel = await screen.findByTestId("name-bids");
    expect(panel).toHaveTextContent("1 bids so far");
    expect(panel).not.toHaveTextContent(/waiting for a block/i);
    expect(screen.queryByTestId("name-bid-row-pending")).toBeNull();
  });
});
