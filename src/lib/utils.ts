export function dollarydoosToHns(dollarydoos: number): string {
  return addThousandSeparators((dollarydoos / 1_000_000).toFixed(6));
}

/**
 * Insert commas every three digits left of the decimal point.
 * "1234567.123456" → "1,234,567.123456"
 * Purely cosmetic — never touches IDs or non-numeric strings.
 */
function addThousandSeparators(nStr: string): string {
  const [intPart = "", decPart] = nStr.split(".");
  const withCommas = intPart.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return decPart != null ? `${withCommas}.${decPart}` : withCommas;
}

/**
 * Format an integer count (domains, selected items, etc.) with thousand
 * separators. NOT for IDs — those stay raw.
 */
export function formatCount(value: number): string {
  return addThousandSeparators(String(value));
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

/**
 * Format a decimal HNS amount with thousands separators and exactly 6
 * fractional digits — matching the precision used by `formatHns` (which
 * converts from dollarydoos via `.toFixed(6)`).
 *
 * Examples: 120002.4 → "120,002.400000", 1000 → "1,000.000000",
 *           0.5 → "0.500000"
 */
export function formatHnsAmount(value: number | string): string {
  const num = typeof value === "string" ? Number(value) : value;
  if (!Number.isFinite(num)) return String(value);
  return num.toLocaleString("en-US", {
    minimumFractionDigits: 6,
    maximumFractionDigits: 6,
  });
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
