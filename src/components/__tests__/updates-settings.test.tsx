/**
 * Settings — Updates card.
 *
 * Covers the click-through of the manual "Check for updates" flow: initial
 * version render, idle → checking → up-to-date, idle → checking → available →
 * install → progress → installed, and the error → retry loop. The updater
 * plugin is desktop-only; we mock `@tauri-apps/api/core` so the store's calls
 * to `invoke` and `Channel` are captured.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, waitFor, fireEvent, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// A shared handle to the last-created Channel so the test can drive events.
let lastChannel: { onmessage?: (msg: unknown) => void } | null = null;

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invokeMock(...a),
  Channel: class {
    onmessage?: (msg: unknown) => void;
    constructor() {
      lastChannel = this;
    }
  },
}));
// The store also uses ../lib/invoke which delegates to the mock in browser
// mode; here we're not in Tauri, so we short-circuit via isTauri override.
vi.mock("../../lib/runtime", () => ({
  isTauri: () => true,
  isBrowser: () => false,
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({ enable: vi.fn().mockResolvedValue(undefined), disable: vi.fn().mockResolvedValue(undefined), isEnabled: vi.fn().mockResolvedValue(false) }));

import { UpdatesSettings } from "../UpdatesSettings";
import { useAppUpdate } from "../../hooks/useAppUpdate";

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  lastChannel = null;
  // Reset the shared store between tests.
  useAppUpdate.setState({
    phase: "idle",
    available: null,
    progress: null,
    error: null,
    dismissedVersion: null,
  });
});

describe("Settings — Updates card", () => {
  it("renders the current version", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "current_version") return Promise.resolve("0.3.0");
      return Promise.resolve(null);
    });
    render(<UpdatesSettings />, { wrapper: wrapper() });
    // Wait for the useCurrentVersion query to resolve, then read the label.
    await waitFor(() =>
      expect(screen.getByTestId("current-version")).toHaveTextContent("v0.3.0"),
    );
  });

  it("idle → checking → up-to-date when the endpoint reports no update", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "current_version") return Promise.resolve("0.3.0");
      if (cmd === "check_for_update") return Promise.resolve(null);
      return Promise.resolve(null);
    });
    render(<UpdatesSettings />, { wrapper: wrapper() });
    const btn = await screen.findByTestId("check-for-updates");
    fireEvent.click(btn);
    await waitFor(() =>
      expect(screen.getByTestId("update-uptodate")).toBeInTheDocument(),
    );
  });

  it("shows the Install button and progress bar when an update is available", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "current_version") return Promise.resolve("0.3.0");
      if (cmd === "check_for_update") {
        return Promise.resolve({
          version: "0.4.0",
          currentVersion: "0.3.0",
          notes: "New features",
          date: null,
        });
      }
      if (cmd === "install_update") {
        // Simulate download events on the channel we captured.
        setTimeout(() => {
          lastChannel?.onmessage?.({ event: "Started", data: { contentLength: 1000 } });
          lastChannel?.onmessage?.({ event: "Progress", data: { chunkLength: 500 } });
        }, 0);
        return new Promise((resolve) => setTimeout(resolve, 10));
      }
      return Promise.resolve(null);
    });
    render(<UpdatesSettings />, { wrapper: wrapper() });
    fireEvent.click(await screen.findByTestId("check-for-updates"));

    const installBtn = await screen.findByTestId("install-update");
    expect(screen.getByText(/0\.4\.0 is available/i)).toBeInTheDocument();
    expect(screen.getByText(/New features/)).toBeInTheDocument();

    fireEvent.click(installBtn);
    await waitFor(() =>
      expect(screen.getByTestId("update-progress")).toBeInTheDocument(),
    );
    await waitFor(() =>
      expect(screen.getByTestId("update-installed")).toBeInTheDocument(),
    );
  });

  it("shows an error + Retry when the check throws", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "current_version") return Promise.resolve("0.3.0");
      if (cmd === "check_for_update") return Promise.reject(new Error("network"));
      return Promise.resolve(null);
    });
    render(<UpdatesSettings />, { wrapper: wrapper() });
    await act(async () => {
      fireEvent.click(await screen.findByTestId("check-for-updates"));
    });
    const err = await screen.findByTestId("update-error");
    expect(err).toHaveTextContent(/network/i);
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });
});
