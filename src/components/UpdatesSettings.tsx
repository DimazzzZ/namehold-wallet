import { useCurrentVersion } from "../queries/updates";
import { useAppUpdate, relaunchApp } from "../hooks/useAppUpdate";
import { Button } from "./ui/Button";

/**
 * The "Updates" card in Settings: shows the running version and drives the
 * check → available → install → relaunch flow. Shares its state with the
 * global update banner via `useAppUpdate`, so an update found by the silent
 * launch check is installable here too.
 */
export function UpdatesSettings() {
  const { data: version } = useCurrentVersion();
  const { phase, available, progress, error, check, install } = useAppUpdate();

  const pct = progress != null ? Math.round(progress * 100) : null;

  return (
    <div className="space-y-3" data-testid="updates-settings">
      <div className="flex items-center justify-between">
        <div className="text-sm text-gray-700">
          Current version{" "}
          <span className="font-mono" data-testid="current-version">
            v{version ?? "—"}
          </span>
        </div>
        <Button
          size="sm"
          onClick={() => check()}
          loading={phase === "checking"}
          disabled={phase === "checking" || phase === "installing"}
          data-testid="check-for-updates"
        >
          {phase === "checking" ? "Checking…" : "Check for updates"}
        </Button>
      </div>

      {phase === "upToDate" && (
        <div className="text-xs text-green-700" data-testid="update-uptodate">
          You are on the latest version.
        </div>
      )}

      {(phase === "available" || phase === "installing" || phase === "installed") &&
        available && (
          <div className="rounded border border-blue-200 bg-blue-50 p-3 space-y-2">
            <div className="text-sm font-medium text-blue-900">
              Version {available.version} is available
            </div>
            {available.notes && (
              <pre className="text-xs text-blue-800 whitespace-pre-wrap max-h-32 overflow-auto">
                {available.notes}
              </pre>
            )}

            {phase === "available" && (
              <Button
                size="sm"
                variant="primary"
                onClick={() => install()}
                data-testid="install-update"
              >
                Install now
              </Button>
            )}

            {phase === "installing" && (
              <div data-testid="update-progress">
                <div className="h-2 w-full rounded bg-blue-100 overflow-hidden">
                  <div
                    className="h-full bg-blue-600 transition-all"
                    style={{ width: pct != null ? `${pct}%` : "40%" }}
                  />
                </div>
                <div className="mt-1 text-xs text-blue-800">
                  {pct != null ? `Downloading… ${pct}%` : "Downloading…"}
                </div>
              </div>
            )}

            {phase === "installed" && (
              <div className="space-y-2" data-testid="update-installed">
                <div className="text-xs text-green-700">
                  Update installed. Restart to finish.
                </div>
                <Button size="sm" variant="primary" onClick={() => relaunchApp()}>
                  Restart now
                </Button>
              </div>
            )}
          </div>
        )}

      {phase === "error" && (
        <div className="space-y-1" data-testid="update-error">
          <div className="text-xs text-red-600">{error ?? "Update failed."}</div>
          <Button size="sm" onClick={() => check()}>
            Retry
          </Button>
        </div>
      )}
    </div>
  );
}
