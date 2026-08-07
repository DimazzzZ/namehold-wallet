import { useEffect, useState } from "react";
import { useAppUpdate, relaunchApp } from "../hooks/useAppUpdate";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";
import { isTauri } from "../lib/runtime";
import { WhatsNewModal } from "./WhatsNewModal";

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
  const [showNotes, setShowNotes] = useState(false);
  const [relaunching, setRelaunching] = useState(false);
  const showToast = useUiStore((s) => s.showToast);

  const handleRelaunch = async () => {
    if (relaunching) return;
    setRelaunching(true);
    try {
      await relaunchApp();
      // Success = the process is being replaced; we won't get here in
      // practice, but keep the flag set so the button stays disabled.
    } catch (e) {
      // relaunchApp only fails when the tauri IPC itself errors — surface
      // the reason so the user isn't stuck staring at a dead button.
      setRelaunching(false);
      showToast(mapError(e), "error");
    }
  };

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
    <>
      <div
        className="px-6 py-1.5 text-xs text-blue-900 bg-blue-100 border-b border-blue-200 flex items-center gap-3"
        data-testid="update-banner"
      >
      <span>
        🎉 <strong>Namehold v{available!.version}</strong> is available.
      </span>

      {phase === "available" && (
        <>
          {available.notes && (
            <button
              type="button"
              className="text-blue-800/70 hover:text-blue-900"
              onClick={() => setShowNotes(true)}
              data-testid="update-banner-whats-new"
            >
              What's new?
            </button>
          )}
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
        <>
          <span className="text-green-800">Update installed.</span>
          <button
            type="button"
            className="underline font-medium hover:no-underline"
            onClick={() => void handleRelaunch()}
            disabled={relaunching}
            data-testid="update-banner-relaunch"
          >
            {relaunching ? "Relaunching…" : "Relaunch now"}
          </button>
          <button
            type="button"
            className="text-blue-800/70 hover:text-blue-900"
            onClick={() => dismiss()}
            data-testid="update-banner-installed-later"
          >
            Later
          </button>
        </>
      )}
      </div>
      <WhatsNewModal
        open={showNotes}
        onClose={() => setShowNotes(false)}
        version={available.version}
        notes={available.notes ?? ""}
      />
    </>
  );
}
