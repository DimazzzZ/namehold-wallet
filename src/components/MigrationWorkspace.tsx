import { PageHeader } from "./ui/PageHeader";
import { NamebaseDashboard } from "./NamebaseDashboard";

/**
 * Move from Namebase. The legacy platform closed on 2026-10-01, so this now
 * explains the shutdown and keeps the CSV history import; the live views only
 * render if a session somehow still answers.
 */
export function MigrationWorkspace() {
  return (
    <div>
      <PageHeader
        title="Migration"
        subtitle="Legacy Namebase has shut down. Import history you exported earlier."
      />
      <NamebaseDashboard />
    </div>
  );
}
