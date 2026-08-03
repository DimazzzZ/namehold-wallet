import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { NameInfoModal } from "../NameInfoModal";
import { useReadNameInfo, useNameBids, useNameRecords } from "../../queries/read";
import { useActiveProfile } from "../../queries/wallet";
import { useNodeLive } from "../../queries/node";
import type { ChainName, NameBids } from "../../types";

// Mock the query hooks
vi.mock("../../queries/read");
vi.mock("../../queries/wallet");
vi.mock("../../queries/node");

const mockUseReadNameInfo = vi.mocked(useReadNameInfo);
const mockUseNameBids = vi.mocked(useNameBids);
const mockUseNameRecords = vi.mocked(useNameRecords);
const mockUseActiveProfile = vi.mocked(useActiveProfile);
const mockUseNodeLive = vi.mocked(useNodeLive);

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockUseReadNameInfo.mockReturnValue({
    data: null,
    isLoading: false,
    isError: false,
  } as any);
  mockUseNameBids.mockReturnValue({
    data: null,
    isLoading: false,
    isError: false,
  } as any);
  mockUseNameRecords.mockReturnValue({
    data: { records: [] },
    isLoading: false,
    isError: false,
  } as any);
  mockUseActiveProfile.mockReturnValue({
    data: { id: "p1" },
    isLoading: false,
    isError: false,
  } as any);
  mockUseNodeLive.mockReturnValue(true);
});

