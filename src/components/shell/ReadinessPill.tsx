import { ChevronDown, ChevronRight } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useSignerToggle } from "../../hooks/useSignerToggle";
import { mapError } from "../../lib/errors";
import {
  confirmResync,
  networkDisplayName,
  nodeActionLabel,
  OUTSIDE_NODE_NOT_STOPPED,
} from "../../lib/nodeNetwork";
import { cn, formatDate, latestTimestamp } from "../../lib/utils";
import { walletKindLabel } from "../../lib/walletKind";
import {
  type NodeStatus,
  useNodeStatus,
  useResyncHsd,
  useStartHsd,
  useStopHsd,
} from "../../queries/node";
import { useStartFullSync, useSyncStatus } from "../../queries/sync";
import { useActiveProfile, useWriteCapability } from "../../queries/wallet";
import { useSyncTriggerStore } from "../../stores/syncTrigger";
import { useUiStore } from "../../stores/ui";
import type { WalletProfileSummary, WriteCapability } from "../../types";
import { Button } from "../ui/Button";
import { Icon } from "../ui/Icon";
import { Popover } from "../ui/Popover";
import { Tooltip } from "../ui/Tooltip";

export type NodeView = "connected" | "outside" | "syncing" | "starting" | "offline";
export type ReadinessKind = "none" | "watch" | "ledger" | "hot";
export type ReadinessTone = "ok" | "warn" | "info" | "off";

export interface Readiness {
  kind: ReadinessKind;
  nodeView: NodeView;
  syncPct: number | null;
  tone: ReadinessTone;
  label: string;
  detail: string | null;
  title: string;
  summary: string | null;
}

/**
 * The one answer to "which wallet is this and can it send?". The first
 * matching row wins (shell-v2 §5e); numbers come from the backend only.
 */
export function readiness(input: {
  profile: Pick<WalletProfileSummary, "watchOnly" | "kind" | "network"> | null | undefined;
  writeCap: WriteCapability | null | undefined;
  node: Partial<NodeStatus> | null | undefined;
}): Readiness {
  const { profile, writeCap, node } = input;
  const nodeView: NodeView = node?.connected
    ? node.synced
      ? node.ownership === "outside"
        ? "outside"
        : "connected"
      : "syncing"
    : (node?.starting ?? node?.process_alive)
      ? "starting"
      : "offline";
  const syncPct =
    node?.verification_progress != null ? Math.floor(node.verification_progress * 100) : null;
  // A Ledger signs on the device: no signer session, nothing to unlock.
  const kind: ReadinessKind = !profile
    ? "none"
    : profile.watchOnly
      ? "watch"
      : profile.kind === "ledger_hardware"
        ? "ledger"
        : "hot";
  const base = { kind, nodeView, syncPct };
  const r = (
    tone: ReadinessTone,
    label: string,
    detail: string | null,
    title: string,
    summary: string | null,
  ): Readiness => ({ ...base, tone, label, detail, title, summary });

  if (kind === "none") {
    return r(
      "off",
      "No wallet",
      "create or import one",
      "No wallet yet",
      "Create or import a wallet to start. Nothing can be sent until then.",
    );
  }
  if (kind === "watch") {
    return r(
      "info",
      "Watch-only",
      "can't send",
      "Watch-only wallet",
      "This wallet tracks an account xpub. It can read balances and names but can't sign or send.",
    );
  }
  if (!writeCap) return r("off", "Checking…", null, "Checking…", null);
  if (kind === "ledger" && writeCap.canWrite) {
    return r(
      "ok",
      "Ready to send",
      "confirm on your Ledger",
      "Ready to send",
      "Every spend is confirmed on your Ledger. Nothing to unlock here.",
    );
  }
  if (writeCap.canWrite) {
    return r(
      "ok",
      "Ready to send",
      node?.height != null ? `block ${node.height.toLocaleString("en-US")}` : null,
      "Ready to send",
      "You can sign and broadcast from this wallet.",
    );
  }
  if (!writeCap.broadcasterAvailable) {
    if (nodeView === "offline") {
      return r(
        "warn",
        "Can't send",
        "node offline",
        "Can't send",
        "Your node is offline. Start it to sync your coins and send.",
      );
    }
    if (nodeView === "starting") {
      const node = profile?.network ? `${networkDisplayName(profile.network)} node` : "node";
      return r(
        "warn",
        "Can't send",
        "node starting",
        "Can't send yet",
        `The ${node} is starting. Sending becomes available once it has synced.`,
      );
    }
    if (nodeView === "syncing") {
      return r(
        "warn",
        "Can't send",
        syncPct != null ? `node syncing ${syncPct}%` : "node syncing",
        "Can't send yet",
        "Your node is still syncing. Spendable balance and sending become available once it finishes.",
      );
    }
    return r(
      "warn",
      "Can't send",
      "can't broadcast",
      "Can't send",
      writeCap.reason ?? "Namehold has no node to send through.",
    );
  }
  if (kind === "hot" && !writeCap.signerUnlocked) {
    return r(
      "warn",
      "Locked",
      "unlock to send",
      "Locked",
      "You can browse; unlock to sign and send.",
    );
  }
  if (kind === "ledger") {
    return r(
      "warn",
      "Can't send",
      "see why",
      "Can't send",
      writeCap.reason ?? "Namehold can't send from this Ledger wallet right now.",
    );
  }
  return r("warn", "Can't send", null, "Can't send", writeCap.reason);
}

