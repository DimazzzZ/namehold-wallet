import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// The one place the fate of a built draft is decided (useDraftRunner). Every
// flow that builds a draft uses it; their own tests only check they call it.

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useDraftRunner } from "../useDraftRunner";

type Fail = "unlock" | "sign" | "broadcast" | null;

function node(fail: Fail = null, opts: { deleteFails?: boolean } = {}) {
  return async (cmd: string) => {
    switch (cmd) {
      case "unlock_local_signer":
        if (fail === "unlock") throw new Error("Confirmation declined");
        return null;
      case "sign_tx_draft":
        if (fail === "sign") throw new Error("Confirmation declined");
        return { id: "d1" };
      case "broadcast_tx_draft":
        if (fail === "broadcast") throw new Error("Node RPC error: no answer");
        return { draftId: "d1", txid: "ab".repeat(32), status: "broadcasted" };
      case "delete_tx_draft":
        if (opts.deleteFails) throw new Error("db busy");
        return null;
      default:
        return null;
    }
  };
}

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
}

const deletes = () =>
  invokeMock.mock.calls.filter((c) => c[0] === "delete_tx_draft").map((c) => c[1]);
const runner = () => renderHook(() => useDraftRunner(), { wrapper: wrapper() }).result;

beforeEach(() => invokeMock.mockReset());

describe("useDraftRunner", () => {
  it("send: signs and broadcasts, and discards nothing", async () => {
    invokeMock.mockImplementation(node());
    const r = runner();
    await act(async () => {
      const res = await r.current.send("d1", "p1", true);
      expect(res.txid).toHaveLength(64);
    });
    expect(deletes()).toEqual([]);
  });

  it.each(["unlock", "sign"] as const)(
    "send: a failure at %s (before broadcast) discards the draft and rethrows",
    async (fail) => {
      invokeMock.mockImplementation(node(fail));
      const r = runner();
      await act(async () => {
        await expect(r.current.send("d1", "p1", fail !== "unlock")).rejects.toThrow();
      });
      expect(deletes()).toEqual([{ draftId: "d1" }]);
    },
  );

  it("send: a broadcast failure never discards (the node may hold the transaction)", async () => {
    invokeMock.mockImplementation(node("broadcast"));
    const r = runner();
    await act(async () => {
      await expect(r.current.send("d1", "p1", true)).rejects.toThrow();
    });
    expect(deletes()).toEqual([]);
  });

  it("send with keepOnFailure keeps the draft for a retry; abandon later discards it", async () => {
    invokeMock.mockImplementation(node("sign"));
    const r = runner();
    await act(async () => {
      await expect(r.current.send("d1", "p1", true, { keepOnFailure: true })).rejects.toThrow();
    });
    expect(deletes()).toEqual([]);
    await act(async () => {
      expect(await r.current.abandon("d1")).toBe(true);
    });
    expect(deletes()).toEqual([{ draftId: "d1" }]);
  });

  it("abandon: a built, unsent draft is discarded; nothing to abandon is a no-op", async () => {
    invokeMock.mockImplementation(node());
    const r = runner();
    await act(async () => {
      expect(await r.current.abandon("d2")).toBe(true);
      expect(await r.current.abandon(null)).toBe(false);
      expect(await r.current.abandon(undefined)).toBe(false);
    });
    expect(deletes()).toEqual([{ draftId: "d2" }]);
  });

  it("abandon after a tried broadcast is a no-op, sent or failed", async () => {
    invokeMock.mockImplementation(node("broadcast"));
    const r = runner();
    await act(async () => {
      await expect(r.current.run("d1", "p1", true)).rejects.toThrow();
      expect(await r.current.abandon("d1")).toBe(false);
    });
    invokeMock.mockImplementation(node());
    await act(async () => {
      await r.current.run("d3", "p1", true);
      expect(await r.current.abandon("d3")).toBe(false);
    });
    expect(deletes()).toEqual([]);
  });

  it("settleFailed: discards on a failure before broadcast or one with no stage, keeps after broadcast", async () => {
    invokeMock.mockImplementation(node("broadcast"));
    const r = runner();
    let broadcastError: unknown;
    await act(async () => {
      await r.current.run("d1", "p1", true).catch((e) => (broadcastError = e));
      expect(await r.current.settleFailed("d1", broadcastError)).toBe(false);
      // An error from around the send (a signer wrapper) carries no stage: before broadcast.
      expect(await r.current.settleFailed("d4", new Error("Wallet locked"))).toBe(true);
    });
    expect(deletes()).toEqual([{ draftId: "d4" }]);
  });

  it("a failed discard is swallowed (Activity still offers Discard)", async () => {
    invokeMock.mockImplementation(node("sign", { deleteFails: true }));
    const r = runner();
    await act(async () => {
      await expect(r.current.send("d1", "p1", true)).rejects.toThrow("Confirmation declined");
      expect(await r.current.abandon("d5")).toBe(false);
    });
  });
});
