import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "../lib/invoke";
import { Alert } from "./ui/Alert";
import { Button } from "./ui/Button";
import { Spinner } from "./ui/Spinner";

/** `startup_status` (src-tauri/src/commands/startup.rs). */
export type StartupStatus =
  | { state: "starting" }
  | {
      state: "migrating";
      step: number;
      total: number;
      version: string;
      elapsed_secs: number;
      percent: number | null;
    }
  | { state: "failed"; error: string }
  | { state: "ready" };

function formatElapsed(secs: number): string {
  if (secs < 60) return `${secs} s`;
  return `${Math.floor(secs / 60)} min ${secs % 60} s`;
}

/**
 * Renders the app once the backend is set up. After an update whose database
 * migration takes long, the window shows the migration's progress instead of
 * staying blank (the app used to be unresponsive until it finished).
 */
export function StartupGate({ children, pollMs = 500 }: { children: ReactNode; pollMs?: number }) {
  const [status, setStatus] = useState<StartupStatus | null>(null);

  useEffect(() => {
    if (status?.state === "ready") return;
    let cancelled = false;
    const check = async () => {
      try {
        const next = await invoke<StartupStatus>("startup_status");
        if (!cancelled) setStatus(next);
      } catch {
        // A transient IPC error: the next poll asks again.
      }
    };
    void check();
    const timer = setInterval(() => void check(), pollMs);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [status?.state, pollMs]);

  if (status?.state === "ready") return <>{children}</>;
  if (status === null || status.state === "starting") return null;

  return (
    <div className="flex min-h-screen items-center justify-center bg-gray-50 px-6">
      <div className="w-full max-w-md rounded-lg border border-gray-200 bg-white p-6">
        {status.state === "migrating" ? (
          <Migrating status={status} />
        ) : (
          <Failed error={status.error} />
        )}
      </div>
    </div>
  );
}

function Migrating({ status }: { status: Extract<StartupStatus, { state: "migrating" }> }) {
  const percent = status.percent ?? 0;
  return (
    <div role="status">
      <div className="flex items-center gap-2">
        <Spinner />
        <h1 className="text-sm font-semibold text-gray-900">Updating Namehold's database</h1>
      </div>
      <div
        className="mt-4 h-1.5 w-full overflow-hidden rounded-full bg-gray-100"
        role="progressbar"
        aria-label="Database update"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
      >
        <div
          className="h-full rounded-full bg-blue-600 transition-all"
          style={{ width: `${percent}%` }}
        />
      </div>
      <p className="mt-2 text-xs tabular-nums text-gray-600">
        {status.total > 0 ? `Step ${status.step} of ${status.total}` : "Starting"}
        {status.percent !== null ? ` · ${status.percent}%` : ""} ·{" "}
        {formatElapsed(status.elapsed_secs)}
      </p>
      <p className="mt-4 text-sm text-gray-600">
        This runs once after an update and can take a few minutes on a large database. Keep Namehold
        open: if it is closed now, the update starts over next time.
      </p>
    </div>
  );
}

function Failed({ error }: { error: string }) {
  const [retrying, setRetrying] = useState(false);
  return (
    <div>
      <Alert tone="error" title="The database update did not finish">
        <p className="break-words">{error}</p>
        <p className="mt-2">
          Nothing is lost: a step that does not finish is rolled back, and Try again starts it over.
        </p>
      </Alert>
      <div className="mt-4 flex justify-end">
        <Button
          variant="primary"
          disabled={retrying}
          onClick={() => {
            setRetrying(true);
            void invoke("retry_startup").finally(() => setRetrying(false));
          }}
        >
          Try again
        </Button>
      </div>
    </div>
  );
}
