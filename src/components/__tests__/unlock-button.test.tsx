/**
 * UnlockButton — self-hiding unlock action for locked-wallet notices.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { UnlockButton } from "../UnlockButton";

// Mock the wallet hooks
vi.mock("../../queries/wallet", () => ({
  useActiveProfile: vi.fn(),
  useSignerSession: vi.fn(),
  useUnlockSigner: vi.fn(),
}));

// Mock the UI store
vi.mock("../../stores/ui", () => ({
  useUiStore: vi.fn((selector) => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const store = {
      showToast: vi.fn(),
    };
    return selector(store);
  }),
}));

// Mock mapError
vi.mock("../../lib/errors", () => ({
  mapError: vi.fn((e) => (e instanceof Error ? e.message : String(e))),
}));

import { useActiveProfile, useSignerSession, useUnlockSigner } from "../../queries/wallet";
import { useUiStore } from "../../stores/ui";

beforeEach(() => {
  vi.clearAllMocks();
});

describe("UnlockButton", () => {
  it("renders nothing when there is no active profile", () => {
    (useActiveProfile as any).mockReturnValue({ data: null });
    (useSignerSession as any).mockReturnValue({ data: { unlocked: false } });
    (useUnlockSigner as any).mockReturnValue({ isPending: false, mutateAsync: vi.fn() });

    const { container } = render(<UnlockButton />);
    expect(container.firstChild).toBeNull();
  });

  it("renders nothing when the wallet is already unlocked", () => {
    (useActiveProfile as any).mockReturnValue({ data: { id: "profile-1" } });
    (useSignerSession as any).mockReturnValue({ data: { unlocked: true } });
    (useUnlockSigner as any).mockReturnValue({ isPending: false, mutateAsync: vi.fn() });

    const { container } = render(<UnlockButton />);
    expect(container.firstChild).toBeNull();
  });

  it("renders the button when locked and profile exists", () => {
    (useActiveProfile as any).mockReturnValue({ data: { id: "profile-1" } });
    (useSignerSession as any).mockReturnValue({ data: { unlocked: false } });
    (useUnlockSigner as any).mockReturnValue({ isPending: false, mutateAsync: vi.fn() });

    render(<UnlockButton />);
    expect(screen.getByTestId("unlock-now")).toBeInTheDocument();
    expect(screen.getByTestId("unlock-now")).toHaveTextContent("Unlock");
  });

  it("shows 'Unlocking…' label while pending", () => {
    (useActiveProfile as any).mockReturnValue({ data: { id: "profile-1" } });
    (useSignerSession as any).mockReturnValue({ data: { unlocked: false } });
    (useUnlockSigner as any).mockReturnValue({ isPending: true, mutateAsync: vi.fn() });

    render(<UnlockButton />);
    expect(screen.getByTestId("unlock-now")).toHaveTextContent("Unlocking…");
    expect(screen.getByTestId("unlock-now")).toBeDisabled();
  });

  it("calls unlock mutation and shows success toast on success", async () => {
    const mockMutateAsync = vi.fn().mockResolvedValue(undefined);
    const mockShowToast = vi.fn();

    (useActiveProfile as any).mockReturnValue({ data: { id: "profile-1" } });
    (useSignerSession as any).mockReturnValue({ data: { unlocked: false } });
    (useUnlockSigner as any).mockReturnValue({ isPending: false, mutateAsync: mockMutateAsync });
    (useUiStore as any).mockImplementation((selector: any) => {
      return selector({ showToast: mockShowToast });
    });

    render(<UnlockButton />);
    fireEvent.click(screen.getByTestId("unlock-now"));

    await waitFor(() => {
      expect(mockMutateAsync).toHaveBeenCalledWith("profile-1");
      expect(mockShowToast).toHaveBeenCalledWith("Wallet unlocked", "success");
    });
  });

  it("shows error toast on unlock failure", async () => {
    const mockMutateAsync = vi.fn().mockRejectedValue(new Error("Unlock failed"));
    const mockShowToast = vi.fn();

    (useActiveProfile as any).mockReturnValue({ data: { id: "profile-1" } });
    (useSignerSession as any).mockReturnValue({ data: { unlocked: false } });
    (useUnlockSigner as any).mockReturnValue({ isPending: false, mutateAsync: mockMutateAsync });
    (useUiStore as any).mockImplementation((selector: any) => {
      return selector({ showToast: mockShowToast });
    });

    render(<UnlockButton />);
    fireEvent.click(screen.getByTestId("unlock-now"));

    await waitFor(() => {
      expect(mockShowToast).toHaveBeenCalledWith("Unlock failed", "error");
    });
  });

  it("calls onUnlocked callback after successful unlock", async () => {
    const mockMutateAsync = vi.fn().mockResolvedValue(undefined);
    const mockOnUnlocked = vi.fn();

    (useActiveProfile as any).mockReturnValue({ data: { id: "profile-1" } });
    (useSignerSession as any).mockReturnValue({ data: { unlocked: false } });
    (useUnlockSigner as any).mockReturnValue({ isPending: false, mutateAsync: mockMutateAsync });
    (useUiStore as any).mockImplementation((selector: any) => {
      return selector({ showToast: vi.fn() });
    });

    render(<UnlockButton onUnlocked={mockOnUnlocked} />);
    fireEvent.click(screen.getByTestId("unlock-now"));

    await waitFor(() => {
      expect(mockOnUnlocked).toHaveBeenCalled();
    });
  });

  it("respects custom label prop", () => {
    (useActiveProfile as any).mockReturnValue({ data: { id: "profile-1" } });
    (useSignerSession as any).mockReturnValue({ data: { unlocked: false } });
    (useUnlockSigner as any).mockReturnValue({ isPending: false, mutateAsync: vi.fn() });

    render(<UnlockButton label="Unlock now" />);
    expect(screen.getByTestId("unlock-now")).toHaveTextContent("Unlock now");
  });
});
