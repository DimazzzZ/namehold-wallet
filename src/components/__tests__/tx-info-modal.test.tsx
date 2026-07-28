import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { TxInfoModal } from "../TxInfoModal";
import { useReadTxInfo } from "../../queries/read";
import { useNodeLive } from "../../queries/node";
import type { TxInfo } from "../../types";

vi.mock("../../queries/read");
vi.mock("../../queries/node");

const mockUseReadTxInfo = vi.mocked(useReadTxInfo);
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

const TXID = "a1b2c3d4e5f60718293a4b5c6d7e8f90112233445566778899aabbccddeeff00";

const CONFIRMED_TX: TxInfo = {
  txid: TXID,
  confirmations: 12,
  height: 200_000,
  block: "0000000000000000abcdef",
  time: 1700000000,
  fee: 20_000, // doos
  inputsCount: 2,
  outputsCount: 3,
  totalOut: 5_000_000_000, // 5000 HNS
};

const PENDING_TX: TxInfo = {
  txid: TXID,
  confirmations: 0,
  height: -1,
  block: null,
  time: 0,
  fee: 20_000,
  inputsCount: 1,
  outputsCount: 2,
  totalOut: 1_000_000,
};

beforeEach(() => {
  vi.clearAllMocks();
  mockUseReadTxInfo.mockReturnValue({
    data: null,
    isLoading: false,
    isError: false,
  } as any);
  mockUseNodeLive.mockReturnValue(true);
});

