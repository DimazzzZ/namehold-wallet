export const NAMES_ROUTE = "/names";

export type NameTab = "overview" | "dns" | "transfer" | "sign";
const TABS: NameTab[] = ["overview", "dns", "transfer", "sign"];

/** A name's page; the raw ACE name is encoded, overview carries no ?tab. */
export function namePath(rawName: string, tab?: NameTab): string {
  const base = `${NAMES_ROUTE}/${encodeURIComponent(rawName)}`;
  return tab && tab !== "overview" ? `${base}?tab=${tab}` : base;
}

/** ?tab= → a tab; unknown or missing → overview. */
export function parseNameTab(v: string | null): NameTab {
  return TABS.includes(v as NameTab) ? (v as NameTab) : "overview";
}
