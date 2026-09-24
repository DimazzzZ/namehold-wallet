import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { NameDetails } from "../name-actions/NameDetails";
import { makeNodeStatus } from "../../test/fixtures/nodeStatus";
import type { HsdName, NameResource } from "../../types";

// NameDetails is the read-only body extracted from the former NameInfoModal
// when the two name modals were unified into NameActionsModal. These tests
// preserve the read-only rendering coverage (heights, transfer, owner UTXO,
// closed-auction values, DNS records) that used to live on NameInfoModal.

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

/** Route the two commands the component reads: DNS records and node status. */
function route(o: { records?: NameResource | null; nodeLive?: boolean } = {}) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "read_name_records":
        return Promise.resolve(o.records ?? { records: [] });
      case "node_status":
        return Promise.resolve(
          makeNodeStatus({ read_source: (o.nodeLive ?? true) ? "local" : "explorer" }),
        );
      default:
        return Promise.reject(new Error(`unexpected command ${cmd}`));
    }
  });
}

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

function baseInfo(overrides: Partial<HsdName> = {}): HsdName {
  return {
    name: "example",
    state: "CLOSED",
    registered: true,
    expired: false,
    height: 100,
    renewal: 200,
    owner: { hash: "abc", index: 0 },
    value: null,
    highest: null,
    stats: null,
    transfer: 0,
    ...overrides,
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  route();
});

describe("NameDetails", () => {
  it("renders nothing when info is null", () => {
    const { container } = render(<NameDetails name="example" profileId="p1" info={null} />, {
      wrapper: wrapper(),
    });
    expect(container.querySelector('[data-testid="name-details"]')).toBeNull();
  });

  it("renders opened/renewed block heights", () => {
    render(<NameDetails name="example" profileId="p1" info={baseInfo()} />, {
      wrapper: wrapper(),
    });
    expect(screen.getByText("Opened:")).toBeInTheDocument();
    expect(screen.getByText("Block 100")).toBeInTheDocument();
    expect(screen.getByText("Renewed:")).toBeInTheDocument();
    expect(screen.getByText("Block 200")).toBeInTheDocument();
  });

  it("renders Paid price + Top bid for a CLOSED name with values", () => {
    render(
      <NameDetails
        name="example"
        profileId="p1"
        info={baseInfo({ state: "CLOSED", value: 1_000_000, highest: 2_000_000 })}
      />,
      { wrapper: wrapper() },
    );
    expect(screen.getByText(/Paid price/)).toBeInTheDocument();
    expect(screen.getByText(/Top bid/)).toBeInTheDocument();
  });

  it("hides owner UTXO row when owner is null", () => {
    render(<NameDetails name="example" profileId="p1" info={baseInfo({ owner: null })} />, {
      wrapper: wrapper(),
    });
    expect(screen.queryByText("Owner UTXO:")).not.toBeInTheDocument();
  });

  it("shows owner UTXO row when owner is present", () => {
    render(<NameDetails name="example" profileId="p1" info={baseInfo()} />, {
      wrapper: wrapper(),
    });
    expect(screen.getByText("Owner UTXO:")).toBeInTheDocument();
    expect(screen.getByText("abc:0")).toBeInTheDocument();
  });

  it("shows transfer status when transfer is non-zero", () => {
    render(<NameDetails name="example" profileId="p1" info={baseInfo({ transfer: 500 })} />, {
      wrapper: wrapper(),
    });
    expect(screen.getByText(/Transfer in progress/)).toBeInTheDocument();
  });

  it("renders DNS records with TTL when node is live", async () => {
    route({
      records: {
        records: [
          { type: "NS", ns: "ns1.example." },
          { type: "TXT", txt: ["v=spf1 -all"] },
        ],
        ttl: 3600,
      },
    });
    render(<NameDetails name="example" profileId="p1" info={baseInfo()} />, {
      wrapper: wrapper(),
    });
    expect(await screen.findByText(/TTL:/)).toBeInTheDocument();
    expect(screen.getByText("DNS Records")).toBeInTheDocument();
    expect(screen.getByText("3600s")).toBeInTheDocument();
    expect(screen.getByTestId("name-info-dns-table")).toBeInTheDocument();
  });

  it("shows 'Requires a synced node' when node is not live", async () => {
    route({ nodeLive: false });
    render(<NameDetails name="example" profileId="p1" info={baseInfo()} />, {
      wrapper: wrapper(),
    });
    expect(await screen.findByTestId("name-info-dns-no-node")).toBeInTheDocument();
    expect(screen.getByText(/Requires a synced local node/)).toBeInTheDocument();
  });

  it("suppresses its own DNS block when hideDnsRecords is set", () => {
    // Owned names: NameActionsModal's editable DnsRecordsEditor owns the
    // records, so NameDetails must not render a second (read-only) copy.
    render(<NameDetails name="example" profileId="p1" info={baseInfo()} hideDnsRecords />, {
      wrapper: wrapper(),
    });
    expect(screen.queryByText("DNS Records")).not.toBeInTheDocument();
  });
});
