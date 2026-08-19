import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { BatchBidModal } from "../BatchBidModal";
import * as walletQueries from "../../queries/wallet";
import * as readQueries from "../../queries/read";

// Mock the wallet queries
vi.mock("../../queries/wallet", () => ({
  useNameAction: vi.fn(),
  useActiveProfile: vi.fn(),
  useSignerSession: vi.fn(),
  useUnlockSigner: vi.fn(),
  useSignTxDraft: vi.fn(),
  useBroadcastTxDraft: vi.fn(),
}));

vi.mock("../../queries/read", () => ({
  useNamesActionCapabilities: vi.fn(),
}));

// Mock @tanstack/react-query's useQueryClient (invalidateQueries).
const invalidateQueries = vi.fn();
vi.mock("@tanstack/react-query", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tanstack/react-query")>();
  return {
    ...actual,
    useQueryClient: () => ({ invalidateQueries }),
  };
});

const showToast = vi.fn();
vi.mock("../../stores/ui", () => ({
  useUiStore: vi.fn((selector) => selector({ showToast })),
}));

/** Build a minimal NameActionCapabilities stub for a name. */
function cap(name: string, canBid: boolean, reason: string | null = null) {
  return {
    name,
    phase: canBid ? "BIDDING" : "AVAILABLE",
    canBid: { allowed: canBid, reason },
    // Other fields aren't read by the modal; cast loosely below.
  } as any;
}

