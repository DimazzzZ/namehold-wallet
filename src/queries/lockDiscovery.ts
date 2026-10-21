import { useEffect, useRef } from "react";
import { foundLocksText } from "../components/market/marketText";
import { isSignerUnlockedFor } from "../lib/signer";
import { useUiStore } from "../stores/ui";
import { useNodeLive } from "./node";
import { useFindLocks } from "./shakedex";
import { useSyncStatus } from "./sync";
import { useActiveProfile, useSignerSession } from "./wallet";

/**
 * R34: look for the active profile's own Shakedex locks after a restore.
 * Mount ONCE at the app root. On each end of a sync (`running` true to
 * false) with the node live, a recovery-phrase profile and the signer
 * unlocked for it, ask the backend to search (`force: false`: it runs only
 * until one search has read every address and judged every candidate, and
 * answers `ran: false` after that). Names it adopted are announced in one
 * toast, and My listings is refreshed by the command's own invalidation.
 * Errors are silent: the next sync tries again.
 */
export function useLockDiscovery(): void {
  const running = useSyncStatus().data?.running ?? false;
  const nodeLive = useNodeLive();
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const showToast = useUiStore((s) => s.showToast);
  const find = useFindLocks();

  // What the effect reads at the edge, without re-arming it on every change.
  const ready =
    nodeLive && profile?.kind === "mnemonic_hot" && isSignerUnlockedFor(signer, profile.id);
  const readyRef = useRef(ready);
  readyRef.current = ready;
  const searching = useRef(false);

  const wasRunning = useRef(false);
  useEffect(() => {
    const ended = wasRunning.current && !running;
    wasRunning.current = running;
    if (!ended || !readyRef.current || searching.current) return;
    searching.current = true;
    find
      .mutateAsync(false)
      .then((report) => {
        if (report.adopted.length > 0) showToast(foundLocksText(report.adopted.length));
      })
      .catch(() => {})
      .finally(() => {
        searching.current = false;
      });
    // `find` and `showToast` are stable for the edge's purpose.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [running]);
}
