// Tests for release-gate.mjs (node --test).
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { gateProblems, parseRuns, REQUIRED_CHECKS } from "./release-gate.mjs";

const SCRIPT = fileURLToPath(new URL("./release-gate.mjs", import.meta.url));
const WORKFLOWS = fileURLToPath(new URL("../.github/workflows/", import.meta.url));
const green = [
  "Frontend (typecheck · build · test)",
  "Rust lint (fmt · clippy)",
  "Rust tests",
  "Validate workflow YAML",
  "E2E Tests (Tauri App)",
].map((name) => ({ name, conclusion: "success" }));

test("a commit on main with every required check green may be released", () => {
  assert.deepEqual(gateProblems("identical", green), []);
  assert.deepEqual(gateProblems("behind", green), [], "main has moved on");
  const extra = [...green, { name: "Dependency audit", conclusion: "failure" }];
  assert.deepEqual(gateProblems("behind", extra), [], "the audit is advisory");
});

test("a commit that is not in main's history is refused", () => {
  for (const status of ["ahead", "diverged", "", undefined]) {
    assert.match(gateProblems(status, green).join(), /not on main/, String(status));
  }
});

test("a required check missing, failed, cancelled or unfinished is refused", () => {
  assert.equal(gateProblems("identical", []).length, REQUIRED_CHECKS.length);
  for (const [i, required] of REQUIRED_CHECKS.entries()) {
    const without = green.filter((_, j) => j !== i);
    assert.match(gateProblems("identical", without).join(), /no check run/, required);
    for (const conclusion of ["failure", "cancelled", "skipped", ""]) {
      const runs = green.map((r, j) => (j === i ? { ...r, conclusion } : r));
      assert.equal(gateProblems("identical", runs).length, 1, `${required} ${conclusion}`);
    }
  }
});

test("every required check is the name of a job in a workflow", () => {
  const names = readdirSync(WORKFLOWS).flatMap((f) =>
    [...readFileSync(join(WORKFLOWS, f), "utf8").matchAll(/^ {4}name: (.+)$/gm)].map((m) => m[1]),
  );
  for (const required of REQUIRED_CHECKS) {
    assert.ok(
      names.some((n) => n.startsWith(required)),
      required,
    );
  }
});

test("the check runs parse, a run with no conclusion included", () => {
  assert.deepEqual(parseRuns("a b\tsuccess\nc\t\n\n"), [
    { name: "a b", conclusion: "success" },
    { name: "c", conclusion: "" },
  ]);
});

test("the command line exits 0 only for a commit that passes", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-"));
  const tsv = join(dir, "runs.tsv");
  writeFileSync(tsv, green.map((r) => `${r.name}\t${r.conclusion}`).join("\n"));
  const exit = (...args) => spawnSync("node", [SCRIPT, ...args]).status;
  assert.equal(exit("behind", tsv), 0);
  assert.equal(exit("diverged", tsv), 1);
  assert.equal(exit("behind"), 1);
});