describe("NameInfoModal", () => {
  it("renders state badge and countdown for a BIDDING name", () => {
    const nameInfo: ChainName = {
      name: "example",
      state: "BIDDING",
      registered: false,
      expired: false,
      height: 100,
      renewal: 200,
      owner: { hash: "abc", index: 0 },
      value: null,
      highest: null,
      stats: {
        blocksUntilReveal: 144,
        hoursUntilReveal: 24,
      } as any,
      transfer: 0,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);

    render(<NameInfoModal name="example" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Should show the state badge
    expect(screen.getByText("Bidding")).toBeInTheDocument();

    // Should show the countdown
    expect(screen.getByTestId("name-info-countdown")).toBeInTheDocument();
    expect(screen.getByTestId("name-info-countdown").textContent).toContain("Reveal starts in");
  });

  it("does NOT show 'Expired' badge during an active auction phase (REVEAL + expired=true)", () => {
    // A name whose PREVIOUS registration lapsed but that's currently going
    // through a new auction (REVEAL). hsrd reports `expired: true` on the
    // stale prior state; showing that badge alongside "Reveal" confuses
    // users into thinking the current auction is expired. Suppressed.
    const nameInfo: ChainName = {
      name: "namehold",
      state: "REVEAL",
      registered: false,
      expired: true,
      height: 100,
      renewal: 200,
      owner: { hash: "abc", index: 0 },
      value: null,
      highest: null,
      stats: {
        blocksUntilClose: 100,
        hoursUntilClose: 16,
      } as any,
      transfer: 0,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);

    render(<NameInfoModal name="namehold" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // The state badge is shown...
    expect(screen.getByText("Reveal")).toBeInTheDocument();
    // ...but the misleading "Expired" badge is suppressed during REVEAL.
    expect(screen.queryByText("Expired")).not.toBeInTheDocument();
  });

  it("renders Paid price + Top bid for a CLOSED name", () => {
    const nameInfo: ChainName = {
      name: "example",
      state: "CLOSED",
      registered: true,
      expired: false,
      height: 100,
      renewal: 200,
      owner: { hash: "abc", index: 0 },
      value: 1_000_000,
      highest: 2_000_000,
      stats: null,
      transfer: 0,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);

    render(<NameInfoModal name="example" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Should show both badges
    expect(screen.getByText("Closed")).toBeInTheDocument();
    expect(screen.getByText("Registered")).toBeInTheDocument();

    // Should show paid price and top bid
    expect(screen.getByText(/Paid price/)).toBeInTheDocument();
    expect(screen.getByText(/Top bid/)).toBeInTheDocument();
  });

  it("hides owner UTXO row when owner is null", () => {
    const nameInfo: ChainName = {
      name: "example",
      state: "AVAILABLE",
      registered: false,
      expired: false,
      height: null,
      renewal: null,
      owner: null,
      value: null,
      highest: null,
      stats: null,
      transfer: 0,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);

    render(<NameInfoModal name="example" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Owner UTXO row should not appear
    expect(screen.queryByText("Owner UTXO:")).not.toBeInTheDocument();
  });

  it("shows transfer status when transfer is non-zero", () => {
    const nameInfo: ChainName = {
      name: "example",
      state: "TRANSFER",
      registered: true,
      expired: false,
      height: 100,
      renewal: 200,
      owner: { hash: "abc", index: 0 },
      value: null,
      highest: null,
      stats: null,
      transfer: 500,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);

    render(<NameInfoModal name="example" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Should show transfer status
    expect(screen.getByText(/Transfer in progress/)).toBeInTheDocument();
  });

  it("renders DNS records with TTL when node is live", () => {
    const nameInfo: ChainName = {
      name: "example",
      state: "CLOSED",
      registered: true,
      expired: false,
      height: 100,
      renewal: 200,
      owner: { hash: "abc", index: 0 },
      value: null,
      highest: null,
      stats: null,
      transfer: 0,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);
    mockUseNameRecords.mockReturnValue({
      data: {
        records: [
          { type: "NS", ns: "ns1.example." },
          { type: "TXT", txt: ["v=spf1 -all"] },
        ],
        ttl: 3600,
      },
      isLoading: false,
      isError: false,
    } as any);
    mockUseNodeLive.mockReturnValue(true);

    render(<NameInfoModal name="example" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Should show DNS records section
    expect(screen.getByText("DNS Records")).toBeInTheDocument();
    expect(screen.getByText(/TTL:/)).toBeInTheDocument();
    expect(screen.getByText("3600s")).toBeInTheDocument();
  });

  it("shows 'Requires a synced node' when node is not live", () => {
    const nameInfo: ChainName = {
      name: "example",
      state: "CLOSED",
      registered: true,
      expired: false,
      height: 100,
      renewal: 200,
      owner: { hash: "abc", index: 0 },
      value: null,
      highest: null,
      stats: null,
      transfer: 0,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);
    mockUseNodeLive.mockReturnValue(false);

    render(<NameInfoModal name="example" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Should show the "requires synced node" message
    expect(screen.getByTestId("name-info-dns-no-node")).toBeInTheDocument();
    expect(screen.getByText(/Requires a synced local node/)).toBeInTheDocument();
  });

  it("renders explorer link", () => {
    const nameInfo: ChainName = {
      name: "example",
      state: "CLOSED",
      registered: true,
      expired: false,
      height: 100,
      renewal: 200,
      owner: { hash: "abc", index: 0 },
      value: null,
      highest: null,
      stats: null,
      transfer: 0,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);

    render(<NameInfoModal name="example" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Should have explorer link
    const link = screen.getByTestId("name-explorer-link");
    expect(link).toBeInTheDocument();
    expect(link.textContent).toContain("View on explorer");
  });

  it("passes raw (punycode) name to read_name_info", () => {
    const nameInfo: ChainName = {
      name: "xn--e1afmkfd.xn--p1ai",
      state: "CLOSED",
      registered: true,
      expired: false,
      height: 100,
      renewal: 200,
      owner: { hash: "abc", index: 0 },
      value: null,
      highest: null,
      stats: null,
      transfer: 0,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);

    render(<NameInfoModal name="xn--e1afmkfd.xn--p1ai" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Verify the hook was called with the raw name
    expect(mockUseReadNameInfo).toHaveBeenCalledWith("xn--e1afmkfd.xn--p1ai");
  });

  it("renders my bids when present", () => {
    const nameInfo: ChainName = {
      name: "example",
      state: "BIDDING",
      registered: false,
      expired: false,
      height: 100,
      renewal: 200,
      owner: { hash: "abc", index: 0 },
      value: null,
      highest: null,
      stats: null,
      transfer: 0,
    };
    const bids: NameBids = {
      name: "example",
      state: null,
      highest: null,
      value: null,
      bids: [
        {
          txid: "tx1",
          index: 0,
          lockup: 2_000_000,
          value: 1_000_000,
          revealed: false,
          win: false,
          reveal: null,
          time: null,
          mine: true,
          myValue: 1_000_000,
        },
        {
          txid: "tx2",
          index: 0,
          lockup: 1_500_000,
          value: 1_500_000,
          revealed: true,
          win: false,
          reveal: null,
          time: null,
          mine: true,
          myValue: 1_500_000,
        },
      ],
      myBidCount: 2,
    };
    mockUseReadNameInfo.mockReturnValue({ data: nameInfo, isLoading: false, isError: false } as any);
    mockUseNameBids.mockReturnValue({ data: bids, isLoading: false, isError: false } as any);

    render(<NameInfoModal name="example" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Should show bids section
    expect(screen.getByText(/Your bids/)).toBeInTheDocument();
    expect(screen.getByText(/2 · 1 revealed/)).toBeInTheDocument();
  });
});
