import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ReceiveAddressList } from "../ReceiveAddressList";
import * as readQueries from "../../queries/read";
import * as walletQueries from "../../queries/wallet";
import * as errors from "../../lib/errors";
import type { ReceiveAddressRow } from "../../types";

// Mock the queries
vi.mock("../../queries/read", () => ({
  useReceiveAddresses: vi.fn(),
  useDeriveNextReceiveAddress: vi.fn(),
}));

vi.mock("../../queries/wallet", () => ({
  useActiveProfile: vi.fn(),
}));

vi.mock("../../lib/clipboard", () => ({
  writeText: vi.fn(),
}));

vi.mock("../../lib/errors", () => ({
  mapError: vi.fn((e) => `mapped: ${e instanceof Error ? e.message : String(e)}`),
}));

vi.mock("../../stores/ui", () => ({
  useUiStore: vi.fn((selector) => {
    const store = {
      showToast: vi.fn(),
    };
    return selector(store);
  }),
}));

describe("ReceiveAddressList", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows loading state initially", () => {
    vi.mocked(readQueries.useReceiveAddresses).mockReturnValue({
      data: undefined,
      isLoading: true,
    } as any);
    vi.mocked(walletQueries.useActiveProfile).mockReturnValue({
      data: { id: "p1", network: "mainnet" },
    } as any);
    vi.mocked(readQueries.useDeriveNextReceiveAddress).mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    } as any);

    render(<ReceiveAddressList />);
    expect(screen.getByText("Loading addresses…")).toBeInTheDocument();
  });

  it("shows empty state when no addresses exist", () => {
    vi.mocked(readQueries.useReceiveAddresses).mockReturnValue({
      data: [],
      isLoading: false,
    } as any);
    vi.mocked(walletQueries.useActiveProfile).mockReturnValue({
      data: { id: "p1", network: "mainnet" },
    } as any);
    vi.mocked(readQueries.useDeriveNextReceiveAddress).mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    } as any);

    render(<ReceiveAddressList />);
    expect(
      screen.getByText("No addresses derived yet. Generate one or run a sync.")
    ).toBeInTheDocument();
  });

  it("renders address rows with index, address, and used badge", () => {
    const rows: ReceiveAddressRow[] = [
      { index: 0, address: "rs1qrecv0", used: true, firstSeenAt: "2026-01-01T10:00:00" },
      { index: 1, address: "rs1qrecv1", used: false, firstSeenAt: "2026-01-01T10:01:00" },
    ];
    vi.mocked(readQueries.useReceiveAddresses).mockReturnValue({
      data: rows,
      isLoading: false,
    } as any);
    vi.mocked(walletQueries.useActiveProfile).mockReturnValue({
      data: { id: "p1", network: "mainnet" },
    } as any);
    vi.mocked(readQueries.useDeriveNextReceiveAddress).mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    } as any);

    render(<ReceiveAddressList />);
    expect(screen.getByTestId("addr-row-0")).toBeInTheDocument();
    expect(screen.getByTestId("addr-row-1")).toBeInTheDocument();
    expect(screen.getByText("used")).toBeInTheDocument();
    expect(screen.getByText("fresh")).toBeInTheDocument();
  });

  it("toggles QR display on QR button click", () => {
    const rows: ReceiveAddressRow[] = [
      { index: 0, address: "rs1qrecv0", used: false, firstSeenAt: "2026-01-01T10:00:00" },
    ];
    vi.mocked(readQueries.useReceiveAddresses).mockReturnValue({
      data: rows,
      isLoading: false,
    } as any);
    vi.mocked(walletQueries.useActiveProfile).mockReturnValue({
      data: { id: "p1", network: "mainnet" },
    } as any);
    vi.mocked(readQueries.useDeriveNextReceiveAddress).mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    } as any);

    render(<ReceiveAddressList />);
    expect(screen.queryByTestId("qr-display-0")).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId("qr-btn-0"));
    expect(screen.getByTestId("qr-display-0")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("qr-btn-0"));
    expect(screen.queryByTestId("qr-display-0")).not.toBeInTheDocument();
  });

  it("calls derive mutation on generate button click", async () => {
    const mutateAsync = vi.fn().mockResolvedValue("rs1qnew");
    vi.mocked(readQueries.useReceiveAddresses).mockReturnValue({
      data: [],
      isLoading: false,
    } as any);
    vi.mocked(walletQueries.useActiveProfile).mockReturnValue({
      data: { id: "p1", network: "mainnet" },
    } as any);
    vi.mocked(readQueries.useDeriveNextReceiveAddress).mockReturnValue({
      mutateAsync,
      isPending: false,
    } as any);

    render(<ReceiveAddressList />);
    const btn = screen.getByTestId("derive-next-address-btn");
    fireEvent.click(btn);

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({ walletProfileId: "p1" });
    });
  });

  it("routes derive failures through mapError instead of raw interpolation", async () => {
    const mutateAsync = vi.fn().mockRejectedValue(new Error("boom"));
    vi.mocked(readQueries.useReceiveAddresses).mockReturnValue({
      data: [],
      isLoading: false,
    } as any);
    vi.mocked(walletQueries.useActiveProfile).mockReturnValue({
      data: { id: "p1", network: "mainnet" },
    } as any);
    vi.mocked(readQueries.useDeriveNextReceiveAddress).mockReturnValue({
      mutateAsync,
      isPending: false,
    } as any);

    render(<ReceiveAddressList />);
    fireEvent.click(screen.getByTestId("derive-next-address-btn"));

    await waitFor(() => {
      expect(errors.mapError).toHaveBeenCalled();
    });
    // Assert we don't fall back to raw `${e}` interpolation.
    const call = vi.mocked(errors.mapError).mock.calls[0]!;
    expect(call[1]).toBe("build");
  });
});
