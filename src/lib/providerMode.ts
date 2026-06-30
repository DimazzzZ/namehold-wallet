import type { HsdBalance, StatusTone, WalletTransactionRow } from "../types";

/**
 * Pure, framework-free normalization helpers shared by the read query layer.
 */

/**
 * Parse a settings value that stores a JSON string array. Returns a clean
 * array, tolerating malformed or empty values.
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
