/**
 * Onboarding — Connection choice step ("How do you want to connect?").
 *
 * The first-run flow now includes a connection choice before wallet creation.
 * This test verifies each option persists the right settings (the connection
 * settings through the confirmed `update_protected_settings`) and advances to
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
  // the settings writes are no-ops, everything else returns a generic success.
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
    if (cmd === "update_setting" || cmd === "update_protected_settings") {
      return Promise.resolve(undefined);
    }
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

  // The connection settings are protected: they go to the backend only in
  // `update_protected_settings` (confirmed in the secure window), and only
  // those that differ from what is stored.
  it("local node selection over the stored defaults asks nothing and advances", async () => {
    render(<Onboarding />, { wrapper: wrapper() });

    fireEvent.click(screen.getByTestId("select-local-button"));

    await waitFor(() => screen.getByText(/Create a new wallet/i));
    expect(invokeMock).not.toHaveBeenCalledWith("update_protected_settings", expect.anything());
    expect(invokeMock).not.toHaveBeenCalledWith("update_setting", expect.anything());
  });

  it("local node selection sends the changed connection settings to be confirmed", async () => {
    loadSettings({ onboarding_complete: "false", chain_source: "remote_node", node_mode: "spv" });
    render(<Onboarding />, { wrapper: wrapper() });

    fireEvent.click(screen.getByTestId("select-local-button"));

    await waitFor(() => screen.getByText(/Create a new wallet/i));
    expect(invokeMock).toHaveBeenCalledWith("update_protected_settings", {
      changes: [
        { key: "chain_source", value: "local_node" },
        { key: "node_mode", value: "full" },
      ],
    });
    expect(invokeMock).not.toHaveBeenCalledWith("update_setting", expect.anything());
  });

  it("SPV selection sends node_mode to be confirmed and advances to wallet form", async () => {
    render(<Onboarding />, { wrapper: wrapper() });

    fireEvent.click(screen.getByTestId("select-spv-button"));

    await waitFor(() => screen.getByText(/Create a new wallet/i));
    expect(invokeMock).toHaveBeenCalledWith("update_protected_settings", {
      changes: [{ key: "node_mode", value: "spv" }],
    });
    expect(invokeMock).not.toHaveBeenCalledWith("update_setting", expect.anything());
  });

  // Each option: a declined confirmation saves nothing and keeps the step.
  async function expectDeclinedStay() {
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_protected_settings", expect.anything()),
    );
    // Give a wrong advance the chance to render before asserting it did not.
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.getByText(/How do you want to connect\?/i)).toBeInTheDocument();
    expect(screen.queryByText(/Create a new wallet/i)).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith("update_setting", expect.anything());
  }

  function declineConfirmations() {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "update_protected_settings") return Promise.reject("Confirmation declined");
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
      return Promise.resolve(undefined);
    });
  }

  it("a declined confirmation of SPV saves nothing and stays on the connection step", async () => {
    declineConfirmations();
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.click(screen.getByTestId("select-spv-button"));
    await expectDeclinedStay();
  });

  it("a declined confirmation of the local node saves nothing and stays on the step", async () => {
    loadSettings({ onboarding_complete: "false", chain_source: "remote_node" });
    declineConfirmations();
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.click(screen.getByTestId("select-local-button"));
    await expectDeclinedStay();
  });

  it("a declined confirmation of a remote node saves nothing and stays on the step", async () => {
    declineConfirmations();
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.change(screen.getByTestId("remote-url-input"), {
      target: { value: "https://node.example.com:12037" },
    });
    fireEvent.click(screen.getByTestId("test-connection-button"));
    await waitFor(() => expect(screen.getByTestId("select-remote-button")).not.toBeDisabled());
    fireEvent.click(screen.getByTestId("select-remote-button"));
    await expectDeclinedStay();
  });

  it("a value the backend refuses is shown and keeps the connection step", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "update_protected_settings"
        ? Promise.reject("Invalid input: the node is refused")
        : Promise.resolve(undefined),
    );
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.click(screen.getByTestId("select-spv-button"));
    expect(await screen.findByTestId("connection-save-error")).toHaveTextContent(
      "the node is refused",
    );
    expect(screen.getByTestId("select-spv-button")).toBeInTheDocument();
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

    // Click continue: the changed connection settings go in one confirmed
    // call (node_mode "full", an empty API key and no remote sending are
    // what is stored already, so they are not sent).
    fireEvent.click(continueBtn);
    await waitFor(() => screen.getByText(/Create a new wallet/i));
    expect(invokeMock).toHaveBeenCalledWith("update_protected_settings", {
      changes: [
        { key: "chain_source", value: "remote_node" },
        { key: "node_rpc_url", value: "http://example.com:12037" },
      ],
    });
    expect(invokeMock).not.toHaveBeenCalledWith("update_setting", expect.anything());
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
    expect(invokeMock).not.toHaveBeenCalledWith("update_protected_settings", expect.anything());
  });

  // G1: the connection step now lifts the network picker up front and threads
  // it into `check_node_connection`, so the backend can compare the node's
  // chain against the network the user just picked — even before any wallet
  // profile exists. This test pins that end-to-end: pick a network, probe a
  // node on a different chain, warning shows, Continue stays disabled.
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
    expect(invokeMock).not.toHaveBeenCalledWith("update_protected_settings", expect.anything());
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

  // G1: the picked network is passed to the backend as `expectedNetwork`.
  it("threads the selected network into the connection probe", async () => {
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.change(screen.getByTestId("onboarding-network-select"), {
      target: { value: "testnet" },
    });
    fireEvent.change(screen.getByTestId("remote-url-input"), {
      target: { value: "https://node.example.com:13037" },
    });
    fireEvent.click(screen.getByTestId("test-connection-button"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "check_node_connection",
        expect.objectContaining({ expectedNetwork: "testnet" }),
      ),
    );
  });

  // G1: changing the network after a green probe must invalidate it — a node
  // proven on mainnet is not proven on testnet.
  it("changing the network invalidates a prior successful probe", async () => {
    render(<Onboarding />, { wrapper: wrapper() });
    fireEvent.change(screen.getByTestId("remote-url-input"), {
      target: { value: "https://node.example.com:12037" },
    });
    fireEvent.click(screen.getByTestId("test-connection-button"));
    await waitFor(() => expect(screen.getByTestId("connection-success")).toBeInTheDocument());
    expect(screen.getByTestId("select-remote-button")).not.toBeDisabled();

    fireEvent.change(screen.getByTestId("onboarding-network-select"), {
      target: { value: "regtest" },
    });
    expect(screen.queryByTestId("connection-success")).toBeNull();
    expect(screen.getByTestId("select-remote-button")).toBeDisabled();
  });
});
