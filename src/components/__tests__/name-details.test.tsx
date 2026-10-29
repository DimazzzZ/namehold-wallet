import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent } from "@testing-library/react";
import { NameDetails } from "../name-actions/NameDetails";
import { ReadOnlyDnsRecords } from "../name-actions/ReadOnlyDnsRecords";
import { makeNodeStatus } from "../../test/fixtures/nodeStatus";
import type { HsdName, NameResource } from "../../types";

// NameDetails is the name page's Details card; ReadOnlyDnsRecords is the DNS
// tab's view for a name this wallet can't edit. Together they keep the
// read-only coverage (heights, owner UTXO, closed-auction values, DNS records)
// that used to live on NameInfoModal.

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("../../data/icann-gtld-2026.json", async () => ({
  default: (await import("../../test/fixtures/icann-list")).ICANN_FIXTURE,
}));
vi.mock("../../lib/openExternal", async (orig) => ({
  ...(await orig<typeof import("../../lib/openExternal")>()),
  openExternal: vi.fn(),
}));
import { openExternal } from "../../lib/openExternal";
import { ICANN_LIST_FETCHED_AT, icannCollision } from "../../lib/icann";

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
    expect(screen.getByText("Opened")).toBeInTheDocument();
    expect(screen.getByText("block 100")).toBeInTheDocument();
    expect(screen.getByText("Renewed")).toBeInTheDocument();
    expect(screen.getByText("block 200")).toBeInTheDocument();
  });

  it("says nothing is on-chain yet for an unopened name", () => {
    render(
      <NameDetails
        name="example"
        profileId="p1"
        info={baseInfo({ state: "AVAILABLE", height: null, renewal: null, owner: null })}
      />,
      { wrapper: wrapper() },
    );
    expect(screen.getByText("Nothing on-chain yet.")).toBeInTheDocument();
  });

  it("renders Paid price + High bid for a CLOSED name with values", () => {
    render(
      <NameDetails
        name="example"
        profileId="p1"
        info={baseInfo({ state: "CLOSED", value: 1_000_000, highest: 2_000_000 })}
      />,
      { wrapper: wrapper() },
    );
    expect(screen.getByText(/Paid price/)).toBeInTheDocument();
    expect(screen.getByText("High bid")).toBeInTheDocument();
  });

  it("shows expiry in blocks only on regtest, and as a date on mainnet", () => {
    const info = baseInfo({ stats: { blocksUntilExpire: 1000 } });
    const { unmount } = render(
      <NameDetails name="example" profileId="p1" info={info} network="regtest" />,
      { wrapper: wrapper() },
    );
    expect(screen.getByText("in 1,000 blocks")).toBeInTheDocument();
    unmount();
    render(<NameDetails name="example" profileId="p1" info={info} network="mainnet" />, {
      wrapper: wrapper(),
    });
    expect(screen.getByText(/· in 1,000 blocks/)).toBeInTheDocument();
  });

  it("hides owner UTXO when owner is null", () => {
    render(<NameDetails name="example" profileId="p1" info={baseInfo({ owner: null })} />, {
      wrapper: wrapper(),
    });
    expect(screen.queryByText("Technical")).not.toBeInTheDocument();
  });

  it("shows owner UTXO behind the collapsed Technical disclosure", () => {
    render(<NameDetails name="example" profileId="p1" info={baseInfo()} />, {
      wrapper: wrapper(),
    });
    expect(screen.queryByText("abc:0")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("Technical"));
    expect(screen.getByText("Owner UTXO")).toBeInTheDocument();
    expect(screen.getByText("abc:0")).toBeInTheDocument();
  });
});

describe("ReadOnlyDnsRecords", () => {
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
    render(<ReadOnlyDnsRecords name="example" profileId="p1" />, { wrapper: wrapper() });
    expect(await screen.findByText(/TTL:/)).toBeInTheDocument();
    expect(screen.getByText("DNS records")).toBeInTheDocument();
    expect(screen.getByText("3600s")).toBeInTheDocument();
    expect(screen.getByTestId("name-info-dns-table")).toBeInTheDocument();
  });

  it("shows 'Requires a synced node' when node is not live", async () => {
    route({ nodeLive: false });
    render(<ReadOnlyDnsRecords name="example" profileId="p1" />, { wrapper: wrapper() });
    expect(await screen.findByTestId("name-info-dns-no-node")).toBeInTheDocument();
    expect(screen.getByText(/Requires a synced local node/)).toBeInTheDocument();
  });
});

