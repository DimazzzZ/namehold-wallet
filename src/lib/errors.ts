const ERROR_MAP: Record<string, string> = {
  "connection refused": "Cannot connect to wallet. Is hsd running?",
  "ECONNREFUSED": "Cannot connect to wallet. Is hsd running?",
  "connection reset": "Connection lost. Check if hsd is still running.",
  "timed out": "Connection timed out. hsd may be syncing or overloaded.",
  "timeout": "Connection timed out. hsd may be syncing or overloaded.",
  "Unauthorized": "Invalid API key. Check your settings.",
  "unauthorized": "Invalid API key. Check your settings.",
  "insufficient funds": "Insufficient HNS balance for this transaction.",
  "Insufficient funds": "Insufficient HNS balance for this transaction.",
  "error decoding response body": "Wallet returned unexpected data. Try refreshing.",
  "Not found": "Wallet or endpoint not found. Check wallet ID in settings.",
  "bad API key": "Invalid API key. Check your settings.",
  "wallet is locked": "Wallet is locked. Unlock it with your passphrase.",
  "Wallet is locked": "Wallet is locked. Unlock it with your passphrase.",
  "no such wallet": "Wallet not found. Check wallet ID in settings.",
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