const TONE_PILL: Record<ReadinessTone, string> = {
  ok: "border-green-200 bg-green-50 text-green-800",
  warn: "border-amber-200 bg-amber-50 text-amber-800",
  info: "border-blue-200 bg-blue-50 text-blue-800",
  off: "border-gray-200 bg-white text-gray-700",
};
const TONE_DOT: Record<ReadinessTone, string> = {
  ok: "bg-green-500",
  warn: "bg-amber-500",
  info: "bg-blue-500",
  off: "bg-gray-400",
};

function Row({
  dot,
  children,
  action,
}: {
  dot: string | null;
  children: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-2 px-3.5 py-1.5">
      {dot ? (
        <span className={cn("size-2 rounded-full shrink-0", dot)} aria-hidden />
      ) : (
        <span className="w-2 shrink-0" aria-hidden />
      )}
      <div className="flex-1 min-w-0">{children}</div>
      {action && <div className="flex items-center gap-1 shrink-0">{action}</div>}
    </div>
  );
}

/** The top bar's readiness pill and its popover (replaces the v1 StatusStrip). */
export function ReadinessPill() {
  const navigate = useNavigate();
  const { data: profile } = useActiveProfile();
  const { data: writeCap } = useWriteCapability();
  const { data: node } = useNodeStatus();
  const signer = useSignerToggle();
  const showToast = useUiStore((s) => s.showToast);
  const open = useUiStore((s) => s.readinessOpen);
  const setOpen = useUiStore((s) => s.setReadinessOpen);
  const openWallets = useUiStore((s) => s.openWallets);
  const startHsd = useStartHsd();
  const stopHsd = useStopHsd();
  const resyncHsd = useResyncHsd();
  const startSync = useStartFullSync();
  const syncRunning = useSyncStatus().data?.running ?? false;
  const setManualSync = useSyncTriggerStore((s) => s.setManualSync);

  const r = readiness({ profile, writeCap, node });
  // Spec 2026-10-24: the controls act on the active wallet's network's node
  // and name it (R8); a node Namehold did not start is never stopped (R7) —
  // whatever it is doing, even still syncing.
  const nodeNetwork = node?.network ?? null;
  const nodeOutside = node?.ownership === "outside";
  const nodeBusy = startHsd.isPending || stopHsd.isPending || resyncHsd.isPending;

  const runNodeAction = async (fn: () => Promise<unknown>, successMsg: string) => {
    try {
      await fn();
      showToast(successMsg, "info");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleSync = async () => {
    try {
      setManualSync(true);
      await startSync.mutateAsync();
      showToast("Sync started in background", "info");
    } catch (e) {
      setManualSync(false);
      showToast(mapError(e), "error");
    }
  };

  const height = node?.height != null ? node.height.toLocaleString("en-US") : null;
  const nodeText = (() => {
    switch (r.nodeView) {
      case "connected":
        return ["Node connected · synced", height && `block ${height}`].filter(Boolean).join(" · ");
      case "outside":
        return ["Node connected (external node)", height && `block ${height}`]
          .filter(Boolean)
          .join(" · ");
      case "syncing": {
        const blocks =
          height && node?.headers != null
            ? `block ${height} / ${node.headers.toLocaleString("en-US")}`
            : height && `block ${height}`;
        return ["Node syncing", r.syncPct != null && `${r.syncPct}%`, blocks]
          .filter(Boolean)
          .join(" · ");
      }
      case "starting":
        return nodeActionLabel("Starting", nodeNetwork);
      default:
        return "Node offline";
    }
  })();
  const nodeDot =
    r.nodeView === "connected" || r.nodeView === "outside"
      ? "bg-green-500"
      : r.nodeView === "offline"
        ? "bg-gray-400"
        : "bg-amber-500";

  const isSpv = node?.node_mode === "spv";
  const readLocal = node?.read_source === "local";
  const source = isSpv
    ? {
        text: "Reads from the explorer (SPV)",
        tip: "SPV mode: data comes from explorer (no local UTXO cache)",
      }
    : readLocal
      ? { text: "Reads from your local node", tip: "Reading from local node cache (node synced)" }
      : {
          text: "Reads from the explorer",
          tip: "Reading from external explorer (node not synced)",
        };

  const lastSync = profile
    ? latestTimestamp(profile.lastSyncedAt, profile.lastExplorerSyncAt)
    : null;

  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      align="right"
      role="dialog"
      aria-label="Wallet readiness"
      data-testid="readiness-popover"
      panelClassName="w-90 py-0 rounded-lg shadow-lg text-[13px]"
      trigger={({ open: isOpen, toggle }) => (
        <Tooltip content={r.summary}>
          <button
            type="button"
            onClick={toggle}
            aria-haspopup="dialog"
            aria-expanded={isOpen}
            data-testid="readiness-pill"
            className={cn(
              "rounded-full border px-2.5 py-1 text-xs font-medium inline-flex items-center gap-1.5",
              TONE_PILL[r.tone],
            )}
          >
            <span className={cn("size-2 rounded-full", TONE_DOT[r.tone])} aria-hidden />
            {r.label}
            {r.detail && <span className="font-normal opacity-85">· {r.detail}</span>}
            <Icon icon={ChevronDown} size={13} />
          </button>
        </Tooltip>
      )}
    >
      {({ close }) => (
        <>
          <div className="px-3.5 py-3 border-b border-gray-100">
            <div className="font-semibold text-gray-900">{r.title}</div>
            {r.summary && <div className="text-xs text-gray-500 mt-0.5">{r.summary}</div>}
          </div>
          <div className="py-1.5">
            <Row
              dot={profile ? "bg-blue-500" : "bg-gray-400"}
              action={
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    close();
                    openWallets(profile ? "list" : "add");
                  }}
                >
                  Wallets…
                </Button>
              }
            >
              {profile
                ? `${profile.label} · ${walletKindLabel(profile)} · ${networkDisplayName(profile.network)}`
                : "No wallet"}
            </Row>

            {r.kind === "hot" && (
              <Row
                dot={signer.unlocked ? "bg-green-500" : "bg-gray-400"}
                action={
                  <Button
                    size="sm"
                    variant={signer.unlocked ? "secondary" : "primary"}
                    disabled={signer.busy !== null}
                    onClick={() => void signer.toggle()}
                    data-testid="readiness-signer-toggle"
                  >
                    {signer.busy === "locking"
                      ? "Locking…"
                      : signer.busy === "unlocking"
                        ? "Unlocking…"
                        : signer.unlocked
                          ? "Lock"
                          : "Unlock"}
                  </Button>
                }
              >
                <div>{signer.unlocked ? "Signer unlocked" : "Signer locked"}</div>
                <div className="text-[11px] text-gray-500">
                  {signer.unlocked
                    ? "Locks automatically after the session timeout"
                    : signer.hasPassphrase
                      ? "Unlock with your passphrase, in a secure window"
                      : "No passphrase set — just click Unlock"}
                </div>
              </Row>
            )}

            {r.kind === "ledger" && (
              <Row dot="bg-green-500">
                <div>Signs on your Ledger</div>
                <div className="text-[11px] text-gray-500">
                  Keep it connected and the Handshake app open when you send
                </div>
              </Row>
            )}

            <Row
              dot={nodeDot}
              action={
                r.nodeView === "offline" ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={nodeBusy}
                    data-testid="readiness-node-start"
                    onClick={() =>
                      void runNodeAction(() => startHsd.mutateAsync(), "Starting node…")
                    }
                  >
                    {nodeActionLabel("Start", nodeNetwork)}
                  </Button>
                ) : r.nodeView === "starting" ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={nodeBusy}
                    data-testid="readiness-node-cancel"
                    onClick={() =>
                      void runNodeAction(() => stopHsd.mutateAsync(), "Stopping node…")
                    }
                  >
                    Cancel start
                  </Button>
                ) : (
                  <>
                    <Tooltip
                      content={
                        nodeOutside
                          ? OUTSIDE_NODE_NOT_STOPPED
                          : nodeActionLabel("Stop", nodeNetwork).replace(/^Stop/, "Stop the")
                      }
                    >
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={nodeBusy || nodeOutside}
                        data-testid="readiness-node-stop"
                        onClick={() =>
                          void runNodeAction(() => stopHsd.mutateAsync(), "Stopping node…")
                        }
                      >
                        Stop
                      </Button>
                    </Tooltip>
                    <Tooltip content={nodeOutside ? OUTSIDE_NODE_NOT_STOPPED : "Re-sync chain"}>
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={nodeBusy || nodeOutside}
                        data-testid="readiness-node-resync"
                        onClick={() => {
                          if (!confirmResync()) return;
                          void runNodeAction(
                            () => resyncHsd.mutateAsync(),
                            "Re-syncing chain — this may take a while",
                          );
                        }}
                      >
                        Re-sync
                      </Button>
                    </Tooltip>
                  </>
                )
              }
            >
              {nodeText}
            </Row>
            {r.nodeView === "syncing" && r.syncPct != null && (
              <div className="pl-[30px] pr-3.5 pb-1.5">
                <div className="h-1 rounded bg-gray-200">
                  <div
                    className="h-1 rounded bg-blue-500"
                    style={{ width: `${r.syncPct}%` }}
                    data-testid="readiness-sync-progress"
                  />
                </div>
              </div>
            )}

            <Row dot={readLocal && !isSpv ? "bg-green-500" : "bg-blue-500"}>
              <Tooltip content={source.tip}>
                <span>{source.text}</span>
              </Tooltip>
            </Row>

            {r.kind === "hot" && writeCap && !writeCap.canWrite && writeCap.reason && (
              <div
                className="mx-3.5 my-1 rounded-md border border-amber-200 bg-amber-50 px-2.5 py-1.5 text-xs text-amber-800"
                data-testid="readiness-reason"
              >
                {writeCap.reason}
              </div>
            )}

            {profile && (
              <Row
                dot={null}
                action={
                  <Button
                    size="sm"
                    variant="ghost"
                    className="text-blue-600"
                    disabled={syncRunning}
                    onClick={() => void handleSync()}
                    data-testid="readiness-sync-now"
                  >
                    {syncRunning ? "Syncing…" : "Sync now"}
                  </Button>
                }
              >
                <span className="text-xs text-gray-500">
                  {lastSync
                    ? [
                        `Last synced ${formatDate(lastSync)}`,
                        profile.lastSyncedHeight != null &&
                          `height ${profile.lastSyncedHeight.toLocaleString("en-US")}`,
                      ]
                        .filter(Boolean)
                        .join(" · ")
                    : "Not synced yet"}
                </span>
              </Row>
            )}
          </div>
          <div className="px-3.5 py-2 border-t border-gray-100 flex justify-end text-xs">
            <button
              type="button"
              className="inline-flex items-center gap-0.5 text-blue-600 hover:underline"
              onClick={() => {
                close();
                navigate("/settings?section=connections");
              }}
            >
              Connections in Settings <Icon icon={ChevronRight} size={13} />
            </button>
          </div>
        </>
      )}
    </Popover>
  );
}