describe("BatchBidModal", () => {
  const buildMutateAsync = vi.fn();
  const unlockMutateAsync = vi.fn();
  const signMutateAsync = vi.fn();
  const broadcastMutateAsync = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(walletQueries.useNameAction).mockReturnValue({
      mutateAsync: buildMutateAsync,
      isPending: false,
    } as any);
    vi.mocked(walletQueries.useActiveProfile).mockReturnValue({
      data: { id: "p1", network: "mainnet" },
    } as any);
    vi.mocked(walletQueries.useSignerSession).mockReturnValue({
      data: { unlocked: true },
    } as any);
    vi.mocked(walletQueries.useUnlockSigner).mockReturnValue({
      mutateAsync: unlockMutateAsync,
    } as any);
    vi.mocked(walletQueries.useSignTxDraft).mockReturnValue({
      mutateAsync: signMutateAsync,
    } as any);
    vi.mocked(walletQueries.useBroadcastTxDraft).mockReturnValue({
      mutateAsync: broadcastMutateAsync,
    } as any);
    // Default: no names checked yet → empty caps.
    vi.mocked(readQueries.useNamesActionCapabilities).mockReturnValue({
      data: [],
      isFetching: false,
    } as any);
  });

  it("does not render when closed", () => {
    render(<BatchBidModal open={false} onClose={vi.fn()} />);
    expect(screen.queryByText("Batch Bid")).not.toBeInTheDocument();
  });

  it("renders input step when open", () => {
    render(<BatchBidModal open={true} onClose={vi.fn()} />);
    expect(screen.getByText("Batch Bid")).toBeInTheDocument();
    expect(screen.getByTestId("batch-bid-names-input")).toBeInTheDocument();
    expect(screen.getByTestId("batch-bid-value-input")).toBeInTheDocument();
    expect(screen.getByTestId("batch-bid-lockup-input")).toBeInTheDocument();
    expect(screen.getByTestId("batch-bid-check-names-btn")).toBeInTheDocument();
  });

  it("check-names button disabled when form is incomplete", () => {
    render(<BatchBidModal open={true} onClose={vi.fn()} />);
    expect(screen.getByTestId("batch-bid-check-names-btn")).toBeDisabled();
  });

  it("preflight shows biddable vs not-available with reason; build uses only biddable subset", async () => {
    // Once we advance to preflight, caps report name1 biddable, name2 not.
    vi.mocked(readQueries.useNamesActionCapabilities).mockReturnValue({
      data: [cap("name1", true), cap("name2", false, "Phase is AVAILABLE")],
      isFetching: false,
    } as any);
    buildMutateAsync.mockResolvedValue({ id: "draft1", summary: { feeDoos: 500 } });

    render(<BatchBidModal open={true} onClose={vi.fn()} activeProfileId="p1" />);

    fireEvent.change(screen.getByTestId("batch-bid-names-input"), {
      target: { value: "name1\nname2" },
    });
    fireEvent.change(screen.getByTestId("batch-bid-value-input"), {
      target: { value: "1" },
    });
    fireEvent.change(screen.getByTestId("batch-bid-lockup-input"), {
      target: { value: "2" },
    });

    fireEvent.click(screen.getByTestId("batch-bid-check-names-btn"));

    // Preflight shows both categories.
    await waitFor(() => {
      expect(screen.getByText(/Phase is AVAILABLE/i)).toBeInTheDocument();
    });
    expect(screen.getByText("✓ .name1")).toBeInTheDocument();
    expect(screen.getByText(/✗ .name2/)).toBeInTheDocument();

    // Build uses only the biddable subset (name1).
    fireEvent.click(screen.getByTestId("batch-bid-build-draft-btn"));
    await waitFor(() => {
      expect(buildMutateAsync).toHaveBeenCalledWith({
        names: ["name1"],
        bidValue: 1_000_000,
        lockup: 2_000_000,
        feeRate: undefined,
      });
    });
  });

  it("after build, confirm signs + broadcasts and invalidates queries", async () => {
    vi.mocked(readQueries.useNamesActionCapabilities).mockReturnValue({
      data: [cap("name1", true)],
      isFetching: false,
    } as any);
    buildMutateAsync.mockResolvedValue({ id: "draft1", summary: { feeDoos: 500 } });
    signMutateAsync.mockResolvedValue({ id: "draft1" });
    broadcastMutateAsync.mockResolvedValue({ txid: "abcdef0123456789", status: "ok" });
    const onClose = vi.fn();

    render(<BatchBidModal open={true} onClose={onClose} activeProfileId="p1" />);

    fireEvent.change(screen.getByTestId("batch-bid-names-input"), {
      target: { value: "name1" },
    });
    fireEvent.change(screen.getByTestId("batch-bid-value-input"), {
      target: { value: "1" },
    });
    fireEvent.change(screen.getByTestId("batch-bid-lockup-input"), {
      target: { value: "2" },
    });
    fireEvent.click(screen.getByTestId("batch-bid-check-names-btn"));
    await waitFor(() =>
      expect(screen.getByTestId("batch-bid-build-draft-btn")).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByTestId("batch-bid-build-draft-btn"));

    // Confirm modal appears.
    await waitFor(() =>
      expect(screen.getByText(/Confirm batch bid/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /Confirm/i }));

    await waitFor(() => {
      expect(signMutateAsync).toHaveBeenCalledWith("draft1");
      expect(broadcastMutateAsync).toHaveBeenCalledWith("draft1");
    });
    await waitFor(() => {
      expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ["wallet"] });
      expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ["read"] });
      expect(onClose).toHaveBeenCalled();
    });
  });

  it("shows a mapped (humanized) error toast when the build rejects", async () => {
    vi.mocked(readQueries.useNamesActionCapabilities).mockReturnValue({
      data: [cap("name1", true)],
      isFetching: false,
    } as any);
    buildMutateAsync.mockRejectedValue(new Error("insufficient funds"));

    render(<BatchBidModal open={true} onClose={vi.fn()} activeProfileId="p1" />);

    fireEvent.change(screen.getByTestId("batch-bid-names-input"), {
      target: { value: "name1" },
    });
    fireEvent.change(screen.getByTestId("batch-bid-value-input"), {
      target: { value: "1" },
    });
    fireEvent.change(screen.getByTestId("batch-bid-lockup-input"), {
      target: { value: "2" },
    });
    fireEvent.click(screen.getByTestId("batch-bid-check-names-btn"));
    await waitFor(() =>
      expect(screen.getByTestId("batch-bid-build-draft-btn")).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByTestId("batch-bid-build-draft-btn"));

    await waitFor(() => {
      expect(showToast).toHaveBeenCalledWith(
        "Build failed: Insufficient HNS balance for this transaction.",
        "error",
      );
    });
    expect(showToast).not.toHaveBeenCalledWith(
      expect.stringContaining("[object Object]"),
      "error",
    );
  });
});
