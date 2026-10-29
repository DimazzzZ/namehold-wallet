/**
 * The pure half of `scripts/fetch-icann-strings.ts`: parsing ICANN's CSV
 * export and building the strings list the ICANN collision flag reads
 * (docs/specs/2026-10-29-icann-collision-flag.md, R1). No Node imports, so
 * the app's type check and vitest can load it; the fetching and writing live
 * in the script next to it.
 *
 * Rules (spec Terms):
 * - only `Active` applications count;
 * - an applicant whose `-R` replacement row is active switched strings, so
 *   the original row is dropped and the replacement's string and id are kept;
 * - a variant string (`xn--3ds831g (在線)`) becomes its own key with
 *   `variantOf` naming the primary;
 * - contention is copied only where the id and the kept string both match a
 *   Reveal Day row, so a replaced string does not carry its old contention.
 */
export const APPLICATIONS_EXPORT_URL =
  "https://newgtldprogram-aps.icann.org/aps/applications/export/csv";
export const CONTENTION_SETS_URL =
  "https://newgtldprogram.icann.org/en/application-rounds/round2/reveal-day-contention-sets.csv";
export const OUTPUT_PATH = "src/data/icann-gtld-2026.json";

export type IcannApplication = { id: string; applicant: string; contention: boolean };
export type IcannEntry = { uLabel?: string; variantOf?: string; applications: IcannApplication[] };
export type IcannList = {
  round: "2026";
  fetchedAt: string;
  source: { applications: string; contentionSets: string };
  strings: Record<string, IcannEntry>;
};

export type ApplicationRow = {
  id: string;
  applicant: string;
  primary: string;
  uLabel: string;
  /** Variant strings as `{ aLabel, uLabel }`, already split from `xn--… (…)`. */
  variants: { aLabel: string; uLabel: string }[];
  status: "Active" | "Deactivated";
  type: string;
};

const REQUIRED_COLUMNS = [
  "Primary String",
  "Primary String Unicode Label",
  "TLD Variants",
  "Application ID",
  "Applicant Name",
  "Application Type",
  "Application Status",
] as const;

/** A small RFC 4180 reader: quoted fields, doubled quotes, CRLF or LF. */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  let i = 0;
  if (text.charCodeAt(0) === 0xfeff) i = 1;
  for (; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else {
          quoted = false;
        }
      } else {
        field += c;
      }
    } else if (c === '"') {
      quoted = true;
    } else if (c === ",") {
      row.push(field);
      field = "";
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
    } else {
      field += c;
    }
  }
  if (field !== "" || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  return rows;
}

function layoutError(what: string): Error {
  return new Error(
    `unexpected ICANN export layout: ${what}. The export format changed; read the file and update scripts/fetch-icann-strings.ts.`,
  );
}

/** Split `xn--3ds831g (在線)` into its A-label and Unicode form. */
function splitVariant(cell: string): { aLabel: string; uLabel: string } | null {
  const v = cell.trim();
  if (!v || v === "N/A") return null;
  const m = /^(\S+)(?:\s*\((.*)\))?$/.exec(v);
  if (!m) throw layoutError(`variant cell "${cell}"`);
  return { aLabel: (m[1] ?? "").toLowerCase(), uLabel: m[2] ?? "" };
}

