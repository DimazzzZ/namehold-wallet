#!/usr/bin/env node
/**
 * Refresh `src/data/icann-gtld-2026.json`, the list of strings applied for
 * as new gTLDs in ICANN's 2026 round that the ICANN collision flag reads
 * (docs/specs/2026-10-29-icann-collision-flag.md, R1).
 *
 * Run with `just icann-strings` (`node scripts/fetch-icann-strings.ts`; Node
 * strips the types). No dependencies. The app never calls ICANN itself: this
 * file is the only thing that does, and its output is committed.
 *
 * Sources (verified 2026-10-29):
 * - the Application Publication site's CSV export, a POST whose body is the
 *   site's own search request; it returns every application whatever the
 *   page size. The file starts with a BOM, one quoted disclaimer line and a
 *   blank line, then the header.
 * - the Reveal Day contention sets CSV, keyed by application id and string.
 *
 * The parsing and the rules live in `scripts/icann-strings.ts`, which the
 * tests load; this file only fetches, guards the count and writes.
 */
import { writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import {
  APPLICATIONS_EXPORT_URL,
  CONTENTION_SETS_URL,
  OUTPUT_PATH,
  buildStringsList,
  countApplications,
  parseApplicationsCsv,
  parseContentionCsv,
} from "./icann-strings.ts";

/** ICANN's published application count on 2026-10-07; the guard below allows withdrawals. */
const PUBLISHED_APPLICATIONS = 1615;
const COUNT_TOLERANCE = 200;

async function main(): Promise<void> {
  const exportBody = {
    searchAndFilter: {},
    sort: { field: "primaryString", direction: "ASC" },
    pagination: { pageNum: 0, pageSize: 10 },
  };
  const [appsRes, contRes] = await Promise.all([
    fetch(APPLICATIONS_EXPORT_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(exportBody),
    }),
    fetch(CONTENTION_SETS_URL, { redirect: "follow" }),
  ]);
  if (!appsRes.ok) throw new Error(`applications export: HTTP ${appsRes.status}`);
  if (!contRes.ok) throw new Error(`contention sets: HTTP ${contRes.status}`);
  const rows = parseApplicationsCsv(await appsRes.text());
  const contention = parseContentionCsv(await contRes.text());
  const list = buildStringsList(rows, contention, new Date().toISOString());
  const applications = countApplications(list);
  if (Math.abs(applications - PUBLISHED_APPLICATIONS) > COUNT_TOLERANCE) {
    throw new Error(
      `built ${applications} applications, but ICANN published ${PUBLISHED_APPLICATIONS}; more than ${COUNT_TOLERANCE} apart, so the export format or the replacement rule probably changed. Nothing written.`,
    );
  }
  writeFileSync(OUTPUT_PATH, JSON.stringify(list, null, 2) + "\n");
  const entries = Object.values(list.strings);
  const variants = entries.filter((e) => e.variantOf).length;
  const contended = entries.filter((e) => e.applications.some((a) => a.contention)).length;
  console.log(
    `applications ${applications}, strings ${entries.length}, variants ${variants}, in contention ${contended} → ${OUTPUT_PATH}`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => {
    console.error(e instanceof Error ? e.message : e);
    process.exit(1);
  });
}
