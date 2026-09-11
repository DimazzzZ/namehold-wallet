/**
 * Onboarding — Connection choice step ("How do you want to connect?").
 *
 * The first-run flow now includes a connection choice before wallet creation.
 * This test verifies each option persists the right settings and advances to
 * the wallet form.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { Onboarding } from "../Onboarding";
import { loadSettings } from "../../test/fixtures/settings";

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  // Route by command name: check_node_connection returns the probe result,
  // update_setting is a no-op, everything else returns a generic success.
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "check_node_connection") {
      return Promise.resolve({
        reachable: true,
        height: 100,
        headers: 100,
        synced: true,
        network: "main",
        networkMatches: null,
        error: null,
      });
    }
    if (cmd === "update_setting") return Promise.resolve(undefined);
    return Promise.resolve({ id: "p1", label: "Primary" });
  });
  // `saveAll` early-returns when settings are null, so seed a full settings
  // object (mirrors what `load()` produces from DEFAULT_SETTINGS). This flow
  // starts before onboarding is complete, unlike most other component tests.
  loadSettings({ onboarding_complete: "false" });
});

describe("Onboarding — Connection choice", () => {
  it("renders the connection choice step first", async () => {
    render(<Onboarding />, { wrapper: wrapper() });
    expect(screen.getByText(/How do you want to connect\?/i)).toBeInTheDocument();
    expect(screen.getByTestId("select-local-button")).toBeInTheDocument();
    expect(screen.getByTestId("select-spv-button")).toBeInTheDocument();
  });

  it("local node selection persists chain_source and advances to wallet form", async () => {
    render(<Onboarding />, { wrapper: wrapper() });

    fireEvent.click(screen.getByTestId("select-local-button"));

    // Should advance to wallet form.
    await waitFor(() => screen.getByText(/Create a new wallet/i));
    expect(screen.getByText(/Create a new wallet/i)).toBeInTheDocument();

    // Verify settings were persisted.
    expect(invokeMock).toHaveBeenCalledWith("update_setting", {
      key: "chain_source",
      value: "local_node",
    });
    expect(invokeMock).toHaveBeenCalledWith("update_setting", {
      key: "node_mode",
      value: "full",
    });
  });

  it("SPV selection persists node_mode and advances to wallet form", async () => {
    render(<Onboarding />, { wrapper: wrapper() });

    fireEvent.click(screen.getByTestId("select-spv-button"));

    // Should advance to wallet form.
    await waitFor(() => screen.getByText(/Create a new wallet/i));
    expect(screen.getByText(/Create a new wallet/i)).toBeInTheDocument();

    // Verify settings were persisted.
    expect(invokeMock).toHaveBeenCalledWith("update_setting", {
      key: "chain_source",
      value: "local_node",
    });
    expect(invokeMock).toHaveBeenCalledWith("update_setting", {
      key: "node_mode",
      value: "spv",
    });
  });

  it("remote node requires test before continuing", async () => {
    render(<Onboarding />, { wrapper: wrapper() });

    // Fill in the remote URL.
    const urlInput = screen.getByTestId("remote-url-input") as HTMLInputElement;
    fireEvent.change(urlInput, { target: { value: "http://example.com:12037" } });

    // Try to continue without testing — should be blocked.
    const continueBtn = screen.getByTestId("select-remote-button");
    expect(continueBtn).toBeDisabled();

    // Mock a successful connection check.
    invokeMock.mockResolvedValueOnce({
      reachable: true,
      height: 100,
      headers: 100,
      synced: true,
      network: "main",
      networkMatches: null,
      error: null,
    });

    // Test the connection.
    fireEvent.click(screen.getByTestId("test-connection-button"));
    await waitFor(() => expect(screen.getByTestId("connection-success")).toBeInTheDocument());

    // Now the continue button should be enabled.
    expect(continueBtn).not.toBeDisabled();

    // Click continue and verify settings are persisted.
    fireEvent.click(continueBtn);
    await waitFor(() => screen.getByText(/Create a new wallet/i));
    expect(invokeMock).toHaveBeenCalledWith("update_setting", {
      key: "chain_source",
      value: "remote_node",
    });
    expect(invokeMock).toHaveBeenCalledWith("update_setting", {
      key: "node_rpc_url",
      value: "http://example.com:12037",
    });
    expect(invokeMock).toHaveBeenCalledWith("update_setting", {
      key: "node_mode",
      value: "full",
    });
  });

  it("failed probe shows the node's reason and keeps Continue disabled", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "check_node_connection"
        ? Promise.resolve({
            reachable: false,
            height: null,
            headers: null,
            synced: false,
            network: null,
            networkMatches: null,
            error: "connection refused",
          })
        : Promise.resolve(undefined),
    );
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.change(screen.getByTestId("remote-url-input"), {
      target: { value: "https://node.example.com:12037" },
    });
    fireEvent.click(screen.getByTestId("test-connection-button"));
    await waitFor(() =>
      expect(screen.getByTestId("connection-error")).toHaveTextContent("connection refused"),
    );
    expect(screen.getByTestId("select-remote-button")).toBeDisabled();
    expect(invokeMock).not.toHaveBeenCalledWith("update_setting", expect.anything());
  });

  it("a reachable node on the wrong network keeps Continue disabled", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "check_node_connection"
        ? Promise.resolve({
            reachable: true,
            height: 100,
            headers: 100,
            synced: true,
            network: "testnet",
            networkMatches: false,
            error: null,
          })
        : Promise.resolve(undefined),
    );
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.change(screen.getByTestId("remote-url-input"), {
      target: { value: "https://node.example.com:12037" },
    });
    fireEvent.click(screen.getByTestId("test-connection-button"));
    // Reachable — the green line shows — but the mismatch warning must appear
    // and Continue must stay closed.
    await waitFor(() =>
      expect(screen.getByTestId("connection-network-mismatch")).toHaveTextContent(/mismatch/i),
    );
    expect(screen.getByTestId("connection-success")).toBeInTheDocument();
    expect(screen.getByTestId("select-remote-button")).toBeDisabled();
    expect(invokeMock).not.toHaveBeenCalledWith("update_setting", expect.anything());
  });

  it("editing the URL after a successful probe invalidates it and disables Continue", async () => {
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.change(screen.getByTestId("remote-url-input"), {
      target: { value: "https://a.example.com:12037" },
    });
    fireEvent.click(screen.getByTestId("test-connection-button"));
    await waitFor(() => expect(screen.getByTestId("connection-success")).toBeInTheDocument());
    const continueBtn = screen.getByTestId("select-remote-button");
    expect(continueBtn).not.toBeDisabled();

    fireEvent.change(screen.getByTestId("remote-url-input"), {
      target: { value: "https://b.example.com:12037" },
    });
    expect(screen.queryByTestId("connection-success")).toBeNull();
    expect(continueBtn).toBeDisabled();
  });

  it("editing the API key after a successful probe invalidates it too", async () => {
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.change(screen.getByTestId("remote-url-input"), {
      target: { value: "https://a.example.com:12037" },
    });
    fireEvent.click(screen.getByTestId("test-connection-button"));
    await waitFor(() => expect(screen.getByTestId("connection-success")).toBeInTheDocument());

    fireEvent.change(screen.getByTestId("remote-api-key-input"), { target: { value: "k2" } });
    expect(screen.queryByTestId("connection-success")).toBeNull();
    expect(screen.getByTestId("select-remote-button")).toBeDisabled();
  });
});
