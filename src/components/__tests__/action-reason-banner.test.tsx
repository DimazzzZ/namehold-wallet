/**
 * ActionReasonBanner — the "why you can't do this" notice inside a guided
 * action panel. It must carry an Unlock button when the wallet is locked, the
 * same as the wallet page and the modal's own write-capability gate; a reason
 * unlocking cannot fix must stay text-only.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { ActionReasonBanner } from "../name-actions/ActionReasonBanner";

vi.mock("../../queries/wallet", () => ({
  useActiveProfile: vi.fn(),
  useSignerSession: vi.fn(),
  useUnlockSigner: vi.fn(),
}));
vi.mock("../../stores/ui", () => ({
  useUiStore: vi.fn((selector: (s: { showToast: () => void }) => unknown) =>
    selector({ showToast: vi.fn() }),
  ),
}));
vi.mock("../../lib/errors", () => ({
  mapError: vi.fn((e: unknown) => String(e)),
}));

import { useActiveProfile, useSignerSession, useUnlockSigner } from "../../queries/wallet";

/* eslint-disable @typescript-eslint/no-explicit-any */
function signer(unlocked: boolean) {
  (useActiveProfile as any).mockReturnValue({ data: { id: "p1" } });
  (useSignerSession as any).mockReturnValue({ data: { unlocked } });
  (useUnlockSigner as any).mockReturnValue({ isPending: false, mutateAsync: vi.fn() });
}
/* eslint-enable @typescript-eslint/no-explicit-any */

beforeEach(() => vi.clearAllMocks());

describe("ActionReasonBanner", () => {
  it("renders nothing without a reason", () => {
    signer(false);
    const { container } = render(<ActionReasonBanner reason={null} />);
    expect(container.firstChild).toBeNull();
  });

  it("puts an Unlock button beside the reason when the wallet is locked", () => {
    signer(false);
    render(<ActionReasonBanner reason="Unlock your wallet to sign transactions." />);

    expect(screen.getByTestId("action-reason")).toHaveTextContent(
      "Unlock your wallet to sign transactions.",
    );
    expect(screen.getByTestId("unlock-now")).toBeInTheDocument();
  });

  it("stays text-only for a reason unlocking cannot fix", () => {
    // Signer already unlocked — the action is blocked by the auction phase, so
    // offering "Unlock" would be a dead end.
    signer(true);
    render(<ActionReasonBanner reason="Reveal has not started yet." />);

    expect(screen.getByTestId("action-reason")).toHaveTextContent("Reveal has not started yet.");
    expect(screen.queryByTestId("unlock-now")).toBeNull();
  });
});