export function parseApplicationsCsv(text: string): ApplicationRow[] {
  const rows = parseCsv(text);
  const [disclaimer, blank, header] = rows;
  if (!disclaimer || disclaimer.length !== 1 || !/^Please note/.test(disclaimer[0] ?? "")) {
    throw layoutError("the first line is not the disclaimer");
  }
  if (!blank || blank.join("") !== "") throw layoutError("the second line is not blank");
  if (!header) throw layoutError("no header line");
  const col = {} as Record<(typeof REQUIRED_COLUMNS)[number], number>;
  for (const c of REQUIRED_COLUMNS) {
    const i = header.indexOf(c);
    if (i < 0) throw layoutError(`column "${c}" is missing`);
    col[c] = i;
  }
  const out: ApplicationRow[] = [];
  for (const r of rows.slice(3)) {
    if (r.length === 1 && r[0] === "") continue;
    if (r.length !== header.length)
      throw layoutError(`a row has ${r.length} cells, header ${header.length}`);
    const cell = (c: (typeof REQUIRED_COLUMNS)[number]): string => (r[col[c]] ?? "").trim();
    const status = cell("Application Status");
    if (status !== "Active" && status !== "Deactivated") throw layoutError(`status "${status}"`);
    const variant = splitVariant(cell("TLD Variants"));
    out.push({
      id: cell("Application ID"),
      applicant: cell("Applicant Name"),
      primary: cell("Primary String").toLowerCase(),
      uLabel: cell("Primary String Unicode Label"),
      variants: variant ? [variant] : [],
      status,
      type: cell("Application Type"),
    });
  }
  return out;
}

export function parseContentionCsv(text: string): { id: string; string: string }[] {
  const rows = parseCsv(text);
  const header = rows[0] ?? [];
  const idCol = header.indexOf("Application ID");
  const strCol = header.indexOf("Applied-for string(s)");
  if (idCol < 0 || strCol < 0) throw layoutError("contention sets header");
  const out: { id: string; string: string }[] = [];
  for (const r of rows.slice(1)) {
    if (r.length === 1 && r[0] === "") continue;
    const id = (r[idCol] ?? "").trim();
    for (const s of (r[strCol] ?? "").split("/")) {
      const string = s.trim().toLowerCase();
      if (string) out.push({ id, string });
    }
  }
  return out;
}

const baseId = (id: string): string => id.replace(/-R$/, "");

export function buildStringsList(
  rows: ApplicationRow[],
  contention: { id: string; string: string }[],
  fetchedAt: string,
): IcannList {
  const active = rows.filter((r) => r.status === "Active");
  const groups = new Map<string, ApplicationRow[]>();
  for (const r of active) {
    const g = groups.get(baseId(r.id)) ?? [];
    g.push(r);
    groups.set(baseId(r.id), g);
  }
  const kept: ApplicationRow[] = [];
  for (const g of groups.values()) {
    const replacements = g.filter((r) => r.id.endsWith("-R"));
    kept.push(...(replacements.length > 0 ? replacements : g));
  }
  const inContention = new Set(contention.map((c) => `${c.id}|${c.string}`));
  const strings: Record<string, IcannEntry> = {};
  const add = (key: string, entry: Omit<IcannEntry, "applications">, app: IcannApplication) => {
    const e = (strings[key] ??= { ...entry, applications: [] });
    e.applications.push(app);
  };
  for (const r of kept) {
    const app = (string: string): IcannApplication => ({
      id: r.id,
      applicant: r.applicant,
      contention: inContention.has(`${r.id}|${string}`),
    });
    add(r.primary, r.uLabel ? { uLabel: r.uLabel } : {}, app(r.primary));
    for (const v of r.variants) {
      add(
        v.aLabel,
        { ...(v.uLabel ? { uLabel: v.uLabel } : {}), variantOf: r.primary },
        app(v.aLabel),
      );
    }
  }
  // "In contention" means more than one applicant for the string (spec Terms).
  // The Reveal Day CSV predates the Replacement Period, so a string whose
  // rivals all switched away would keep a stale flag on its lone applicant.
  for (const e of Object.values(strings)) {
    if (new Set(e.applications.map((a) => a.applicant)).size <= 1) {
      for (const a of e.applications) a.contention = false;
    }
  }
  const sorted: Record<string, IcannEntry> = {};
  for (const k of Object.keys(strings).sort()) sorted[k] = strings[k]!;
  return {
    round: "2026",
    fetchedAt,
    source: { applications: APPLICATIONS_EXPORT_URL, contentionSets: CONTENTION_SETS_URL },
    strings: sorted,
  };
}

export function countApplications(list: IcannList): number {
  return new Set(Object.values(list.strings).flatMap((e) => e.applications.map((a) => a.id))).size;
}
