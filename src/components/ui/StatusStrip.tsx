import { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { cn } from "../../lib/utils";
import { useNodeStatus } from "../../queries/node";
import { useNamebaseStatus } from "../../queries/namebase";
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
 * Compact, always-visible operational status indicators rendered in the shell.
 * Each item is clickable and routes to the relevant workspace.
 */
export function StatusStrip({ className }: { className?: string }) {
  const navigate = useNavigate();
  const { data: node } = useNodeStatus();
  const { data: namebase } = useNamebaseStatus();

  const items = useMemo<ShellStatusItem[]>(() => {
    const result: ShellStatusItem[] = [];

    const nodeRunning = Boolean(node?.running);
    result.push({
      key: "node",
      label: "Node",
      value: nodeRunning ? "Running" : "Stopped",
      tone: nodeRunning ? "success" : "default",
      detail: node?.error ?? node?.hsd_version,
      route: "/node",
    });

    const walletConnected = Boolean(node?.wallet_connected);
    result.push({
      key: "wallet",
      label: "Wallet",
      value: walletConnected ? "Connected" : "Offline",
      tone: walletConnected ? "success" : "warning",
      route: "/wallet",
    });

    const nbConnected = Boolean(namebase?.connected);
    result.push({
      key: "namebase",
      label: "Namebase",
      value: nbConnected ? "Connected" : "Not connected",
      tone: nbConnected ? "success" : "default",
      detail: namebase?.error,
      route: "/migration",
    });

    return result;
  }, [node, namebase]);

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
