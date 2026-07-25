#!/usr/bin/env node
/**
 * Lint: verify that src/secure/** never imports from the main React app.
 *
 * The secure window is a separate Vite entry point that MUST NOT share code
 * with the React bundle. If a file in src/secure/ imports from src/ outside
 * that subtree (e.g. `import { foo } from "../lib/bar"`), the trust boundary
 * is violated — React-bundle code (and its transitive deps) would end up in
 * the secure window's bundle, widening the attack surface.
 *
 * Allowed imports:
 *   - Relative imports within src/secure/ (e.g. "./render", "../render")
 *   - Package imports (e.g. "@tauri-apps/api/core", "vitest")
 *
 * Disallowed:
 *   - Any relative import that resolves outside src/secure/ (e.g. "../lib/foo")
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve, dirname, relative } from "node:path";

const SECURE_DIR = resolve("src/secure");

function collectTsFiles(dir) {
  const results = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const stat = statSync(full);
    if (stat.isDirectory()) {
      results.push(...collectTsFiles(full));
    } else if (/\.(ts|tsx|js|jsx|mjs)$/.test(entry)) {
      results.push(full);
    }
  }
  return results;
}

// Match: import ... from "..." or import "..." (side-effect)
// Also match: export ... from "..."
const IMPORT_RE = /(?:import|export)\s+.*?\s+from\s+['"]([^'"]+)['"]|import\s+['"]([^'"]+)['"]/g;

let violations = 0;

for (const file of collectTsFiles(SECURE_DIR)) {
  const content = readFileSync(file, "utf8");
  const lines = content.split("\n");
  for (let i = 0; i < lines.length; i++) {
    let match;
    IMPORT_RE.lastIndex = 0;
    while ((match = IMPORT_RE.exec(lines[i])) !== null) {
      const specifier = match[1] || match[2];
      // Only check relative imports (start with . or ..)
      if (!specifier.startsWith(".")) continue;
      // Resolve the import target relative to the importing file
      const target = resolve(dirname(file), specifier);
      // Check if the resolved path is still within src/secure/
      const rel = relative(SECURE_DIR, target);
      if (rel.startsWith("..")) {
        const shortFile = relative(process.cwd(), file);
        console.error(
          `ERROR: ${shortFile}:${i + 1} imports "${specifier}" which resolves outside src/secure/`
        );
        violations++;
      }
    }
  }
}

if (violations > 0) {
  console.error(
    `\n${violations} import boundary violation(s) found. ` +
      `The secure window must not import from the main React app.`
  );
  process.exit(1);
} else {
  console.log("OK: src/secure/ import boundary is clean.");
}
