/**
 * useAppUpdate store — dismiss() guard during installed phase.
 *
 * Verifies that dismiss() is a no-op when phase is "installed", and that
 * it still works normally for the "available" phase (remembers dismissedVersion
 * and collapses the banner).
 */
import { describe, it, expect, beforeEach } from "vitest";
import { useAppUpdate } from "../useAppUpdate";

beforeEach(() => {
  useAppUpdate.setState({
    phase: "idle",
    available: null,
    progress: null,
    error: null,
    dismissedVersion: null,
  });
  // Clear localStorage
  try {
    localStorage.removeItem("namehold.update.dismissedVersion");
  } catch {
    // Non-fatal if localStorage is unavailable in test env
  }
});

describe("useAppUpdate — dismiss() guard", () => {
  it("does nothing when phase is 'installed'", () => {
    useAppUpdate.setState({
      phase: "installed",
      available: {
        version: "0.5.0",
        currentVersion: "0.4.0",
        notes: null,
        date: null,
      },
      progress: 1,
    });

    const initialDismissed = useAppUpdate.getState().dismissedVersion;

    useAppUpdate.getState().dismiss();

    // Phase should remain "installed"
    expect(useAppUpdate.getState().phase).toBe("installed");
    // dismissedVersion should not change
    expect(useAppUpdate.getState().dismissedVersion).toBe(initialDismissed);
  });

  it("still works normally for the 'available' phase", () => {
    useAppUpdate.setState({
      phase: "available",
      available: {
        version: "0.5.0",
        currentVersion: "0.4.0",
        notes: null,
        date: null,
      },
      progress: null,
    });

    useAppUpdate.getState().dismiss();

    // Phase should collapse to idle
    expect(useAppUpdate.getState().phase).toBe("idle");
    // dismissedVersion should be set to the available version
    expect(useAppUpdate.getState().dismissedVersion).toBe("0.5.0");
  });
});
