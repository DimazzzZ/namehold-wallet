#!/usr/bin/env node
// Refuses to start a release from a commit that is not on main with green CI.
//
//   release-gate.mjs <compare-status> <check-runs.tsv>
//
// <compare-status> is `status` of the compare API for main...<commit>; the
// tsv lists the commit's check runs as `name<TAB>conclusion`. Run by the
// prepare job of release.yml.
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

// Name prefixes: the Rust tests job's name carries a test count. A job renamed
// without updating this stops every release, and a test reads the workflows.
export const REQUIRED_CHECKS = [
  "Frontend (typecheck",
  "Rust lint (",
  "Rust tests (",
  "Validate workflow YAML",
  "E2E Tests (",
];

/** Everything wrong with this commit; empty means it may be released. */
export function gateProblems(compareStatus, runs) {
  const problems = [];
  // "behind": main has moved on, the commit is in its history.
  if (compareStatus !== "identical" && compareStatus !== "behind") {
    problems.push(`the commit is not on main (compare status "${compareStatus}")`);
  }
  for (const required of REQUIRED_CHECKS) {
    const matching = runs.filter((r) => r.name.startsWith(required));
    if (matching.length === 0) problems.push(`no check run named "${required}..." on this commit`);
    for (const r of matching) {
      if (r.conclusion !== "success") {
        problems.push(`"${r.name}" is "${r.conclusion || "not finished"}", not "success"`);
      }
    }
  }
  return problems;
}

export function parseRuns(text) {
  return text
    .split(/\r?\n/)
    .filter((l) => l !== "")
    .map((l) => {
      const [name, conclusion = ""] = l.split("\t");
      return { name, conclusion };
    });
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const [status, file] = process.argv.slice(2);
  const problems =
    file === undefined
      ? ["usage: release-gate.mjs <compare-status> <check-runs.tsv>"]
      : gateProblems(status, parseRuns(readFileSync(file, "utf8")));
  for (const p of problems) console.error(`::error::${p}`);
  if (problems.length > 0) process.exit(1);
  console.log("release-gate: the commit is on main and its CI passed");
}
