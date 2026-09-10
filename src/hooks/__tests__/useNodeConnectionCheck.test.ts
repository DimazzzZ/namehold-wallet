import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useNodeConnectionCheck } from "../useNodeConnectionCheck";

const reachable = { reachable: true, height: 100, headers: 100, synced: true, network: "main", error: null };
const unreachable = { reachable: false, height: null, headers: null, synced: false, network: null, error: "connection refused" };

beforeEach(() => invokeMock.mockReset());

describe("useNodeConnectionCheck", () => {
  it("refuses an empty URL without calling the backend", async () => {
    const { result } = renderHook(() => useNodeConnectionCheck());
    await act(async () => {
      await result.current.run("   ");
    });
    expect(invokeMock).not.toHaveBeenCalled();
    expect(result.current.error).toMatch(/enter a node rpc url/i);
    expect(result.current.ok).toBe(false);
  });

  it("trims the URL, omits a blank key, and reports ok on a reachable node", async () => {
    invokeMock.mockResolvedValue(reachable);
    const { result } = renderHook(() => useNodeConnectionCheck());
    await act(async () => {
      await result.current.run("  https://n.example.com:12037 ", "");
    });
    expect(invokeMock).toHaveBeenCalledWith("check_node_connection", {
      url: "https://n.example.com:12037",
      api_key: undefined,
    });
    expect(result.current.ok).toBe(true);
    expect(result.current.result).toEqual(reachable);
    expect(result.current.error).toBeNull();
    expect(result.current.testing).toBe(false);
  });

  it("surfaces the node's reason when unreachable", async () => {
    invokeMock.mockResolvedValue(unreachable);
    const { result } = renderHook(() => useNodeConnectionCheck());
    await act(async () => {
      await result.current.run("https://n.example.com:12037");
    });
    expect(result.current.ok).toBe(false);
    expect(result.current.error).toBe("connection refused");
  });

  it("surfaces a thrown backend error (e.g. the plaintext-key guard)", async () => {
    invokeMock.mockRejectedValueOnce(new Error("refusing to send API key over plaintext HTTP"));
    const { result } = renderHook(() => useNodeConnectionCheck());
    await act(async () => {
      await result.current.run("http://10.0.0.5:12037", "k");
    });
    expect(result.current.ok).toBe(false);
    expect(result.current.error).toMatch(/plaintext/);
  });

  it("reset() drops a previous result and error", async () => {
    invokeMock.mockResolvedValue(reachable);
    const { result } = renderHook(() => useNodeConnectionCheck());
    await act(async () => {
      await result.current.run("https://n.example.com:12037");
    });
    expect(result.current.ok).toBe(true);
    act(() => result.current.reset());
    expect(result.current.ok).toBe(false);
    expect(result.current.result).toBeNull();
    expect(result.current.error).toBeNull();
  });

  it("a stale response arriving after reset() mid-flight is dropped, not applied", async () => {
    let resolveA: (v: typeof reachable) => void;
    invokeMock.mockReturnValueOnce(
      new Promise((r) => {
        resolveA = r;
      }),
    );
    const { result } = renderHook(() => useNodeConnectionCheck());

    // Kick off a probe for URL A but don't await it — it's still in flight.
    let runPromise!: Promise<void>;
    act(() => {
      runPromise = result.current.run("https://a.example.com:12037");
    });
    expect(result.current.testing).toBe(true);

    // User edits the field before A responds.
    act(() => result.current.reset());
    // Editing mid-probe must not leave the Test button stuck disabled.
    expect(result.current.testing).toBe(false);

    // Now A's response finally arrives.
    await act(async () => {
      resolveA(reachable);
      await runPromise;
    });

    expect(result.current.result).toBeNull();
    expect(result.current.error).toBeNull();
    expect(result.current.ok).toBe(false);
  });

  it("a superseded run() (A) never overwrites the outcome of the newest run() (B)", async () => {
    let resolveA: (v: typeof reachable) => void;
    let resolveB: (v: typeof unreachable) => void;
    invokeMock
      .mockReturnValueOnce(
        new Promise((r) => {
          resolveA = r;
        }),
      )
      .mockReturnValueOnce(
        new Promise((r) => {
          resolveB = r;
        }),
      );
    const { result } = renderHook(() => useNodeConnectionCheck());

    let runAPromise!: Promise<void>;
    act(() => {
      runAPromise = result.current.run("https://a.example.com:12037");
    });
    let runBPromise!: Promise<void>;
    act(() => {
      runBPromise = result.current.run("https://b.example.com:12037");
    });

    // B resolves first, then the stale A resolves after — A must not win.
    await act(async () => {
      resolveB(unreachable);
      await runBPromise;
    });
    await act(async () => {
      resolveA(reachable);
      await runAPromise;
    });

    expect(result.current.result).toEqual(unreachable);
    expect(result.current.error).toBe("connection refused");
    expect(result.current.ok).toBe(false);
    expect(result.current.testing).toBe(false);
  });
});
