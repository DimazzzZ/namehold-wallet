#!/usr/bin/env node
/**
 * Lint: no native `title` attributes in the React app — use `<Tooltip>`.
 *
 * A native title cannot be styled or positioned, has no hover debounce we
 * control, and — the reason this rule exists — is never shown on an element
 * with `pointer-events: none`. Every disabled `Button` carries exactly that, so
 * `title={reasonItIsDisabled}` was invisible precisely when it mattered.
 *
 * `title` is also an ordinary prop name on several of our components (Dialog,
 * PageHeader, Card, Alert, EmptyState — and Badge, which renders a Tooltip
 * itself), so this only flags `title=` on a lowercase DOM tag or on a component
 * known to spread its props onto one.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";

const SRC = resolve("src");

/** Components that forward unknown props straight onto a DOM element. */
const SPREADS_TO_DOM = new Set(["Button", "NavLink", "Link"]);

function collectTsxFiles(dir) {
  const results = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      results.push(...collectTsxFiles(full));
    } else if (entry.endsWith(".tsx")) {
      results.push(full);
    }
  }
  return results;
}

/** The tag whose attribute list line `i` belongs to, or null. */
function owningTag(lines, i) {
  for (let j = i; j >= 0 && j > i - 40; j--) {
    const m = /<([A-Za-z][A-Za-z0-9.]*)/.exec(lines[j]);
    if (m && (lines[j].trimStart().startsWith("<") || j === i)) return m[1];
  }
  return null;
}

let violations = 0;

for (const file of collectTsxFiles(SRC)) {
  const lines = readFileSync(file, "utf8").split("\n");
  for (let i = 0; i < lines.length; i++) {
    if (!/(^|\s)title=/.test(lines[i])) continue;
    const tag = owningTag(lines, i);
    if (!tag) continue;
    const isDom = tag[0] === tag[0].toLowerCase();
    if (!isDom && !SPREADS_TO_DOM.has(tag)) continue;
    console.error(
      `ERROR: ${relative(process.cwd(), file)}:${i + 1} sets a native title on <${tag}>. ` +
        `Wrap it in <Tooltip content={...}> instead.`,
    );
    violations++;
  }
}

if (violations > 0) {
  console.error(
    `\n${violations} native title attribute(s) found. ` +
      `The app uses <Tooltip> so hints are styled, positioned, debounced, ` +
      `and visible on disabled controls.`,
  );
  process.exit(1);
} else {
  console.log("OK: no native title attributes.");
}
