import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { BatchBidModal } from "../BatchBidModal";
import * as walletQueries from "../../queries/wallet";

// Mock the wallet queries
vi.mock("../../queries/wallet", () => ({
  useNameAction: vi.fn(),
  useActiveProfile: vi.fn(),
}));

const showToast = vi.fn();
vi.mock("../../stores/ui", () => ({
  useUiStore: vi.fn((selector) => selector({ showToast })),
}));

describe("BatchBidModal", () => {
  const mutateAsync = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(walletQueries.useNameAction).mockReturnValue({
      mutateAsync,
      isPending: false,
    } as any);
    vi.mocked(walletQueries.useActiveProfile).mockReturnValue({
      data: { id: "p1", network: "mainnet" },
    } as any);
  });

  it("does not render when closed", () => {
    render(<BatchBidModal open={false} onClose={vi.fn()} />);
    expect(screen.queryByText("Batch Bid")).not.toBeInTheDocument();
  });

  it("renders when open with inputs", () => {
    render(<BatchBidModal open={true} onClose={vi.fn()} />);
    expect(screen.getByText("Batch Bid")).toBeInTheDocument();
    expect(screen.getByTestId("batch-bid-names-input")).toBeInTheDocument();
    expect(screen.getByTestId("batch-bid-value-input")).toBeInTheDocument();
    expect(screen.getByTestId("batch-bid-lockup-input")).toBeInTheDocument();
  });

  it("submit button disabled when form is incomplete", () => {
    render(<BatchBidModal open={true} onClose={vi.fn()} />);
    const btn = screen.getByTestId("batch-bid-submit-btn");
    expect(btn).toBeDisabled();
  });

  it("calls mutateAsync with correct args on submit", async () => {
    mutateAsync.mockResolvedValue({ id: "draft1", summary: { feeDoos: 500 } });
    const onClose = vi.fn();
    render(<BatchBidModal open={true} onClose={onClose} />);

    // Fill names
    fireEvent.change(screen.getByTestId("batch-bid-names-input"), {
      target: { value: "name1\nname2" },
    });
    // Fill bid and lockup (1 HNS = 1_000_000 doos)
    fireEvent.change(screen.getByTestId("batch-bid-value-input"), {
      target: { value: "1" },
    });
    fireEvent.change(screen.getByTestId("batch-bid-lockup-input"), {
      target: { value: "2" },
    });

    const btn = screen.getByTestId("batch-bid-submit-btn");
    expect(btn).not.toBeDisabled();
    fireEvent.click(btn);

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalledWith({
        names: ["name1", "name2"],
        bidValue: 1_000_000,
        lockup: 2_000_000,
        feeRate: undefined,
      });
    });
    await waitFor(() => {
      expect(onClose).toHaveBeenCalled();
    });
  });

  it("shows a mapped (humanized) error toast when the mutation rejects", async () => {
    // A raw AppError string containing a known ERROR_MAP pattern; mapError
    // should convert it to the friendly copy rather than interpolating the
    // raw object.
    mutateAsync.mockRejectedValue(new Error("insufficient funds"));
    render(<BatchBidModal open={true} onClose={vi.fn()} />);

    fireEvent.change(screen.getByTestId("batch-bid-names-input"), {
      target: { value: "name1" },
    });
    fireEvent.change(screen.getByTestId("batch-bid-value-input"), {
      target: { value: "1" },
    });
    fireEvent.change(screen.getByTestId("batch-bid-lockup-input"), {
      target: { value: "2" },
    });
    fireEvent.click(screen.getByTestId("batch-bid-submit-btn"));

    await waitFor(() => {
      expect(showToast).toHaveBeenCalledWith(
        "Build failed: Insufficient HNS balance for this transaction.",
        "error",
      );
    });
    // And never the raw interpolated form.
    expect(showToast).not.toHaveBeenCalledWith(
      expect.stringContaining("[object Object]"),
      "error",
    );
  });
});
