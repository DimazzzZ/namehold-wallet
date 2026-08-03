// Cross-platform sidecar staging for Tauri's externalBin.
// Compiles `namehold-syncd` (release profile) and copies it to
// `src-tauri/binaries/namehold-syncd-<host-triple>[.exe]`, which is where
// Tauri looks when bundling.
//
// Wired into `beforeBuildCommand` in tauri.conf.json, so `pnpm tauri build`
// gets a real daemon binary bundled without extra developer steps.
//
// The bash equivalent (`build-sidecar.sh`) is kept for direct manual use.
import { execSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, statSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
process.chdir(scriptDir);

// `rustc --print host-tuple` is Rust >= 1.84. Older toolchains: fall back to
// parsing `rustc -Vv`.
function hostTriple() {
  try {
    return execSync("rustc --print host-tuple", { encoding: "utf8" }).trim();
  } catch {
    const vv = execSync("rustc -Vv", { encoding: "utf8" });
    const match = vv.match(/^host:\s*(.+)$/m);
    if (!match) throw new Error("Cannot determine rustc host triple");
    return match[1].trim();
  }
}

const triple = hostTriple();
const isWin = process.platform === "win32";
const ext = isWin ? ".exe" : "";
const debug = process.argv.includes("--debug");
const profile = debug ? "debug" : "release";

// Release jobs can pre-stage architecture-specific binaries. Development mode
// always runs Cargo's incremental build so source changes cannot leave a stale
// or empty placeholder beside the app executable.
const destDir = resolve("binaries");
mkdirSync(destDir, { recursive: true });
const dest = resolve(destDir, `namehold-syncd-${triple}${ext}`);
if (!debug) {
  try {
    const st = statSync(dest);
    if (st.size > 0) {
      console.log(`Sidecar already staged: ${dest} (${st.size} bytes) — skipping build`);
      process.exit(0);
    }
  } catch {
    // File doesn't exist — proceed with build.
  }
}

console.log(`Building namehold-syncd (${profile}, target=${triple})`);
execSync(`cargo build ${debug ? "" : "--release "}--bin namehold-syncd`, {
  stdio: "inherit",
});

const metadata = JSON.parse(
  execSync("cargo metadata --no-deps --format-version 1", { encoding: "utf8" }),
);
const src = resolve(metadata.target_directory, profile, `namehold-syncd${ext}`);
copyFileSync(src, dest);
if (!isWin) chmodSync(dest, 0o755);
console.log(`Staged: ${dest}`);
