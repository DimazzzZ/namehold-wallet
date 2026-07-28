import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { BlockInfoModal } from "../BlockInfoModal";
import { useReadBlockInfo } from "../../queries/read";
import { useNodeLive } from "../../queries/node";
import type { BlockInfo } from "../../types";

vi.mock("../../queries/read");
vi.mock("../../queries/node");

const mockUseReadBlockInfo = vi.mocked(useReadBlockInfo);
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

const SAMPLE_BLOCK: BlockInfo = {
  height: 200_000,
  hash: "00000000000000003a3c2e8f7e4e1d0b9a8c7f6e5d4c3b2a1f0e9d8c7b6a5f4e",
  time: 1700000000,
  txCount: 42,
  minerReward: 2000_000_000, // 2000 HNS in doos
  difficulty: 123456.7891,
};

beforeEach(() => {
  vi.clearAllMocks();
  mockUseReadBlockInfo.mockReturnValue({
    data: null,
    isLoading: false,
    isError: false,
  } as any);
  mockUseNodeLive.mockReturnValue(true);
});

describe("BlockInfoModal", () => {
  it("renders all standard fields when block data is available", () => {
    mockUseReadBlockInfo.mockReturnValue({
      data: SAMPLE_BLOCK,
      isLoading: false,
      isError: false,
    } as any);

    render(
      <BlockInfoModal height={200_000} open onClose={vi.fn()} isMainnet />,
      { wrapper: wrapper() },
    );

    // Hash
    expect(screen.getByTestId("block-hash").textContent).toContain(
      "00000000000000003a3c2e8f7e4e1d0b9a8c7f6e5d4c3b2a1f0e9d8c7b6a5f4e",
    );
    // Tx count
    expect(screen.getByTestId("block-tx-count").textContent).toContain("42");
    // Miner reward (formatted as HNS)
    expect(screen.getByTestId("block-miner-reward")).toBeInTheDocument();
    // Difficulty
    expect(screen.getByTestId("block-difficulty")).toBeInTheDocument();
    // Timestamp
    expect(screen.getByTestId("block-time")).toBeInTheDocument();
  });

  it("shows 'requires synced node' when node is not live", () => {
    mockUseNodeLive.mockReturnValue(false);

    render(
      <BlockInfoModal height={100} open onClose={vi.fn()} isMainnet={false} />,
      { wrapper: wrapper() },
    );

    expect(screen.getByTestId("block-info-no-node")).toBeInTheDocument();
    expect(
      screen.getByText(/Requires a synced local node/),
    ).toBeInTheDocument();
  });

  it("shows 'requires synced node' when block data is null (node returned null)", () => {
    mockUseNodeLive.mockReturnValue(true);
    mockUseReadBlockInfo.mockReturnValue({
      data: null,
      isLoading: false,
      isError: false,
    } as any);

    render(
      <BlockInfoModal height={100} open onClose={vi.fn()} isMainnet={false} />,
      { wrapper: wrapper() },
    );

    expect(screen.getByTestId("block-info-no-node")).toBeInTheDocument();
  });

  it("renders explorer link only when isMainnet is true", () => {
    mockUseReadBlockInfo.mockReturnValue({
      data: SAMPLE_BLOCK,
      isLoading: false,
      isError: false,
    } as any);

    // Mainnet: link present
    const { unmount } = render(
      <BlockInfoModal height={200_000} open onClose={vi.fn()} isMainnet />,
      { wrapper: wrapper() },
    );
    expect(screen.getByTestId("block-explorer-link")).toBeInTheDocument();
    unmount();

    // Non-mainnet: link absent
    render(
      <BlockInfoModal height={200_000} open onClose={vi.fn()} isMainnet={false} />,
      { wrapper: wrapper() },
    );
    expect(screen.queryByTestId("block-explorer-link")).not.toBeInTheDocument();
  });

  it("does not render when open is false", () => {
    mockUseReadBlockInfo.mockReturnValue({
      data: SAMPLE_BLOCK,
      isLoading: false,
      isError: false,
    } as any);

    const { container } = render(
      <BlockInfoModal height={200_000} open={false} onClose={vi.fn()} isMainnet />,
      { wrapper: wrapper() },
    );
    expect(container.innerHTML).toBe("");
  });

  it("shows loading spinner while fetching", () => {
    mockUseReadBlockInfo.mockReturnValue({
      data: null,
      isLoading: true,
      isError: false,
    } as any);

    render(
      <BlockInfoModal height={100} open onClose={vi.fn()} isMainnet />,
      { wrapper: wrapper() },
    );
    expect(screen.getByText("Loading block info...")).toBeInTheDocument();
  });
});
