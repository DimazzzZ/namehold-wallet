import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ReceiveDialog } from "../ReceiveDialog";
import * as readQueries from "../../queries/read";
import { openExternal } from "../../lib/openExternal";
import type { WalletProfileSummary } from "../../types";

const showToast = vi.fn();
const mutateAsync = vi.fn();

vi.mock("../../queries/read", () => ({
  useReceiveAddresses: vi.fn(() => ({ data: [], isLoading: false })),
  useRevealNextReceiveAddress: vi.fn(() => ({ mutateAsync, isPending: false })),
}));
vi.mock("../../queries/wallet", () => ({
  useActiveProfile: vi.fn(() => ({ data: { id: "p1", network: "mainnet" } })),
}));
vi.mock("../../lib/clipboard", () => ({ writeText: vi.fn() }));
vi.mock("../../lib/openExternal", async (orig) => ({
  ...(await orig<typeof import("../../lib/openExternal")>()),
  openExternal: vi.fn(),
}));
vi.mock("../../stores/ui", () => ({
  useUiStore: vi.fn((selector) => selector({ showToast })),
}));

const profile = (over: Partial<WalletProfileSummary> = {}) =>
  ({
    id: "p1",
    network: "mainnet",
    receiveAddress: "hs1qprofileaddr",
    accountXpub: "xpub6CUGexample",
    ...over,
  }) as WalletProfileSummary;

describe("ReceiveDialog", () => {
  beforeEach(() => vi.clearAllMocks());

  it("copies the address with an 'Address copied' toast", async () => {
    render(<ReceiveDialog open onClose={vi.fn()} profile={profile()} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy address" }));
    await waitFor(() => expect(showToast).toHaveBeenCalledWith("Address copied", "success"));
  });

  it("offers Explorer only on mainnet", () => {
    const { unmount } = render(<ReceiveDialog open onClose={vi.fn()} profile={profile()} />);
    fireEvent.click(screen.getByTestId("receive-address-explorer-link"));
    expect(openExternal).toHaveBeenCalledWith(expect.stringContaining("hs1qprofileaddr"));
    unmount();

    render(
      <ReceiveDialog
        open
        onClose={vi.fn()}
        profile={profile({ network: "regtest", receiveAddress: "rs1qx" })}
      />,
    );
    expect(screen.queryByTestId("receive-address-explorer-link")).toBeNull();
    expect(screen.getByText(/regtest addresses differ from mainnet/)).toBeInTheDocument();
  });

  it("shows the new address after New address", async () => {
    mutateAsync.mockResolvedValue("hs1qfreshaddr");
    render(<ReceiveDialog open onClose={vi.fn()} profile={profile()} />);
    fireEvent.click(screen.getByTestId("receive-new-address"));
    await waitFor(() =>
      expect(screen.getByTestId("receive-address-value")).toHaveTextContent("hs1qfreshaddr"),
    );
    expect(mutateAsync).toHaveBeenCalledWith({ walletProfileId: "p1" });
  });

  it("keeps the xpub testids and counts all addresses", () => {
    vi.mocked(readQueries.useReceiveAddresses).mockReturnValue({
      data: [{ index: 0, address: "a", used: false, firstSeenAt: null }],
      isLoading: false,
    } as never);
    render(<ReceiveDialog open onClose={vi.fn()} profile={profile()} />);
    expect(screen.getByTestId("account-xpub-value")).toHaveTextContent("xpub6CUGexample");
    expect(screen.getByTestId("copy-xpub")).toHaveTextContent("Copy public key");
    expect(screen.getByTestId("receive-all-addresses")).toHaveTextContent("All addresses (1)");
  });

  it("shows the no-address line when nothing is derived", () => {
    render(<ReceiveDialog open onClose={vi.fn()} profile={profile({ receiveAddress: null })} />);
    expect(screen.getByText("No address derived yet. Try syncing.")).toBeInTheDocument();
    expect(screen.getByTestId("receive-all-addresses")).toBeInTheDocument();
  });

  it("closes on Esc", () => {
    const onClose = vi.fn();
    render(<ReceiveDialog open onClose={onClose} profile={profile()} />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});
