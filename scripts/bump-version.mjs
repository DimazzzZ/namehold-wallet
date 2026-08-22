#!/usr/bin/env node
/**
 * Bump the app version in every config file that owns a copy of it, in one
 * atomic command.
 *
 * Usage:
 *   node scripts/bump-version.mjs 0.6.0
 *   pnpm version:set 0.6.0
 *
 * Updates:
 *   - package.json                 ("version")
 *   - src-tauri/tauri.conf.json    ("version")
 *   - src-tauri/Cargo.toml         (package `version`)
 *
 * Then runs `cargo check` in src-tauri/ to refresh Cargo.lock so the whole
 * bump lands in a single, committable state.
 *
 * The frontend does NOT need bumping — Layout.tsx / AboutPage.tsx read the
 * live version from the backend (`current_version` command → CARGO_PKG_VERSION),
 * and the webqa mock reads `__APP_VERSION__`, which Vite injects from
 * package.json at build time.
 */

import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, "..");

const SEMVER = /^\d+\.\d+\.\d+$/;

function fail(msg) {
  console.error(`✗ ${msg}`);
  process.exit(1);
}

const next = process.argv[2];
if (!next) {
  fail("missing version argument. Usage: pnpm version:set X.Y.Z");
}
if (!SEMVER.test(next)) {
  fail(`invalid version "${next}". Expected semver X.Y.Z (e.g. 0.6.0).`);
}

const pkgPath = join(repoRoot, "package.json");
const confPath = join(repoRoot, "src-tauri", "tauri.conf.json");
const cargoPath = join(repoRoot, "src-tauri", "Cargo.toml");

// ── package.json ────────────────────────────────────────────────────────────
const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
const prev = pkg.version;
pkg.version = next;
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");

// ── tauri.conf.json ───────────────────────────────────────────────────────────
const conf = JSON.parse(readFileSync(confPath, "utf8"));
conf.version = next;
writeFileSync(confPath, JSON.stringify(conf, null, 2) + "\n");

// ── Cargo.toml ────────────────────────────────────────────────────────────────
// Replace only the first top-level `version = "..."` line (the [package]
// version). Anchored to line start so dependency versions are never touched.
const cargo = readFileSync(cargoPath, "utf8");
if (!/^version = "[^"]*"$/m.test(cargo)) {
  fail(`could not find a top-level \`version = "..."\` line in ${cargoPath}`);
}
const cargoNext = cargo.replace(/^version = "[^"]*"$/m, `version = "${next}"`);
writeFileSync(cargoPath, cargoNext);

console.log(`✓ Bumped ${prev} → ${next} in package.json, tauri.conf.json, Cargo.toml`);

// ── Refresh Cargo.lock ────────────────────────────────────────────────────────
console.log("→ Running `cargo check` to update Cargo.lock ...");
const check = spawnSync("cargo", ["check", "--manifest-path", cargoPath], {
  stdio: "inherit",
});
if (check.error) {
  fail(`failed to run cargo: ${check.error.message}`);
}
if (check.status !== 0) {
  fail(`cargo check exited with code ${check.status}. Config files were updated, but Cargo.lock may be stale — fix the build and re-run cargo check.`);
}

console.log(`✓ Cargo.lock updated. Version is now ${next} everywhere.`);
