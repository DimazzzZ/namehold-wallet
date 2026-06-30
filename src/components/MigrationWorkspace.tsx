import { useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import { PageHeader } from "./ui/PageHeader";
import { Tabs } from "./ui/Tabs";
import { NamebaseDashboard } from "./NamebaseDashboard";
import { MigrationAssistant } from "./MigrationAssistant";
import { SyncVerification } from "./SyncVerification";
import { TransfersView } from "./TransfersView";
import type { MigrationSectionKey, WorkspaceTab } from "../types";

const TABS: WorkspaceTab<MigrationSectionKey>[] = [
  { key: "namebase", label: "Namebase", description: "Namebase account & transfers" },
  { key: "transfers", label: "Transfers", description: "Track domain transfer status" },
  { key: "sync", label: "Sync & Verify", description: "Reconcile wallet vs inventory" },
];

const VALID_KEYS = new Set<MigrationSectionKey>(["namebase", "transfers", "sync"]);

export function MigrationWorkspace() {
  const [searchParams, setSearchParams] = useSearchParams();
  const rawTab = searchParams.get("tab") as MigrationSectionKey | null;
  const activeTab: MigrationSectionKey =
    rawTab && VALID_KEYS.has(rawTab) ? rawTab : "namebase";

  const content = useMemo(() => {
    switch (activeTab) {
      case "namebase":
        return (
          <>
            <MigrationAssistant />
            <NamebaseDashboard />
          </>
        );
      case "transfers":
        return <TransfersView />;
      case "sync":
        return <SyncVerification />;
      default:
        return (
          <>
            <MigrationAssistant />
            <NamebaseDashboard />
          </>
        );
    }
  }, [activeTab]);

  return (
    <div>
      <PageHeader
        title="Migration"
        subtitle="Track Namebase transfers and verify wallet ownership against your inventory."
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
