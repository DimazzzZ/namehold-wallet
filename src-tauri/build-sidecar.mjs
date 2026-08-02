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
import { copyFileSync, mkdirSync, statSync } from "node:fs";
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

// Skip if the sidecar is already staged (non-empty). This avoids a redundant
// rebuild when the release workflow stages both architectures before tauri-action
// invokes `beforeBuildCommand`.
const destDir = resolve("binaries");
mkdirSync(destDir, { recursive: true });
const dest = resolve(destDir, `namehold-syncd-${triple}${ext}`);
try {
  const st = statSync(dest);
  if (st.size > 0) {
    console.log(`Sidecar already staged: ${dest} (${st.size} bytes) — skipping build`);
    process.exit(0);
  }
} catch {
  // File doesn't exist — proceed with build.
}

console.log(`Building namehold-syncd (release, target=${triple})`);
execSync("cargo build --release --bin namehold-syncd", { stdio: "inherit" });

const src = resolve("target", "release", `namehold-syncd${ext}`);
copyFileSync(src, dest);
console.log(`Staged: ${dest}`);
