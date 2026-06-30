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
  "timed out": "The request timed out. Please try again.",
  timeout: "The request timed out. Please try again.",
  // Signer state (still valid in the non-custodial model).
  "wallet locked": "Your signer is locked — click Unlock first.",
  "wallet is locked": "Your signer is locked — click Unlock first.",
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

export function mapError(error: unknown): string {
  const raw = String(error);
  
  for (const [pattern, message] of Object.entries(ERROR_MAP)) {
    if (raw.toLowerCase().includes(pattern.toLowerCase())) {
      return message;
    }
  }

  // Strip technical prefixes
  return raw
    .replace(/^Error invoking remote method .*?: /, "")
    .replace(/^HTTP error: /, "")
    .replace(/^error: /i, "")
    .trim() || "An unexpected error occurred";
}
