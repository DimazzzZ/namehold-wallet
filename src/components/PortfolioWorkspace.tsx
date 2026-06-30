import { useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import { PageHeader } from "./ui/PageHeader";
import { Tabs } from "./ui/Tabs";
import { TldInventory } from "./TldInventory";
import { Batches } from "./Batches";
import { Renewals } from "./Renewals";
import { DnsRecords } from "./DnsRecords";
import type { PortfolioSectionKey, WorkspaceTab } from "../types";

const TABS: WorkspaceTab<PortfolioSectionKey>[] = [
  { key: "inventory", label: "Inventory", description: "All imported TLDs" },
  { key: "batches", label: "Batches", description: "Migration groups" },
  { key: "renewals", label: "Renewals", description: "Expiry monitoring" },
  { key: "dns", label: "DNS", description: "DNS records" },
];

const VALID_KEYS = new Set<PortfolioSectionKey>([
  "inventory",
  "batches",
  "renewals",
  "dns",
]);

export function PortfolioWorkspace() {
  const [searchParams, setSearchParams] = useSearchParams();
  const rawTab = searchParams.get("tab") as PortfolioSectionKey | null;
  const activeTab: PortfolioSectionKey =
    rawTab && VALID_KEYS.has(rawTab) ? rawTab : "inventory";

  const content = useMemo(() => {
    switch (activeTab) {
      case "inventory":
        return <TldInventory />;
      case "batches":
        return <Batches />;
      case "renewals":
        return <Renewals />;
      case "dns":
        return <DnsRecords />;
      default:
        return <TldInventory />;
    }
  }, [activeTab]);

  return (
    <div>
      <PageHeader
        title="Portfolio"
        subtitle="Manage your Handshake TLD inventory, batches, renewals, and DNS."
      />
      <Tabs
        tabs={TABS}
        active={activeTab}
        onChange={(key) =>
          setSearchParams(
            (prev) => {
              prev.set("tab", key);
              return prev;
            },
            { replace: true },
          )
        }
        className="mb-5"
      />
      <div>{content}</div>
    </div>
  );
}