describe("NameDetails — a name bought on the market", () => {
  // Reported live: .ferratorr was bought on the market for 250 HNS, and Details
  // read "High bid 0.01 HNS · Paid price (2nd-price) 0 HNS" — its original
  // auction (one bid, so a second price of 0) — with nothing about the 250.
  it("leads with the market price and labels the rest as the auction's", async () => {
    route();
    render(
      <NameDetails
        name="ferratorr"
        profileId="p1"
        info={baseInfo({
          name: "ferratorr",
          value: 0,
          highest: 10_000,
          purchase: { priceDoos: 250_000_000, height: 353_400 },
        })}
      />,
      { wrapper: wrapper() },
    );
    const card = await screen.findByTestId("name-details");
    expect(screen.getByTestId("name-market-purchase")).toHaveTextContent(
      /Bought on the market\s*250 HNS/,
    );
    expect(card).toHaveTextContent(/Auction winner's bid\s*0\.01 HNS/);
    expect(card).toHaveTextContent(/Auction price \(2nd-price\)\s*nothing — no second bid/);
    expect(card).not.toHaveTextContent(/\b0 HNS/);
  });

  it("never shows a 0 HNS second price, bought or not", async () => {
    route();
    render(
      <NameDetails name="solo" profileId="p1" info={baseInfo({ value: 0, highest: 10_000 })} />,
      {
        wrapper: wrapper(),
      },
    );
    const card = await screen.findByTestId("name-details");
    expect(card).toHaveTextContent(/Paid price \(2nd-price\)\s*nothing — no second bid/);
    expect(card).not.toHaveTextContent(/\b0 HNS/);
    expect(screen.queryByTestId("name-market-purchase")).toBeNull();
  });
});

describe("NameDetails — a name with no auction", () => {
  it("shows no price at all for a claimed name (no bid, value 0)", async () => {
    route();
    render(
      <NameDetails name="claimed" profileId="p1" info={baseInfo({ value: 0, highest: 0 })} />,
      {
        wrapper: wrapper(),
      },
    );
    const card = await screen.findByTestId("name-details");
    expect(card).not.toHaveTextContent(/2nd-price/);
    expect(card).not.toHaveTextContent(/no second bid/);
  });
});

describe("NameDetails — ICANN 2026 round row (spec R6)", () => {
  const agent = icannCollision("agent")!;
  const date = new Date(ICANN_LIST_FETCHED_AT).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  });

  it("names the count, three applicants and the rest, with the list date", () => {
    render(<NameDetails name="agent" profileId="p1" info={baseInfo({ name: "agent" })} />, {
      wrapper: wrapper(),
    });
    const row = screen.getByTestId("name-icann");
    expect(row).toHaveTextContent("ICANN 2026 round");
    expect(row).toHaveTextContent(`${agent.applications.length} applications`);
    for (const a of agent.applications.slice(0, 3)) expect(row).toHaveTextContent(a.applicant);
    expect(row).toHaveTextContent(`and ${agent.applications.length - 3} more`);
    expect(row).not.toHaveTextContent(agent.applications[3]!.applicant);
    expect(screen.getAllByRole("button", { name: /Agent Applicant/ })).toHaveLength(3);
    expect(row).toHaveTextContent(`ICANN list as of ${date}`);
    // Two lines: the count beside the label, the applicants on a line of their
    // own under it, so a long list never squeezes the label (narrow card).
    const applicants = screen.getByTestId("name-icann-applicants");
    expect(applicants).toHaveTextContent(agent.applications[0]!.applicant);
    expect(applicants).not.toHaveTextContent("applications");
    expect(row.lastElementChild).toBe(applicants);
  });

  it("opens an applicant's application on ICANN's site", () => {
    render(<NameDetails name="agent" profileId="p1" info={baseInfo({ name: "agent" })} />, {
      wrapper: wrapper(),
    });
    const first = agent.applications[0]!;
    fireEvent.click(screen.getByRole("button", { name: first.applicant }));
    expect(openExternal).toHaveBeenCalledWith(
      `https://newgtldprogram-aps.icann.org/applications/${first.id}/summary`,
    );
  });

  it("names an applicant once however many applications they filed", () => {
    render(<NameDetails name="apex" profileId="p1" info={baseInfo({ name: "apex" })} />, {
      wrapper: wrapper(),
    });
    const row = screen.getByTestId("name-icann");
    expect(row).toHaveTextContent("4 applications");
    expect(screen.getByTestId("name-icann-applicants")).toHaveTextContent("Apex Co");
    expect(row).not.toHaveTextContent("more");
    expect(screen.getAllByRole("button", { name: "Apex Co" })).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Apex Co" }));
    expect(openExternal).toHaveBeenCalledWith(
      "https://newgtldprogram-aps.icann.org/applications/AP1-T1/summary",
    );
  });

  it("comes before the market purchase row", () => {
    render(
      <NameDetails
        name="agent"
        profileId="p1"
        info={baseInfo({ name: "agent", purchase: { priceDoos: 250_000_000, height: 352_642 } })}
      />,
      { wrapper: wrapper() },
    );
    const rows = screen.getByTestId("name-details").querySelectorAll("[data-testid]");
    const ids = [...rows].map((r) => r.getAttribute("data-testid"));
    expect(ids).toContain("name-icann");
    expect(ids).toContain("name-market-purchase");
    expect(ids.indexOf("name-icann")).toBeLessThan(ids.indexOf("name-market-purchase"));
  });

  it("renders the card with only this row for a colliding name without on-chain info", () => {
    render(<NameDetails name="agent" profileId="p1" info={null} />, { wrapper: wrapper() });
    expect(screen.getByTestId("name-details")).toBeInTheDocument();
    expect(screen.getByTestId("name-icann")).toBeInTheDocument();
    // The row is about the string; the chain still has nothing (spec R6).
    expect(screen.getByText("Nothing on-chain yet.")).toBeInTheDocument();
  });

  it("has no row for a name nobody applied for, and no card without info", () => {
    const { container, unmount } = render(
      <NameDetails name="zzzz-not-applied" profileId="p1" info={baseInfo()} />,
      { wrapper: wrapper() },
    );
    expect(screen.queryByTestId("name-icann")).toBeNull();
    unmount();
    const r2 = render(<NameDetails name="zzzz-not-applied" profileId="p1" info={null} />, {
      wrapper: wrapper(),
    });
    expect(r2.container.firstChild).toBeNull();
    expect(container.firstChild).toBeNull();
  });
});
