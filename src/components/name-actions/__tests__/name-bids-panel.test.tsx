import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { NameBidsPanel } from "../NameBidsPanel";

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("NameBidsPanel — phase-aware honest bid display (Task 2)", () => {
  it("BIDDING: shows bid count + own bid count, explains lockup on hover, hides a competitor's true value, shows own plaintext bid + You badge", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "read_name_bids") {
        return Promise.resolve({
          name: "namehold",
          state: "BIDDING",
          highest: null,
          value: null,
          bids: [
            {
              txid: "a1",
              index: 0,
              lockup: 660000,
              value: 0,
              revealed: false,
              win: null,
              reveal: null,
              time: null,
              mine: false,
              myValue: null,
            },
            {
              txid: "a2",
              index: 0,
              lockup: 2000000,
              value: null,
              revealed: false,
              win: null,
              reveal: null,
              time: null,
              mine: true,
              myValue: 13000,
            },
          ],
          myBidCount: 1,
        });
      }
      return Promise.resolve(null);
    });

    render(<NameBidsPanel name="namehold" profileId="p1" phase="BIDDING" />, {
      wrapper: wrapper(),
    });

    expect(await screen.findByText(/2 bids/i)).toBeInTheDocument();
    expect(screen.getByText(/yours: 1/i)).toBeInTheDocument();

    // Lockups shown. Matched on the panel rather than a single element: the
    // word "lockup" is its own tooltip trigger now, so the figure beside it
    // lives in a sibling text node.
    const panel = screen.getByTestId("name-bids");
    expect(panel).toHaveTextContent("lockup: 0.660000 HNS");
    expect(panel).toHaveTextContent("lockup: 2.000000 HNS");
    // The caveat used to be inline on every row as "(max, not the actual bid)" —
    // the panel's most repeated string, and nothing a Vickrey bidder needs told
    // twice. It hangs off the word now.
    expect(screen.queryByText(/not the actual bid/i)).toBeNull();
    fireEvent.mouseEnter(screen.getAllByText("lockup")[0]!);
    await waitFor(() =>
      expect(screen.getByRole("tooltip")).toHaveTextContent(/sealed until the reveal phase/i),
    );

    // The competitor's `value` (0) must NEVER be rendered as their bid.
    expect(screen.queryByText(/bid: 0\.000000 HNS/i)).not.toBeInTheDocument();

    // Our own plaintext bid IS shown, plaintext, with a You badge.
    expect(screen.getByText(/your bid: 0\.013000 HNS/i)).toBeInTheDocument();
    expect(screen.getByText("You")).toBeInTheDocument();
  });

  it("REVEAL/CLOSED: shows revealed values, Winner badge on the winning bid, You badge on mine, and the top-level High bid", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "read_name_bids") {
        return Promise.resolve({
          name: "namehold",
          state: "REVEAL",
          highest: 20000,
          value: null,
          bids: [
            {
              txid: "b1",
              index: 0,
              lockup: 500000,
              value: 13000,
              revealed: true,
              win: false,
              reveal: null,
              time: null,
              mine: false,
              myValue: null,
            },
            {
              txid: "b2",
              index: 0,
              lockup: 2000000,
              value: 20000,
              revealed: true,
              win: true,
              reveal: null,
              time: null,
              mine: true,
              myValue: 20000,
            },
          ],
          myBidCount: 1,
        });
      }
      return Promise.resolve(null);
    });

    render(<NameBidsPanel name="namehold" profileId="p1" phase="REVEAL" />, {
      wrapper: wrapper(),
    });

    expect(await screen.findByText(/bid: 0\.013000 HNS/i)).toBeInTheDocument();
    expect(
      screen.getByText((_, el) => el?.textContent === "bid: 0.020000 HNS"),
    ).toBeInTheDocument();
    expect(screen.getByText("Winner")).toBeInTheDocument();
    expect(screen.getByText("You")).toBeInTheDocument();
    expect(screen.getByText(/High bid: 0\.020000 HNS/i)).toBeInTheDocument();
  });

  it("empty bids ([]) never crash and render either nothing or a muted no-bids line", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "read_name_bids") {
        return Promise.resolve({
          name: "emptyname",
          state: "BIDDING",
          highest: null,
          value: null,
          bids: [],
          myBidCount: 0,
        });
      }
      return Promise.resolve(null);
    });

    render(<NameBidsPanel name="emptyname" profileId="p1" phase="BIDDING" />, {
      wrapper: wrapper(),
    });

    // Give the query a tick to resolve.
    await new Promise((r) => setTimeout(r, 0));

    const testEl = screen.queryByTestId("name-bids");
    if (testEl) {
      expect(testEl.textContent).toMatch(/no bids/i);
    }
    // No crash either way — reaching this point is the assertion.
  });

  it("null bids response never crashes the panel", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "read_name_bids") return Promise.resolve(null);
      return Promise.resolve(null);
    });

    render(<NameBidsPanel name="unknownname" profileId="p1" phase="BIDDING" />, {
      wrapper: wrapper(),
    });

    await new Promise((r) => setTimeout(r, 0));

    const testEl = screen.queryByTestId("name-bids");
    if (testEl) {
      expect(testEl.textContent).toMatch(/no bids/i);
    }
  });

  it("invokes read_name_bids with the RAW punycode name and renders the decoded Unicode form", async () => {
    // "xn--e1adigm" decodes to "козел".
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "read_name_bids") {
        return Promise.resolve({
          name: "xn--e1adigm",
          state: "BIDDING",
          highest: null,
          value: null,
          bids: [
            {
              txid: "c1",
              index: 0,
              lockup: 1000000,
              value: 0,
              revealed: false,
              win: null,
              reveal: null,
              time: null,
              mine: false,
              myValue: null,
            },
          ],
          myBidCount: 0,
        });
      }
      return Promise.resolve(null);
    });

    render(<NameBidsPanel name="xn--e1adigm" profileId="p1" phase="BIDDING" />, {
      wrapper: wrapper(),
    });

    await screen.findByTestId("name-bids");

    const call = invokeMock.mock.calls.find((c) => c[0] === "read_name_bids");
    expect(call?.[1]).toMatchObject({ name: "xn--e1adigm" });

    // The panel's heading used to read "Bids for козел". The modal's own title
    // already names the domain, so the heading is bare now and this component
    // renders the name nowhere — the RAW-name call above is what it owes.
    expect(screen.getByTestId("name-bids")).toHaveTextContent("Bids");
    expect(screen.queryByText(/Bids for/i)).toBeNull();
  });

  it("lists a bid of ours still waiting for a block, and counts it apart", async () => {
    // The list is fed by the chain scanner's index, which only holds BID
    // outputs found in BLOCKS. A bid just sent sits in the mempool, so it used
    // to be absent entirely: place a second bid and the panel still read
    // "1 bids so far", as if the transaction had never happened.
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "read_name_bids"
        ? Promise.resolve({
            name: "vmp3rt3",
            state: null,
            highest: null,
            value: null,
            bids: [
              {
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
              },
              {
                txid: "inflight",
                index: null,
                lockup: 372_000_000,
                value: null,
                revealed: false,
                win: null,
                reveal: null,
                time: null,
                mine: true,
                myValue: 12_000_000,
                pending: true,
              },
            ],
            myBidCount: 2,
          })
        : Promise.resolve(null),
    );

    render(<NameBidsPanel name="vmp3rt3" profileId="p1" phase="BIDDING" />, {
      wrapper: wrapper(),
    });

    const pending = await screen.findByTestId("name-bid-row-pending");
    expect(pending).toHaveTextContent("lockup: 372.000000 HNS");
    expect(pending).toHaveTextContent("your bid: 12.000000 HNS");
    expect(pending).toHaveTextContent(/waiting for a block/i);

    // The headline count is what the CHAIN has; ours is called out beside it
    // rather than folded in as though it were already on-chain.
    const panel = screen.getByTestId("name-bids");
    expect(panel).toHaveTextContent("1 bids so far");
    expect(panel).toHaveTextContent("1 of yours waiting for a block");

    // "You" ends its row, so it lands in the same column on every row instead
    // of drifting with whatever figures precede it.
    expect(pending.textContent?.trimEnd().endsWith("You")).toBe(true);
  });
});
