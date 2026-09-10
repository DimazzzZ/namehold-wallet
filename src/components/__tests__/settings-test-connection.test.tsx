/**
 * Settings — "Test connection" in the Node RPC block.
 *
 * The probe must send the typed URL and omit a blank key (the backend then
 * reuses the stored key for the saved URL — see check_node_connection), and a
 * successful result must be dropped as soon as the URL or key is edited so a
 * green "Connected" never describes a node that was not probed.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn().mockResolvedValue(false),
  requestPermission: vi.fn().mockResolvedValue("default"),
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({
  enable: vi.fn().mockResolvedValue(undefined),
  disable: vi.fn().mockResolvedValue(undefined),
  isEnabled: vi.fn().mockResolvedValue(false),
}));

import { Settings } from "../Settings";
import { loadSettings } from "../../test/fixtures/settings";
import { routeSettingsCommand } from "../../test/fixtures/settingsRoute";

const reachable = { reachable: true, height: 4242, headers: 4242, synced: true, network: "main", error: null };

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) =>
    cmd === "check_node_connection" ? Promise.resolve(reachable) : routeSettingsCommand(cmd),
  );
  loadSettings({ node_rpc_url: "https://node.example.com:12037", __has_node_rpc_api_key: "true" });
});

describe("Settings — Test connection", () => {
  it("probes the saved URL with no key when the key field is blank", async () => {
    render(<Settings />, { wrapper: wrapper() });
    fireEvent.click(await screen.findByTestId("test-connection-button"));
    await waitFor(() =>
      expect(screen.getByTestId("connection-success")).toHaveTextContent(/height 4242/),
    );
    expect(invokeMock).toHaveBeenCalledWith("check_node_connection", {
      url: "https://node.example.com:12037",
      api_key: undefined,
    });
  });

  it("drops a successful result when the URL is edited", async () => {
    render(<Settings />, { wrapper: wrapper() });
    fireEvent.click(await screen.findByTestId("test-connection-button"));
    await waitFor(() => expect(screen.getByTestId("connection-success")).toBeInTheDocument());

    fireEvent.change(screen.getByLabelText(/Node RPC URL/i), {
      target: { value: "https://other.example.com:12037" },
    });
    expect(screen.queryByTestId("connection-success")).toBeNull();
  });
});
