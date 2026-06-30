export function dollarydoosToHns(dollarydoos: number): string {
  return (dollarydoos / 1_000_000).toFixed(6);
}

export function hnsToDollarydoos(hns: string): number {
  return Math.round(parseFloat(hns) * 1_000_000);
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
  try {
    return new Date(iso + "Z").toLocaleString();
  } catch {
    return iso;
  }
}

export function truncate(str: string, len: number): string {
  if (str.length <= len) return str;
  return str.slice(0, len) + "...";
}
