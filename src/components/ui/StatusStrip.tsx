import { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { cn } from "../../lib/utils";
import { useActiveProfile, useSignerSession, useWriteCapability } from "../../queries/wallet";
import { useNodeStatus } from "../../queries/node";
import type { ShellStatusItem, StatusTone } from "../../types";

const TONE_DOT: Record<StatusTone, string> = {
  default: "bg-gray-400",
  info: "bg-blue-500",
  success: "bg-emerald-500",
  warning: "bg-amber-500",
  error: "bg-red-500",
};

const TONE_TEXT: Record<StatusTone, string> = {
  default: "text-gray-600",
  info: "text-blue-700",
  success: "text-emerald-700",
  warning: "text-amber-700",
  error: "text-red-700",
};

/**
 * Compact, always-visible status for the non-custodial model: which wallet is
 * active, whether the signer is unlocked, whether sending is possible (node
 * reachable + unlocked), and which data source is active for reads (local node
 * cache vs. explorer). Namebase status is NOT shown here — it lives only in
 * the Namebase section (the Migration workspace).
 */
export function StatusStrip({ className }: { className?: string }) {
  const navigate = useNavigate();
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const { data: writeCap } = useWriteCapability();
  const { data: node } = useNodeStatus();

  const items = useMemo<ShellStatusItem[]>(() => {
    const result: ShellStatusItem[] = [];

    result.push({
      key: "wallet",
      label: "Wallet",
      value: profile ? `${profile.label} · ${profile.network}` : "None",
      tone: profile ? "info" : "default",
      detail: profile ? undefined : "Create or import a wallet",
      route: "/wallet",
    });

    if (profile && !profile.watchOnly) {
      const unlocked = signer?.unlocked ?? false;
      result.push({
        key: "signer",
        label: "Signer",
        value: unlocked ? "Unlocked" : "Locked",
        tone: unlocked ? "success" : "default",
        route: "/wallet",
      });

      // Node connectivity — the authoritative RPC-answers signal, so the app
      // explicitly says whether a node is connected (not just "can send").
      const nodeConnected = node?.connected ?? false;
      const nodeStarting = node?.process_alive ?? false;
      result.push({
        key: "node",
        label: "Node",
        value: nodeConnected ? "Connected" : nodeStarting ? "Starting…" : "Offline",
        tone: nodeConnected ? "success" : nodeStarting ? "warning" : "default",
        detail: nodeConnected
          ? `block ${node?.height ?? "?"}`
          : "Start a node in Settings",
        route: "/settings",
      });

      const canWrite = writeCap?.canWrite ?? false;
      result.push({
        key: "sending",
        label: "Sending",
        value: canWrite ? "Ready" : "Unavailable",
        tone: canWrite ? "success" : "warning",
        detail: writeCap?.reason ?? undefined,
        route: "/settings",
      });

      // Read source: "local" when the node is connected and synced, "explorer" otherwise.
      // In SPV mode, always shows "Explorer (SPV)" to explain why reads come from explorer.
      const readSource = node?.read_source ?? "explorer";
      const nodeMode = node?.node_mode ?? "full";
      const isSpv = nodeMode === "spv";
      result.push({
        key: "source",
        label: "Source",
        value: isSpv ? "Explorer (SPV)" : readSource === "local" ? "Local" : "Explorer",
        tone: isSpv ? "info" : readSource === "local" ? "success" : "info",
        detail: isSpv
          ? "SPV mode: data comes from explorer (no local UTXO cache)"
          : readSource === "local"
            ? "Reading from local node cache (node synced)"
            : "Reading from external explorer (node not synced)",
        route: "/settings",
      });
    }

    return result;
  }, [profile, signer, writeCap, node]);

  return (
    <div className={cn("flex items-center gap-4", className)}>
      {items.map((item) => (
        <button
          key={item.key}
          type="button"
          title={item.detail}
          onClick={() => item.route && navigate(item.route)}
          className="flex items-center gap-1.5 text-xs hover:opacity-80 transition-opacity"
        >
          <span
            className={cn("inline-block h-2 w-2 rounded-full", TONE_DOT[item.tone])}
            aria-hidden
          />
          <span className="text-gray-500">{item.label}:</span>
          <span className={cn("font-medium", TONE_TEXT[item.tone])}>
            {item.value}
          </span>
        </button>
      ))}
    </div>
  );
}
