export function dollarydoosToHns(dollarydoos: number): string {
  return (dollarydoos / 1_000_000).toFixed(6);
}

export function hnsToDollarydoos(hns: string): number {
  return Math.round(parseFloat(hns) * 1_000_000);
}

/** The bech32 HRP+separator an HNS address starts with on a given network. */
export function hnsAddressPrefix(network: string): string {
  switch (network) {
    case "testnet":
      return "ts1";
    case "regtest":
      return "rs1";
    case "simnet":
      return "ss1";
    default:
      return "hs1"; // mainnet
  }
}

/**
 * Lightweight, network-aware format check for inline UI feedback — verifies the
 * bech32 prefix matches the network and the shape is plausible. NOT a checksum;
 * the Rust `address::decode` remains the source of truth at build time.
 */
export function isLikelyHnsAddress(addr: string, network: string): boolean {
  const a = addr.trim().toLowerCase();
  return (
    a.startsWith(hnsAddressPrefix(network)) &&
    a.length >= 40 &&
    a.length <= 90 &&
    /^[a-z0-9]+$/.test(a)
  );
}

export function formatHns(dollarydoos: number | null | undefined): string {
  if (dollarydoos == null) return "—";
  return dollarydoosToHns(dollarydoos);
}

export function cn(...classes: (string | false | null | undefined)[]): string {
  return classes.filter(Boolean).join(" ");
}

export function formatDate(iso: string | null | undefined): string {
  if (!iso) return "—";
  const s = iso.trim();
  if (!s) return "—";

  // Normalize before parsing so we don't double-stamp a timezone. Inputs vary:
  //   * Namebase ISO already carries a tz: "2026-06-26T00:00:00Z" / "…+02:00"
  //   * SQLite naive UTC: "2026-06-26 00:00:00" (space, no tz)
  //   * date-only: "2026-06-26"
  // A naive value is treated as UTC; a value that already has a tz is left as-is.
  const hasTz = /[zZ]$/.test(s) || /[+-]\d{2}:?\d{2}$/.test(s);
  let normalized = s;
  if (!hasTz) {
    if (/^\d{4}-\d{2}-\d{2}$/.test(s)) {
      normalized = `${s}T00:00:00Z`;
    } else {
      normalized = `${s.replace(" ", "T")}Z`;
    }
  }

  const d = new Date(normalized);
  if (Number.isNaN(d.getTime())) return s; // unparseable → show the raw value, never "Invalid Date"
  return d.toLocaleString();
}

export function truncate(str: string, len: number): string {
  if (str.length <= len) return str;
  return str.slice(0, len) + "...";
}
