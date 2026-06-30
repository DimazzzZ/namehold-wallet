import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";
import type { Settings as AppSettings } from "../../types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { useSettingsStore } from "../../stores/settings";
import { Settings } from "../Settings";

function makeSettings(overrides: Partial<AppSettings> = {}): AppSettings {
  return {
    hsd_wallet_api_url: "http://127.0.0.1:12039",
    hsd_node_api_url: "http://127.0.0.1:12037",
    hsd_api_key: "",
    hsd_wallet_id: "primary",
    hsd_network: "mainnet",
    hsd_prefix: "",
    write_mode: "false",
    connection_mode: "local_managed_hsd",
    external_read_provider: "none",
    external_read_api_url: "https://hnsfans.com",
    external_read_watch_addresses: "[]",
    external_read_watch_names: "[]",
    remote_hsd_label: "",
    trusted_remote_hsd: "false",
    future_signer_mode: "none",
    advanced_mode: "false",
    onboarding_complete: "false",
    ...overrides,
  };
}

function seedSettings(overrides: Partial<AppSettings> = {}) {
  useSettingsStore.setState({
    settings: makeSettings(overrides),
    loaded: true,
    passphrase: "",
  });
}

function wrapper() {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <MemoryRouter initialEntries={["/settings"]}>{children}</MemoryRouter>;
  };
}

beforeEach(() => {
  useSettingsStore.setState({ settings: null, loaded: false, passphrase: "" });
});

describe("Settings provider/connection modes", () => {
  it("renders the connection mode selector with all four modes", () => {
    seedSettings();
    render(<Settings />, { wrapper: wrapper() });
    expect(screen.getByText(/Connection Mode/)).toBeInTheDocument();
    expect(
      screen.getByText(/Local managed hsd \(full read \+ write\)/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Remote hsd \(requires trust to write\)/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Auto fallback \(prefer hsd, fall back to external\)/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/External read-only \(no writes\)/),
    ).toBeInTheDocument();
  });

  it("does not show remote or external panels in local managed mode", () => {
    seedSettings({ connection_mode: "local_managed_hsd" });
    render(<Settings />, { wrapper: wrapper() });
    expect(
      screen.queryByText(/I trust this remote hsd for write operations/),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText(/External Read Provider/),
    ).not.toBeInTheDocument();
  });

  it("shows remote trust controls in remote mode", () => {
    seedSettings({ connection_mode: "remote_hsd" });
    render(<Settings />, { wrapper: wrapper() });
    expect(
      screen.getByText(/I trust this remote hsd for write operations/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Remote hsd Label/)).toBeInTheDocument();
    expect(
      screen.queryByText(/External Read Provider/),
    ).not.toBeInTheDocument();
  });

  it("shows external provider selector in external read-only mode", () => {
    seedSettings({ connection_mode: "external_read_only" });
    render(<Settings />, { wrapper: wrapper() });
    expect(screen.getByText(/External Read Provider/)).toBeInTheDocument();
    expect(
      screen.queryByText(/I trust this remote hsd for write operations/),
    ).not.toBeInTheDocument();
  });

  it("shows external provider selector in auto-fallback mode", () => {
    seedSettings({ connection_mode: "auto_fallback" });
    render(<Settings />, { wrapper: wrapper() });
    expect(screen.getByText(/External Read Provider/)).toBeInTheDocument();
  });

  it("reveals watch fields once a non-none external provider is chosen", () => {
    seedSettings({
      connection_mode: "external_read_only",
      external_read_provider: "hnsfans",
    });
    render(<Settings />, { wrapper: wrapper() });
    expect(screen.getByText(/External Read API URL/)).toBeInTheDocument();
    expect(screen.getByText(/Watch Addresses/)).toBeInTheDocument();
    expect(screen.getByText(/Watch Names/)).toBeInTheDocument();
  });

  it("switching mode via the selector updates the visible panel", () => {
    seedSettings({ connection_mode: "local_managed_hsd" });
    render(<Settings />, { wrapper: wrapper() });
    // Local managed: no external provider panel.
    expect(
      screen.queryByText(/External Read Provider/),
    ).not.toBeInTheDocument();

    // The connection mode <select> currently holds the local-managed value.
    const select = screen.getByDisplayValue(
      /Local managed hsd \(full read \+ write\)/,
    ) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "external_read_only" } });

    expect(screen.getByText(/External Read Provider/)).toBeInTheDocument();
  });
});
