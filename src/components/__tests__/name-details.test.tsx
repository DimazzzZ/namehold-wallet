import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { NameDetails } from "../name-actions/NameDetails";
import { useNameRecords } from "../../queries/read";
import { useNodeLive } from "../../queries/node";
import type { HsdName } from "../../types";

// NameDetails is the read-only body extracted from the former NameInfoModal
// when the two name modals were unified into NameActionsModal. These tests
// preserve the read-only rendering coverage (heights, transfer, owner UTXO,
// closed-auction values, DNS records) that used to live on NameInfoModal.
vi.mock("../../queries/read");
vi.mock("../../queries/node");

const mockUseNameRecords = vi.mocked(useNameRecords);
const mockUseNodeLive = vi.mocked(useNodeLive);

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
  } as HsdName;
}

beforeEach(() => {
  vi.clearAllMocks();
  mockUseNameRecords.mockReturnValue({
    data: { records: [] },
    isLoading: false,
    isError: false,
  } as any);
  mockUseNodeLive.mockReturnValue(true);
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

  it("renders DNS records with TTL when node is live", () => {
    mockUseNameRecords.mockReturnValue({
      data: {
        records: [
          { type: "NS", ns: "ns1.example." },
          { type: "TXT", txt: ["v=spf1 -all"] },
        ],
        ttl: 3600,
      },
      isLoading: false,
      isError: false,
    } as any);
    render(<NameDetails name="example" profileId="p1" info={baseInfo()} />, {
      wrapper: wrapper(),
    });
    expect(screen.getByText("DNS Records")).toBeInTheDocument();
    expect(screen.getByText(/TTL:/)).toBeInTheDocument();
    expect(screen.getByText("3600s")).toBeInTheDocument();
    expect(screen.getByTestId("name-info-dns-table")).toBeInTheDocument();
  });

  it("shows 'Requires a synced node' when node is not live", () => {
    mockUseNodeLive.mockReturnValue(false);
    render(<NameDetails name="example" profileId="p1" info={baseInfo()} />, {
      wrapper: wrapper(),
    });
    expect(screen.getByTestId("name-info-dns-no-node")).toBeInTheDocument();
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
