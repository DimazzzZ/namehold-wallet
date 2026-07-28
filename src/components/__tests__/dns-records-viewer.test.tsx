import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Task 2 viewer bug fix: the backend emits `keyTag` on DS records, but the
// `ResourceRecord` interface previously declared `hash`, so DS rows rendered
// "undefined" for the key tag. Verify the fix renders the real keyTag value.

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { DnsRecords } from "../DnsRecords";

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("DnsRecords viewer — DS keyTag (Task 2)", () => {
  it("renders the DS record's keyTag value, not undefined", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_assets") {
        return Promise.resolve([
          { tld: "namehold", status: "finalized_owned" },
        ]);
      }
      if (cmd === "get_resource") {
        return Promise.resolve({
          name: "namehold",
          state: "CLOSED",
          height: 100,
          data: {
            records: [
              { type: "DS", keyTag: 12345, algorithm: 8, digestType: 2, digest: "ABCDEF01" },
            ],
          },
        });
      }
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });

    render(<DnsRecords />, { wrapper: wrapper() });

    // Wait for the owned-name option to actually be in the DOM before
    // selecting it — the select renders with only "-- Select --" until the
    // `list_assets` query resolves.
    await screen.findByText(".namehold");
    const select = screen.getByLabelText("Select Name");
    fireEvent.change(select, { target: { value: "namehold" } });
    fireEvent.click(screen.getByText("Fetch Records"));

    await waitFor(() => {
      expect(screen.getByText("12345 8 2 ABCDEF01")).toBeInTheDocument();
    });
    expect(screen.queryByText(/undefined/)).not.toBeInTheDocument();
  });
});

describe("DnsRecords viewer — canonical table design", () => {
  it("the records table follows the unified table contract", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_assets") {
        return Promise.resolve([{ tld: "namehold", status: "finalized_owned" }]);
      }
      if (cmd === "get_resource") {
        return Promise.resolve({
          name: "namehold",
          state: "CLOSED",
          height: 100,
          data: {
            records: [
              { type: "NS", ns: "ns1.example." },
              { type: "DS", keyTag: 12345, algorithm: 8, digestType: 2, digest: "ABCDEF01" },
            ],
          },
        });
      }
      return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    });

    render(<DnsRecords />, { wrapper: wrapper() });
    await screen.findByText(".namehold");
    fireEvent.change(screen.getByLabelText("Select Name"), { target: { value: "namehold" } });
    fireEvent.click(screen.getByText("Fetch Records"));
    await screen.findByText("12345 8 2 ABCDEF01");

    const { assertCanonicalTable } = await import("../../test/canonicalTable");
    const table = document.querySelector("table");
    expect(table).toBeTruthy();
    assertCanonicalTable(table as HTMLTableElement, { name: "DnsRecords" });
  });
});