describe("TxInfoModal", () => {
  it("renders all standard fields for a confirmed tx", () => {
    mockUseReadTxInfo.mockReturnValue({
      data: CONFIRMED_TX,
      isLoading: false,
      isError: false,
    } as any);

    render(<TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet />, {
      wrapper: wrapper(),
    });

    // Full txid shown
    expect(screen.getByTestId("tx-hash").textContent).toContain(TXID);
    // Status = Confirmed
    expect(screen.getByTestId("tx-status").textContent).toContain("Confirmed");
    // Confirmations
    expect(screen.getByTestId("tx-confirmations").textContent).toContain("12");
    // Block height
    expect(screen.getByTestId("tx-height").textContent).toContain("200,000");
    // Timestamp present
    expect(screen.getByTestId("tx-time")).toBeInTheDocument();
    // Fee, inputs, outputs, total out present
    expect(screen.getByTestId("tx-fee")).toBeInTheDocument();
    expect(screen.getByTestId("tx-inputs").textContent).toContain("2");
    expect(screen.getByTestId("tx-outputs").textContent).toContain("3");
    expect(screen.getByTestId("tx-total-out")).toBeInTheDocument();
  });

  it("shows 'Pending' status and dashes for height/time when unconfirmed", () => {
    mockUseReadTxInfo.mockReturnValue({
      data: PENDING_TX,
      isLoading: false,
      isError: false,
    } as any);

    render(<TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet={false} />, {
      wrapper: wrapper(),
    });

    expect(screen.getByTestId("tx-status").textContent).toContain("Pending");
    // height -1 → em dash
    expect(screen.getByTestId("tx-height").textContent).toBe("—");
    // time 0 → em dash
    expect(screen.getByTestId("tx-time").textContent).toBe("—");
    expect(screen.getByTestId("tx-confirmations").textContent).toContain("0");
  });

  it("shows 'requires synced node' when node is not live", () => {
    mockUseNodeLive.mockReturnValue(false);

    render(<TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet={false} />, {
      wrapper: wrapper(),
    });

    expect(screen.getByTestId("tx-info-no-node")).toBeInTheDocument();
    expect(screen.getByText(/Requires a synced local node/)).toBeInTheDocument();
  });

  it("shows 'requires synced node' when tx data is null (unknown tx / node down)", () => {
    mockUseNodeLive.mockReturnValue(true);
    mockUseReadTxInfo.mockReturnValue({
      data: null,
      isLoading: false,
      isError: false,
    } as any);

    render(<TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet />, {
      wrapper: wrapper(),
    });

    expect(screen.getByTestId("tx-info-no-node")).toBeInTheDocument();
  });

  it("renders explorer link only when isMainnet is true", () => {
    mockUseReadTxInfo.mockReturnValue({
      data: CONFIRMED_TX,
      isLoading: false,
      isError: false,
    } as any);

    const { unmount } = render(
      <TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet />,
      { wrapper: wrapper() },
    );
    expect(screen.getByTestId("tx-explorer-link")).toBeInTheDocument();
    unmount();

    render(<TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet={false} />, {
      wrapper: wrapper(),
    });
    expect(screen.queryByTestId("tx-explorer-link")).not.toBeInTheDocument();
  });

  it("does not render when open is false", () => {
    mockUseReadTxInfo.mockReturnValue({
      data: CONFIRMED_TX,
      isLoading: false,
      isError: false,
    } as any);

    const { container } = render(
      <TxInfoModal txid={TXID} open={false} onClose={vi.fn()} isMainnet />,
      { wrapper: wrapper() },
    );
    expect(container.innerHTML).toBe("");
  });

  it("shows loading spinner while fetching", () => {
    mockUseReadTxInfo.mockReturnValue({
      data: null,
      isLoading: true,
      isError: false,
    } as any);

    render(<TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet />, {
      wrapper: wrapper(),
    });
    expect(screen.getByText("Loading transaction info...")).toBeInTheDocument();
  });

  it("renders '—' when fee is null (coinbase / unresolved inputs)", () => {
    // Coinbase txs and any hsd response with unresolved input coins yield
    // fee=null; the modal must NOT show "0" — that was the original bug.
    mockUseReadTxInfo.mockReturnValue({
      data: { ...CONFIRMED_TX, fee: null },
      isLoading: false,
      isError: false,
    } as any);

    render(<TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet />, {
      wrapper: wrapper(),
    });

    expect(screen.getByTestId("tx-fee").textContent).toBe("—");
    // Belt-and-braces: nothing that looks like a zero-fee reading.
    expect(screen.getByTestId("tx-fee").textContent).not.toContain("0.000000");
  });

  it("renders totalOut as raw doos (regression pin for the ×1,000,000 bug)", () => {
    // 100_000_000 doos = 100 HNS. The old code multiplied by 1e6, producing
    // 1e14 which formatHns rendered as some huge nonsense HNS value.
    // formatHns(100_000_000) should render "100 HNS" (or the app's current
    // display idiom for exactly 100 HNS). Assert that the total-out text
    // starts with "100" and NOT with a bogus large-number prefix.
    const hundredHnsInDoos = 100_000_000;
    mockUseReadTxInfo.mockReturnValue({
      data: { ...CONFIRMED_TX, totalOut: hundredHnsInDoos },
      isLoading: false,
      isError: false,
    } as any);

    render(<TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet />, {
      wrapper: wrapper(),
    });

    const text = screen.getByTestId("tx-total-out").textContent ?? "";
    // Real: starts with "100" (100 HNS). Bug: would start with "100,000,000"
    // or "1e14" territory.
    expect(text).toMatch(/^100(\s|\.|\D)/);
    expect(text).not.toContain("1,000,000");
    expect(text).not.toContain("100,000,000");
  });

  it("shows 'tx index required' hint when backend returns tx_index_disabled error", () => {
    // When the node lacks --index-tx, the backend returns { error: "tx_index_disabled" }
    // rather than null. The modal must show the targeted hint, NOT the generic
    // "requires synced node" message, and NOT any tx content rows.
    mockUseReadTxInfo.mockReturnValue({
      data: { error: "tx_index_disabled" },
      isLoading: false,
      isError: false,
    } as any);

    render(<TxInfoModal txid={TXID} open onClose={vi.fn()} isMainnet />, {
      wrapper: wrapper(),
    });

    // The index-disabled hint renders.
    expect(screen.getByTestId("tx-info-index-disabled")).toBeInTheDocument();
    expect(screen.getByText(/--index-tx/)).toBeInTheDocument();
    // The generic "requires synced node" does NOT render.
    expect(screen.queryByTestId("tx-info-no-node")).not.toBeInTheDocument();
    // No tx content rows render.
    expect(screen.queryByTestId("tx-hash")).not.toBeInTheDocument();
    expect(screen.queryByTestId("tx-fee")).not.toBeInTheDocument();
  });
});
