import type {
  ConnectionMode,
  HsdBalance,
  ReadContext,
  ReadProviderKind,
  StatusTone,
  WalletTransactionRow,
} from "../types";

/**
 * Pure, framework-free helpers describing what the active connection mode /
 * read provider is allowed to do, plus normalization helpers shared between the
 * provider-aware query layer and components.
 *
 * Keeping this logic isolated (no React, no Tauri) makes it directly unit
 * testable and reusable across WalletView, Overview, NodeControl, etc.
 */

export const CONNECTION_MODE_LABELS: Record<ConnectionMode, string> = {
  local_managed_hsd: "Local managed hsd",
  remote_hsd: "Remote hsd",
  auto_fallback: "Auto (local, fallback read-only)",
  external_read_only: "External read-only",
};

export const READ_PROVIDER_LABELS: Record<ReadProviderKind, string> = {
  local_hsd: "Local hsd",
  remote_hsd: "Remote hsd",
  external_hnsfans: "HNSFans",
};

/**
 * Consistent source name for a read provider. For `remote_hsd`, an optional
 * user-supplied label (e.g. "Home server") is preferred when present.
 */
export function providerLabel(
  kind: ReadProviderKind,
  remoteLabel?: string | null,
): string {
  if (kind === "remote_hsd" && remoteLabel && remoteLabel.trim().length > 0) {
    return remoteLabel.trim();
  }
  return READ_PROVIDER_LABELS[kind];
}

/** Whether the active context permits write operations (send, transfer, etc.). */
export function canWrite(context: ReadContext | null | undefined): boolean {
  return Boolean(context?.writeAllowed);
}

/** Human-readable reason writes are blocked, if any. */
export function writeBlockedReason(
  context: ReadContext | null | undefined,
): string | null {
  if (!context) return "Provider status is unknown.";
  if (context.writeAllowed) return null;
  return (
    context.writeReason ??
    context.activeReadProvider?.reason ??
    "Writes are not available in the current mode."
  );
}

/** Whether NodeControl may start/stop the active backend. */
export function canManageNode(context: ReadContext | null | undefined): boolean {
  return Boolean(context?.activeReadProvider?.manageable);
}

/**
 * Centralized write-gating explanation. Considers both the resolved provider
 * context and the user's `write_mode` preference; returns a human-readable
 * reason writes are unavailable, or `null` when writes are permitted.
 */
export function resolveWriteReason(
  context: ReadContext | null | undefined,
  writeMode: boolean,
): string | null {
  if (!writeMode) {
    return "Write mode is disabled. Enable it in Settings to send or transfer.";
  }
  return writeBlockedReason(context);
}

/** Whether the active read provider is an external read-only explorer. */
export function isExternalProvider(
  context: ReadContext | null | undefined,
): boolean {
  return context?.activeReadProvider?.kind === "external_hnsfans";
}

/** Whether the active read provider exposes a full local wallet. */
export function hasWallet(context: ReadContext | null | undefined): boolean {
  return Boolean(context?.walletAvailable);
}

/** Whether the app is currently running on a degraded fallback path. */
export function isFallbackActive(
  context: ReadContext | null | undefined,
): boolean {
  return Boolean(context?.fallbackActive);
}

/** Tone for surfacing the active provider's health in status strips. */
export function providerTone(context: ReadContext | null | undefined): StatusTone {
  if (!context) return "default";
  const provider = context.activeReadProvider;
  if (!provider?.healthy) return "error";
  if (context.fallbackActive || isExternalProvider(context)) return "warning";
  return "success";
}

/** Short status label for the active provider (e.g. shell status strip). */
export function providerStatusValue(
  context: ReadContext | null | undefined,
): string {
  if (!context) return "Unknown";
  const provider = context.activeReadProvider;
  if (!provider) return "Unknown";
  if (!provider.healthy) return "Unavailable";
  if (context.fallbackActive) return "Read-only (fallback)";
  if (isExternalProvider(context)) return "Read-only";
  return "Connected";
}

/**
 * Parse a settings value that stores a JSON string array (watch addresses /
 * names). Returns a clean array, tolerating malformed or empty values.
 */
export function parseStringArraySetting(raw: string | null | undefined): string[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map((v) => (typeof v === "string" ? v.trim() : ""))
      .filter((v) => v.length > 0);
  } catch {
    return [];
  }
}

/** Serialize a string array back into the settings storage format. */
export function serializeStringArraySetting(values: string[]): string {
  return JSON.stringify(
    values.map((v) => v.trim()).filter((v) => v.length > 0),
  );
}

/**
 * Normalize a raw transaction object (from hsd or an external provider) into the
 * UI-facing `WalletTransactionRow`. Handles both the hsd shape (outputs/fee) and
 * the flatter external shape (value/amount/address fields).
 */
export function normalizeTransaction(
  tx: Record<string, unknown>,
  index: number,
): WalletTransactionRow {
  const hash =
    (typeof tx.hash === "string" && tx.hash) ||
    (typeof tx.txid === "string" && tx.txid) ||
    `tx-${index}`;

  const confirmations =
    typeof tx.confirmations === "number" ? tx.confirmations : null;
  const confirmed =
    confirmations !== null ? confirmations > 0 : Boolean(tx.confirmed);
  const height = typeof tx.height === "number" ? tx.height : null;

  const matchedAddress =
    typeof tx.matchedAddress === "string" ? tx.matchedAddress : null;

  let amountDoos = 0;
  let address = matchedAddress ?? "";
  let direction: WalletTransactionRow["direction"] = "other";

  const outputs = Array.isArray(tx.outputs) ? (tx.outputs as unknown[]) : null;
  if (outputs && outputs.length > 0) {
    for (const out of outputs) {
      const o = out as Record<string, unknown>;
      const value = typeof o.value === "number" ? o.value : 0;
      amountDoos += value;
      const addr = typeof o.address === "string" ? o.address : "";
      if (addr && !address) address = addr;
    }
    direction =
      typeof tx.fee === "number" && tx.fee > 0 ? "send" : "receive";
  } else {
    // Flat external shape: a single value/amount and direction hint.
    const value =
      (typeof tx.value === "number" && tx.value) ||
      (typeof tx.amount === "number" && tx.amount) ||
      0;
    amountDoos = value;
    const dir =
      typeof tx.direction === "string" ? tx.direction.toLowerCase() : "";
    if (dir === "send" || dir === "out" || dir === "sent") {
      direction = "send";
    } else if (dir === "receive" || dir === "in" || dir === "received") {
      direction = "receive";
    } else {
      direction = value < 0 ? "send" : "receive";
    }
    if (!address && typeof tx.address === "string") {
      address = tx.address;
    }
    amountDoos = Math.abs(amountDoos);
  }

  let timestamp: string | null = null;
  if (typeof tx.mtime === "number") {
    timestamp = new Date(tx.mtime * 1000).toISOString();
  } else if (typeof tx.time === "number") {
    timestamp = new Date(tx.time * 1000).toISOString();
  } else if (typeof tx.timestamp === "string") {
    timestamp = tx.timestamp;
  }

  const tone: StatusTone = confirmed ? "success" : "warning";

  return {
    hash,
    direction,
    amountDoos,
    amountHns: amountDoos / 1e6,
    address,
    confirmed,
    confirmations,
    height,
    timestamp,
    tone,
  };
}

/** Total spendable + pending balance in doos. */
export function totalBalanceDoos(balance: HsdBalance | null | undefined): number {
  if (!balance) return 0;
  return balance.confirmed + balance.unconfirmed;
}
