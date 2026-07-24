import { useEffect } from "react";
import { useAppUpdate } from "../hooks/useAppUpdate";
import { isTauri } from "../lib/runtime";

/**
 * Slim top banner shown when a silent auto-check (~30s after launch) finds a
 * new release. Users can install right away or dismiss for this version — the
 * dismissal is remembered in localStorage so we don't nag on every launch.
 *
 * Only runs in Tauri: the updater plugin is desktop-only, and the browser QA
 * mock always reports "up to date" anyway.
 */
export function UpdateBanner() {
  const { phase, available, progress, dismissedVersion, check, install, dismiss } =
    useAppUpdate();

  useEffect(() => {
    if (!isTauri()) return;
    // Delay the check so it doesn't compete with startup work (autostart hsd,
    // deadline scan, first paint). A single fire-and-forget check is enough;
    // users can re-check manually from Settings.
    const timer = window.setTimeout(() => {
      void check({ silent: true });
    }, 30_000);
    return () => window.clearTimeout(timer);
    // Runs once per mount — the store handles idempotency itself.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Suppress the banner if the user already dismissed this exact version.
  const suppressed =
    !available || (dismissedVersion != null && dismissedVersion === available.version);

  const showing =
    !suppressed && (phase === "available" || phase === "installing" || phase === "installed");

  if (!showing) return null;

  const pct = progress != null ? Math.round(progress * 100) : null;

  return (
    <div
      className="px-6 py-1.5 text-xs text-blue-900 bg-blue-100 border-b border-blue-200 flex items-center gap-3"
      data-testid="update-banner"
    >
      <span>
        🎉 <strong>Namehold v{available!.version}</strong> is available.
      </span>

      {phase === "available" && (
        <>
          <button
            type="button"
            className="underline font-medium hover:no-underline"
            onClick={() => void install()}
            data-testid="update-banner-install"
          >
            Install now
          </button>
          <button
            type="button"
            className="text-blue-800/70 hover:text-blue-900"
            onClick={() => dismiss()}
            data-testid="update-banner-later"
          >
            Later
          </button>
        </>
      )}

      {phase === "installing" && (
        <span data-testid="update-banner-progress">
          Installing… {pct != null ? `${pct}%` : ""}
        </span>
      )}

      {phase === "installed" && (
        <span className="text-green-800">
          Update installed — restart from Settings.
        </span>
      )}
    </div>
  );
}
