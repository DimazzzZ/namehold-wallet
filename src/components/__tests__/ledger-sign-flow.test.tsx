import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Mock the Tauri invoke bridge before importing the module under test.
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useExecuteDraft } from "../../queries/wallet";
import { unwrapStaged, stageOf } from "../../lib/errors";

function wrap(ui: ReactNode): ReactNode {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return <QueryClientProvider client={qc}>{ui}</QueryClientProvider>;
}

/**
 * M8: end-to-end sign flow tests focused on the Ledger UX contract.
 *
 * `useExecuteDraft` is the shared unlock→sign→broadcast pipeline that both
 * `NameActionsModal` and `WalletView` drive. For Ledger profiles the caller
 * passes `unlocked = true` (no passphrase to prompt for), so the pipeline must:
 *   1. Skip `unlock_local_signer` entirely.
 *   2. Call `sign_tx_draft` (which the backend routes to the device).
 *   3. Tag any sign-phase rejection with stage="sign" so the UI shows the
 *      right toast copy.
 *   4. Preserve the raw backend error string so the frontend can render the
 *      actionable device guidance (rejected / locked / wrong app / …).
 */
describe("useExecuteDraft — Ledger signing pipeline", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("skips unlock_local_signer when a Ledger caller passes unlocked=true", async () => {
    // sign_tx_draft succeeds, broadcast_tx_draft succeeds.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "sign_tx_draft") return Promise.resolve({ id: "d1", status: "signed" });
      if (cmd === "broadcast_tx_draft") return Promise.resolve({ txid: "cafe1234567890abcdef" });
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });

    const { result } = renderHook(() => useExecuteDraft(), { wrapper: ({ children }) => wrap(children) });

    let outcome: unknown;
    await act(async () => {
      outcome = await result.current.run("d1", "p-ledger", /* unlocked */ true);
    });

    expect(outcome).toEqual({ txid: "cafe1234567890abcdef" });
    const cmds = invokeMock.mock.calls.map((c) => c[0]);
    expect(cmds).toEqual(["sign_tx_draft", "broadcast_tx_draft"]);
    expect(cmds).not.toContain("unlock_local_signer");
  });

  it("surfaces a Ledger user-rejection as a stage=sign error carrying the backend string", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "sign_tx_draft") {
        // Matches AppError::UserRejected — see error.rs.
        return Promise.reject(new Error("Confirmation declined"));
      }
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });

    const { result } = renderHook(() => useExecuteDraft(), { wrapper: ({ children }) => wrap(children) });

    let caught: unknown;
    await act(async () => {
      try {
        await result.current.run("d1", "p-ledger", true);
      } catch (e) {
        caught = e;
      }
    });

    expect(caught).toBeDefined();
    expect(stageOf(caught)).toBe("sign");
    // The staged wrapper must not obscure the underlying message.
    expect(String(unwrapStaged(caught))).toMatch(/Confirmation declined/);
  });

  it("surfaces a Ledger device error (locked / wrong app) with actionable text intact", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "sign_tx_draft") {
        // Matches AppError::Device rendering from status_word_message().
        return Promise.reject(
          new Error("Ledger device error: APDU failed with status 0x5515 (device locked — unlock it)"),
        );
      }
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });

    const { result } = renderHook(() => useExecuteDraft(), { wrapper: ({ children }) => wrap(children) });

    let caught: unknown;
    await act(async () => {
      try {
        await result.current.run("d1", "p-ledger", true);
      } catch (e) {
        caught = e;
      }
    });

    expect(stageOf(caught)).toBe("sign");
    const msg = String(unwrapStaged(caught));
    // Both the SW code and the human hint must survive the pipeline so the
    // toast can render "unlock your Ledger" and not a generic failure.
    expect(msg).toMatch(/0x5515/);
    expect(msg).toMatch(/locked/);
  });

  it("surfaces a wrong-app error with the app hint preserved", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "sign_tx_draft") {
        return Promise.reject(
          new Error(
            "Ledger device error: APDU failed with status 0x6d00 (instruction not supported — is the Handshake app open?)",
          ),
        );
      }
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });

    const { result } = renderHook(() => useExecuteDraft(), { wrapper: ({ children }) => wrap(children) });

    let caught: unknown;
    await act(async () => {
      try {
        await result.current.run("d1", "p-ledger", true);
      } catch (e) {
        caught = e;
      }
    });

    const msg = String(unwrapStaged(caught));
    expect(msg).toMatch(/Handshake app/);
  });

  it("tags a broadcast-phase failure with stage=broadcast so it doesn't look like a sign failure", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "sign_tx_draft") return Promise.resolve({ id: "d1", status: "signed" });
      if (cmd === "broadcast_tx_draft") return Promise.reject(new Error("node offline"));
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });

    const { result } = renderHook(() => useExecuteDraft(), { wrapper: ({ children }) => wrap(children) });

    let caught: unknown;
    await act(async () => {
      try {
        await result.current.run("d1", "p-ledger", true);
      } catch (e) {
        caught = e;
      }
    });

    // The device already produced a signed tx by this point — the failure is
    // in the broadcast leg, which the UI renders as a retriable network hiccup
    // rather than a device problem.
    expect(stageOf(caught)).toBe("broadcast");
    expect(String(unwrapStaged(caught))).toMatch(/node offline/);
  });

  it("still runs unlock_local_signer for a hot-wallet caller (unlocked=false)", async () => {
    // Regression guard: the ledger short-circuit must not accidentally apply
    // to hot wallets. When unlocked=false the pipeline calls unlock first.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "unlock_local_signer") return Promise.resolve("p-hot");
      if (cmd === "sign_tx_draft") return Promise.resolve({ id: "d1", status: "signed" });
      if (cmd === "broadcast_tx_draft") return Promise.resolve({ txid: "beef1234567890abcdef" });
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });

    const { result } = renderHook(() => useExecuteDraft(), { wrapper: ({ children }) => wrap(children) });

    await act(async () => {
      await result.current.run("d1", "p-hot", /* unlocked */ false);
    });

    const cmds = invokeMock.mock.calls.map((c) => c[0]);
    expect(cmds).toEqual(["unlock_local_signer", "sign_tx_draft", "broadcast_tx_draft"]);
  });
});

/**
 * Verify that the unlock-error text a Ledger user sees is the *new* one from
 * M6 (the backend), not the legacy "cannot unlock a watch-only profile" that
 * previously misled them into thinking the profile was broken. This mirrors
 * the mapping the frontend does — the message goes through as-is.
 */
describe("Ledger unlock error copy (M6)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("surfaces the ledger-aware unlock error verbatim from the backend", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "unlock_local_signer") {
        // The new backend message (see M6 patch in secure_wallet.rs).
        return Promise.reject(
          new Error("Ledger profiles are always watch-only and do not require unlocking"),
        );
      }
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });

    const { result } = renderHook(() => useExecuteDraft(), { wrapper: ({ children }) => wrap(children) });

    let caught: unknown;
    await act(async () => {
      try {
        // Simulate a stale caller that forgot to short-circuit for Ledger
        // and asked the pipeline to unlock. The message the user sees must
        // point them at the real explanation (device-signed, no passphrase)
        // rather than the legacy watch-only wording.
        await result.current.run("d1", "p-ledger", /* unlocked */ false);
      } catch (e) {
        caught = e;
      }
    });

    const msg = String(unwrapStaged(caught));
    expect(msg).toMatch(/Ledger profiles/i);
    expect(msg).not.toMatch(/cannot unlock a watch-only profile/);
  });
});

// Silence the "waitFor unused" lint since we opted for explicit act() calls.
void waitFor;
