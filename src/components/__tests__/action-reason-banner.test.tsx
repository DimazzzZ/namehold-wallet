/**
 * ActionReasonBanner — the "why you can't do this" notice inside a guided
 * action panel. It must carry an Unlock button when the wallet is locked, the
 * same as the wallet page and the modal's own write-capability gate; a reason
 * unlocking cannot fix must stay text-only.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ActionReasonBanner } from "../name-actions/ActionReasonBanner";
import { makeProfile, makeSession } from "../../test/fixtures/wallet";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

function signer(unlocked: boolean) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([makeProfile()]);
      case "get_signer_session":
        return Promise.resolve(makeSession({ unlocked }));
      default:
        return Promise.reject(new Error(`unexpected command ${cmd}`));
    }
  });
}

function renderBanner(reason: string | null) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <ActionReasonBanner reason={reason} />
    </QueryClientProvider>,
  );
}

/** The button decides once both queries have answered. */
const sessionLoaded = () =>
  waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith("list_wallet_profiles", undefined);
    expect(invokeMock).toHaveBeenCalledWith("get_signer_session", undefined);
  });

beforeEach(() => {
  invokeMock.mockReset();
});

describe("ActionReasonBanner", () => {
  it("renders nothing without a reason", () => {
    signer(false);
    const { container } = renderBanner(null);
    expect(container.firstChild).toBeNull();
  });

  it("puts an Unlock button beside the reason when the wallet is locked", async () => {
    signer(false);
    renderBanner("Unlock your wallet to sign transactions.");

    expect(screen.getByTestId("action-reason")).toHaveTextContent(
      "Unlock your wallet to sign transactions.",
    );
    expect(await screen.findByTestId("unlock-now")).toBeInTheDocument();
  });

  it("stays text-only for a reason unlocking cannot fix", async () => {
    // Signer already unlocked — the action is blocked by the auction phase, so
    // offering "Unlock" would be a dead end.
    signer(true);
    renderBanner("Reveal has not started yet.");
    await sessionLoaded();

    expect(screen.getByTestId("action-reason")).toHaveTextContent("Reveal has not started yet.");
    expect(screen.queryByTestId("unlock-now")).toBeNull();
  });
});
