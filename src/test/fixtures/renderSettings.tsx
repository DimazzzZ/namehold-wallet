import { render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";
import { Settings } from "../../components/Settings";

/**
 * The provider shell every Settings test needs: a fresh QueryClient with
 * retries off (so a rejected command surfaces immediately instead of being
 * retried past the assertion) inside a MemoryRouter, which Settings requires
 * for its links.
 *
 * Import order matters: a test file's `vi.mock` calls are hoisted above its
 * imports, so the mocks are registered before this module pulls in Settings.
 * The Tauri plugin mocks themselves cannot move here — `vi.mock` is hoisted
 * per file and a factory living in an imported module would not be
 * initialised in time.
 */
export function settingsWrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

/** Render `<Settings />` inside {@link settingsWrapper}. */
export function renderSettings() {
  return render(<Settings />, { wrapper: settingsWrapper() });
}
