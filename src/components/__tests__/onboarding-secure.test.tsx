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
  invokeMock.mockResolvedValue({ id: "p1", label: "Primary" });
});

describe("Onboarding (secure, non-custodial)", () => {
  it("create flow has NO mnemonic/passphrase inputs and delegates to the secure command", async () => {
    const { container } = render(<Onboarding />, { wrapper: wrapper() });

    fireEvent.click(screen.getByText(/Create a new wallet/i));
    // No secret entry surfaces in React.
    expect(container.querySelector("textarea")).toBeNull();
    expect(container.querySelector('input[type="password"]')).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Create in secure window/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "secure_create_wallet",
        expect.objectContaining({ network: expect.any(String) }),
      ),
    );
    // The legacy hsd wallet command must never be used.
    expect(invokeMock).not.toHaveBeenCalledWith("create_wallet", expect.anything());
  });

  it("import flow uses secure_import_wallet with mnemonic_hot and no React seed entry", async () => {
    const { container } = render(<Onboarding />, { wrapper: wrapper() });

    fireEvent.click(screen.getByText(/Import your wallet/i));
    expect(container.querySelector("textarea")).toBeNull();
    expect(container.querySelector('input[type="password"]')).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Import in secure window/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "secure_import_wallet",
        expect.objectContaining({ kind: "mnemonic_hot" }),
      ),
    );
  });
});
