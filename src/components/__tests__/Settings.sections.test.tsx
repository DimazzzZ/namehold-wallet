import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Navigate, Route, Routes, useLocation } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn(), readText: vi.fn() }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn().mockResolvedValue(true),
  requestPermission: vi.fn().mockResolvedValue("granted"),
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({
  enable: vi.fn(),
  disable: vi.fn(),
  isEnabled: vi.fn().mockResolvedValue(false),
}));

import { Settings } from "../Settings";
import { loadSettings } from "../../test/fixtures/settings";
import { routeSettingsCommand } from "../../test/fixtures/settingsRoute";
import { parseSettingsSection, settingsPath } from "../../lib/settingsSections";

function Where() {
  const l = useLocation();
  return <div data-testid="where">{l.pathname + l.search}</div>;
}

function renderAt(path: string) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[path]}>{children}</MemoryRouter>
    </QueryClientProvider>
  );
  return render(
    <Routes>
      <Route path="/migration" element={<Navigate to="/settings?section=import" replace />} />
      <Route
        path="/settings"
        element={
          <>
            <Settings />
            <Where />
          </>
        }
      />
    </Routes>,
    { wrapper: Wrapper },
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => routeSettingsCommand(cmd));
  loadSettings();
});

describe("settingsSections", () => {
  it("parses ?section with the backup alias and a Connections fallback", () => {
    expect(parseSettingsSection("node")).toBe("node");
    expect(parseSettingsSection("backup")).toBe("import");
    expect(parseSettingsSection("nope")).toBe("connections");
    expect(parseSettingsSection(null)).toBe("connections");
    expect(settingsPath()).toBe("/settings");
    expect(settingsPath("advanced")).toBe("/settings?section=advanced");
  });
});

describe("Settings sections", () => {
  it("opens the section from the URL and marks it current", () => {
    renderAt("/settings?section=advanced");
    expect(screen.getByTestId("settings-section-advanced")).toHaveAttribute("aria-current", "page");
    expect(screen.getByTestId("settings-fee-rate")).toBeInTheDocument();
    expect(screen.queryByTestId("explorer-url-input")).toBeNull();
  });

  it("?section=backup opens Backup & import", () => {
    renderAt("/settings?section=backup");
    expect(screen.getByTestId("settings-section-import")).toHaveAttribute("aria-current", "page");
    expect(screen.getByTestId("export-bid-backup")).toBeInTheDocument();
  });

  it("/migration lands on Backup & import with the shut-down notice", async () => {
    renderAt("/migration");
    expect(screen.getByTestId("where")).toHaveTextContent("/settings?section=import");
    expect(await screen.findByTestId("namebase-shut-down")).toBeInTheDocument();
  });

  it("marks the section holding an error, and Show opens it", async () => {
    renderAt("/settings");
    fireEvent.change(screen.getByTestId("explorer-url-input"), { target: { value: "abc" } });
    fireEvent.click(screen.getByTestId("settings-section-notifications"));
    expect(screen.getByTestId("settings-section-error-connections")).toBeInTheDocument();
    expect(screen.getByTestId("settings-save-blocked")).toHaveTextContent(
      "Can’t save: Explorer URL must start with http:// or https://",
    );
    expect(screen.getByTestId("settings-save")).toBeDisabled();
    fireEvent.click(screen.getByTestId("settings-save-blocked-show"));
    await waitFor(() =>
      expect(screen.getByTestId("settings-section-connections")).toHaveAttribute(
        "aria-current",
        "page",
      ),
    );
    expect(screen.queryByTestId("settings-save-blocked-show")).toBeNull();
  });

  it("marks Advanced for a bad fee rate and clears it once fixed", () => {
    renderAt("/settings?section=advanced");
    fireEvent.change(screen.getByTestId("settings-fee-rate"), { target: { value: "x" } });
    expect(screen.getByTestId("settings-section-error-advanced")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("settings-fee-rate"), { target: { value: "" } });
    expect(screen.queryByTestId("settings-section-error-advanced")).toBeNull();
    expect(screen.getByTestId("settings-save")).toBeEnabled();
  });

  it("keeps unsaved edits across a section switch", () => {
    renderAt("/settings");
    fireEvent.change(screen.getByTestId("explorer-url-input"), {
      target: { value: "https://explorer.example" },
    });
    fireEvent.click(screen.getByTestId("settings-section-advanced"));
    fireEvent.click(screen.getByTestId("settings-section-connections"));
    expect(screen.getByTestId("explorer-url-input")).toHaveValue("https://explorer.example");
    expect(screen.getByTestId("settings-save")).toBeInTheDocument();
  });
});
