// Cross-platform sidecar staging for Tauri's externalBin.
// Compiles `namehold-syncd` and copies it to
// `src-tauri/binaries/namehold-syncd-<host-triple>[.exe]`, which is where
// Tauri looks when bundling.
//
// Profile: defaults to `release` for shipping builds. Override with
// `SIDECAR_PROFILE=debug` when the outer build is `tauri build --debug` (E2E
// smoke lane) so we don't burn ~20–50s of release codegen+link on a sidecar
// the tests never even launch.
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

// Select the cargo profile. Default `release` preserves the previous behavior
// (dev machines and release.yml keep shipping a fully optimized sidecar).
// `SIDECAR_PROFILE=debug` opts into a debug sidecar for the E2E lane.
const rawProfile = (process.env.SIDECAR_PROFILE || "release").toLowerCase();
if (rawProfile !== "release" && rawProfile !== "debug") {
  throw new Error(
    `Unsupported SIDECAR_PROFILE="${process.env.SIDECAR_PROFILE}" — expected "release" or "debug"`,
  );
}
const isRelease = rawProfile === "release";
// `cargo build` puts the artifact under target/debug/ for the default (debug)
// profile and target/release/ for --release. Match that layout.
const cargoFlag = isRelease ? " --release" : "";
const targetDir = isRelease ? "release" : "debug";

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

console.log(`Building namehold-syncd (${rawProfile}, target=${triple})`);
execSync(`cargo build${cargoFlag} --bin namehold-syncd`, { stdio: "inherit" });

const src = resolve("target", targetDir, `namehold-syncd${ext}`);
copyFileSync(src, dest);
console.log(`Staged: ${dest}`);
