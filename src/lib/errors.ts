// Friendly messages for the NON-CUSTODIAL model. Matching is a case-insensitive
// substring scan over the raw error text (first match wins), so the more
// specific patterns are listed first. No legacy hsd-wallet / API-key / "wallet
// ID" copy — this app holds its own keys and reads via an explorer.
const ERROR_MAP: Record<string, string> = {
  // Explorer rate-limited (HNSFans answers rapid bursts with HTTP 403).
  "status 403": "The explorer is busy (rate-limited). Wait a moment and Refresh again.",
  forbidden: "The explorer is busy (rate-limited). Wait a moment and Refresh again.",
  // Explorer / network unreachable.
  hnsfans: "Couldn't reach the explorer. Check the Explorer URL in Settings and your connection.",
  "connection refused": "Couldn't reach the configured endpoint. Check your connection and Settings.",
  econnrefused: "Couldn't reach the configured endpoint. Check your connection and Settings.",
  "connection reset": "Connection lost. Please try again.",
  // Ledger-specific timeout — must precede the generic timeout entries so its
  // actionable "confirm on the device" guidance isn't clobbered.
  "timed out waiting for the ledger":
    "Timed out waiting for your Ledger. Approve or reject the prompt on the device and try again.",
  "timed out": "The request timed out. Please try again.",
  timeout: "The request timed out. Please try again.",
  // Signer state (still valid in the non-custodial model).
  "wallet locked": "Your signer is locked — click Unlock first.",
  "wallet is locked": "Your signer is locked — click Unlock first.",
  // Ledger device state — the status-word hint from hid_transport.rs.
  "device locked — unlock it": "Your Ledger is locked — unlock the device and try again.",
  "handshake app": "Open the Handshake app on your Ledger and try again.",
  "no ledger device found": "No Ledger detected. Plug it in, unlock it, and open the Handshake app.",
  // Node not address-indexed (getcoinsbyaddress unavailable) — blocks all spends.
  "getcoinsbyaddress":
    "Your node isn't address-indexed. Restart hsd with address indexing (Settings → Start hsd) and let it finish syncing.",
  "index-address":
    "Your node isn't address-indexed. Restart hsd with address indexing (Settings → Start hsd) and let it finish syncing.",
  // The name's coin isn't in the wallet's synced set yet.
  "does not hold":
    "This wallet hasn't synced this name's coin yet — make sure your node is fully synced and address-indexed (Settings), Refresh, then try again.",
  // Sending.
  "insufficient funds": "Insufficient HNS balance for this transaction.",
};

/**
 * Which leg of the build → unlock/sign → broadcast pipeline threw. Used to
 * prefix the mapped error so the user knows how far a mutation got before it
 * failed (e.g. a signed-but-not-yet-broadcast tx behaves very differently
 * from one that never got built).
 */
export type MutationStage = "build" | "sign" | "broadcast";

/**
 * Wraps an error thrown mid-pipeline (see `useExecuteDraft`) so a caller that
 * cares can recover which stage (sign/broadcast) threw, while every existing
 * caller that just does `String(error)` / `mapError(error)` on it — with no
 * idea `StagedError` exists — keeps seeing byte-identical output, since
 * `toString()` delegates straight to the original error.
 */
export class StagedError extends Error {
  readonly stage: MutationStage;
  readonly original: unknown;

  constructor(stage: MutationStage, original: unknown) {
    super(String(original));
    this.name = "StagedError";
    this.stage = stage;
    this.original = original;
  }

  toString(): string {
    return String(this.original);
  }
}

/** Stage of a `StagedError`, or `undefined` for any other error shape. */
export function stageOf(error: unknown): MutationStage | undefined {
  return error instanceof StagedError ? error.stage : undefined;
}

/** The wrapped original error for a `StagedError`, else the value itself. */
export function unwrapStaged(error: unknown): unknown {
  return error instanceof StagedError ? error.original : error;
}

const STAGE_LABEL: Record<MutationStage, string> = {
  build: "Build failed",
  sign: "Sign failed",
  broadcast: "Broadcast failed",
};

export function mapError(error: unknown, stage?: MutationStage): string {
  const raw = String(error);
  let mapped: string | undefined;

  for (const [pattern, message] of Object.entries(ERROR_MAP)) {
    if (raw.toLowerCase().includes(pattern.toLowerCase())) {
      mapped = message;
      break;
    }
  }

  if (mapped === undefined) {
    // Strip technical prefixes
    mapped =
      raw
        .replace(/^Error invoking remote method .*?: /, "")
        .replace(/^HTTP error: /, "")
        .replace(/^error: /i, "")
        .trim() || "An unexpected error occurred";
  }

  return stage ? `${STAGE_LABEL[stage]}: ${mapped}` : mapped;
}
