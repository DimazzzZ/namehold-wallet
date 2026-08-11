/**
 * useAppUpdate store — simulateUpdateFlow() + simulated install().
 *
 * `simulateUpdateFlow()` seeds the store into the "available" state (marked
 * `simulated: true`) from the `fetch_latest_release_meta` command (or a
 * synthetic fallback when that command rejects) and STOPS there — it does NOT
 * auto-install. The fake download only runs when the user clicks "Install
 * now", which calls `install()`; because `simulated` is set, `install()`
 * runs a fake progress loop instead of the real Rust `install_update`.
 *
 * Uses fake timers so the test doesn't wait for real setTimeout delays.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Mock the store's `../lib/invoke` import before importing the store. The
// store lives at src/hooks/useAppUpdate.ts, so its `../lib/invoke` resolves to
// src/lib/invoke — which from this test file (src/hooks/__tests__/) is
// `../../lib/invoke`.
const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...a: unknown[]) => invokeMock(...a),
}));
vi.mock("../../lib/runtime", () => ({
  isTauri: () => true,
  isBrowser: () => false,
}));
// Channel is imported by the store but only used by the real install() path,
// not by the simulated flow.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: class {
    onmessage?: (msg: unknown) => void;
  },
}));

import { useAppUpdate } from "../useAppUpdate";

/**
 * Flush pending microtasks (the two chained `await invoke(...)` calls at the
 * start of the flow) without advancing wall-clock timers.
 */
async function flushMicrotasks() {
  await vi.advanceTimersByTimeAsync(0);
  await vi.advanceTimersByTimeAsync(0);
  await vi.advanceTimersByTimeAsync(0);
  await vi.advanceTimersByTimeAsync(0);
}

beforeEach(() => {
  vi.useFakeTimers();
  invokeMock.mockReset();
  useAppUpdate.setState({
    phase: "idle",
    available: null,
    progress: null,
    error: null,
    dismissedVersion: null,
    simulated: false,
  });
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
});

describe("useAppUpdate — simulateUpdateFlow()", () => {
  it("seeds the 'available' notice with real release data and stops (no auto-install)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "current_version") return Promise.resolve("0.5.0");
      if (cmd === "fetch_latest_release_meta")
        return Promise.resolve({
          version: "0.6.0",
          notes: "Bug fixes and improvements.",
          date: "2026-08-01T12:00:00Z",
        });
      return Promise.reject(new Error(`unexpected command: ${cmd}`));
    });

    const flow = useAppUpdate.getState().simulateUpdateFlow();
    await flushMicrotasks();
    await flow;

    const s = useAppUpdate.getState();
    expect(s.phase).toBe("available");
    expect(s.simulated).toBe(true);
    expect(s.available?.version).toBe("0.6.0");
    expect(s.available?.currentVersion).toBe("0.5.0");
    expect(s.available?.notes).toBe("Bug fixes and improvements.");
    expect(s.progress).toBeNull();

    // Crucially: it must NOT advance on its own. Even after plenty of time,
    // the phase stays "available" until the user clicks Install.
    await vi.advanceTimersByTimeAsync(5000);
    expect(useAppUpdate.getState().phase).toBe("available");
  });

  it("falls back to a bumped version when fetch_latest_release_meta rejects", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "current_version") return Promise.resolve("0.5.0");
      if (cmd === "fetch_latest_release_meta")
        return Promise.reject(new Error("network error"));
      return Promise.reject(new Error(`unexpected command: ${cmd}`));
    });

    const flow = useAppUpdate.getState().simulateUpdateFlow();
    await flushMicrotasks();
    await flow;

    expect(useAppUpdate.getState().phase).toBe("available");
    // Fallback bumps the patch: 0.5.0 → 0.5.1
    expect(useAppUpdate.getState().available?.version).toBe("0.5.1");
    expect(useAppUpdate.getState().simulated).toBe(true);
  });

  it("bumps version when GitHub's latest matches current version", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "current_version") return Promise.resolve("0.6.0");
      if (cmd === "fetch_latest_release_meta")
        return Promise.resolve({
          version: "0.6.0",
          notes: "Same version notes.",
          date: "2026-08-01T12:00:00Z",
        });
      return Promise.reject(new Error(`unexpected command: ${cmd}`));
    });

    const flow = useAppUpdate.getState().simulateUpdateFlow();
    await flushMicrotasks();
    await flow;

    // Should bump since the fetched version equals current.
    expect(useAppUpdate.getState().available?.version).toBe("0.6.1");
    expect(useAppUpdate.getState().available?.notes).toBe("Same version notes.");
  });

  it("no-ops when phase is already installing", async () => {
    useAppUpdate.setState({ phase: "installing", progress: 0.5 });

    const flow = useAppUpdate.getState().simulateUpdateFlow();
    await flushMicrotasks();
    await flow;

    // Phase unchanged — action bailed early.
    expect(useAppUpdate.getState().phase).toBe("installing");
    expect(useAppUpdate.getState().progress).toBe(0.5);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

describe("useAppUpdate — simulated install()", () => {
  it("runs a fake download loop when the update is simulated", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "current_version") return Promise.resolve("0.5.0");
      if (cmd === "fetch_latest_release_meta")
        return Promise.resolve({ version: "0.6.0", notes: "Notes.", date: null });
      return Promise.reject(new Error(`unexpected command: ${cmd}`));
    });

    // Seed via the simulate flow.
    const flow = useAppUpdate.getState().simulateUpdateFlow();
    await flushMicrotasks();
    await flow;
    expect(useAppUpdate.getState().phase).toBe("available");

    // User clicks "Install now".
    const install = useAppUpdate.getState().install();
    await vi.advanceTimersByTimeAsync(0);
    expect(useAppUpdate.getState().phase).toBe("installing");
    expect(useAppUpdate.getState().progress).toBe(0);

    // Advance through 10 ticks of 120ms each → progress reaches 1, installed.
    await vi.advanceTimersByTimeAsync(120 * 10);
    await install;
    expect(useAppUpdate.getState().phase).toBe("installed");
    expect(useAppUpdate.getState().progress).toBe(1);
  });

  it("aborts the fake download if reset() is called mid-install", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "current_version") return Promise.resolve("0.5.0");
      if (cmd === "fetch_latest_release_meta")
        return Promise.resolve({ version: "0.6.0", notes: null, date: null });
      return Promise.reject(new Error(`unexpected command: ${cmd}`));
    });

    const flow = useAppUpdate.getState().simulateUpdateFlow();
    await flushMicrotasks();
    await flow;

    const install = useAppUpdate.getState().install();
    await vi.advanceTimersByTimeAsync(120 * 2);
    expect(useAppUpdate.getState().phase).toBe("installing");

    // User resets mid-install.
    useAppUpdate.getState().reset();
    expect(useAppUpdate.getState().phase).toBe("idle");
    expect(useAppUpdate.getState().simulated).toBe(false);

    // Advance remaining ticks — phase should stay idle (aborted).
    await vi.advanceTimersByTimeAsync(120 * 10);
    await install;
    expect(useAppUpdate.getState().phase).toBe("idle");
  });
});
