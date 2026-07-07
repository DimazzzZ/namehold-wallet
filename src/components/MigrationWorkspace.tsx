import { PageHeader } from "./ui/PageHeader";
import { NamebaseDashboard } from "./NamebaseDashboard";

/**
 * Move from Namebase — single screen driven entirely by live Namebase data.
 * No reconciliation, no inventory comparison, no extra tabs.
 */
export function MigrationWorkspace() {
  return (
    <div>
      <PageHeader
        title="Migration"
        subtitle="View live Namebase holdings, transfer domains, and track activity."
      />
      <NamebaseDashboard />
    </div>
  );
}
