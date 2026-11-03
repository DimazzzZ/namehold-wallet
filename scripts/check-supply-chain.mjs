#!/usr/bin/env node
// Supply-chain checks on the CI and release pipeline: what runs in a workflow,
// with which token and which secret, from which exact code. GitHub's own
// settings (`sha_pinning_required`, allowed actions) are the enforcement for
// pins; this check catches the change in review, before it reaches main.
//
// Run from anywhere: `pnpm lint:supply-chain` (also runs the tests in
// check-supply-chain.node-test.mjs). Workflow Validation runs it on every pull
// request. Exit status 1 and one line per finding: `file:line: [rule] message`.
//
// The workflow reader below is a line scan, so the check needs no packages. It
// understands the layout these files use (block YAML, two-space indents, steps
// as `- ` items under `jobs.<id>.steps`), and yamlIndirection refuses the YAML
// that would let GitHub read a line differently: flow mappings, quoted, complex
// or tagged keys and nodes, double-quoted escapes, anchors, aliases, merge keys.
// A checkout it cannot place in a step is a finding too. The check runs after a
// workflow pushed to a branch has already started, so it catches changes in
// review; it does not stop them. Someone who can edit release.yml can edit this
// file in the same commit.

import { spawnSync } from "node:child_process";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const SHA40 = /^[0-9a-f]{40}$/;

/** Split a `key: value  # comment` value into its value and comment. */
export function splitValue(raw) {
  let value = raw.trim();
  let comment = "";
  const hash = value.search(/\s#/);
  if (hash >= 0) {
    comment = value.slice(hash).replace(/^\s*#\s?/, "");
    value = value.slice(0, hash).trim();
  }
  if (/^(["']).*\1$/.test(value)) value = value.slice(1, -1);
  return { value, comment };
}

const isStructural = (text) => text.trim() !== "" && !text.trim().startsWith("#");
const indentOf = (text) => text.length - text.trimStart().length;

/**
 * Read a workflow's jobs and steps by indentation. Line numbers are 1-based.
 * A step's `keys` are its own mapping keys (`uses`, `run`, `env`, ...);
 * `with` holds the keys under its `with:`. A step spans lines `line` to `end`
 * (1-based, inclusive), comment and blank lines before the next step included.
 */
export function readWorkflow(text) {
  const lines = text.split(/\r?\n/);
  const jobs = [];
  const jobsAt = lines.findIndex((l) => /^jobs:\s*(#.*)?$/.test(l));
  if (jobsAt < 0) return { lines, jobs };
  let job = null;
  let inSteps = false;
  let itemIndent = -1;
  let step = null;
  const endStep = (at) => {
    if (step) step.end = at;
    step = null;
  };
  for (let i = jobsAt + 1; i < lines.length; i++) {
    const text = lines[i];
    if (!isStructural(text)) {
      if (step) step.lines.push({ n: i + 1, text });
      continue;
    }
    const indent = indentOf(text);
    if (indent === 0) {
      endStep(i);
      break;
    }
    const header = /^ {2}([A-Za-z0-9_-]+):\s*(#.*)?$/.exec(text);
    if (header) {
      endStep(i);
      inSteps = false;
      job = { id: header[1], line: i + 1, keys: {}, steps: [] };
      jobs.push(job);
      continue;
    }
    if (!job) continue;
    if (indent === 4) {
      endStep(i);
      const kv = /^ {4}([A-Za-z0-9_-]+):(.*)$/.exec(text);
      inSteps = kv?.[1] === "steps";
      job.lastKey = kv?.[1];
      if (kv) job.keys[kv[1]] = { line: i + 1, ...splitValue(kv[2]) };
      continue;
    }
    if (!inSteps) {
      // A nested job key, e.g. `environment:\n      name: release`.
      const kv = /^ {6}([A-Za-z0-9_-]+):(.*)$/.exec(text);
      if (kv && job.lastKey)
        job.keys[`${job.lastKey}.${kv[1]}`] = { line: i + 1, ...splitValue(kv[2]) };
      continue;
    }
    if (itemIndent < 0 || indent < itemIndent) itemIndent = indent;
    if (indent === itemIndent && text.trimStart().startsWith("- ")) {
      endStep(i);
      step = { line: i + 1, lines: [], keys: {}, with: {}, end: lines.length };
      job.steps.push(step);
    }
    if (!step) continue;
    step.lines.push({ n: i + 1, text });
    const own = new RegExp(`^ {${itemIndent}}(?:- | {2})([A-Za-z0-9_-]+):(.*)$`).exec(text);
    if (own) {
      step.keys[own[1]] = { line: i + 1, ...splitValue(own[2]) };
      step.lastKey = own[1];
      continue;
    }
    const sub = new RegExp(`^ {${itemIndent + 4}}([A-Za-z0-9_-]+):(.*)$`).exec(text);
    if (sub && step.lastKey === "with") step.with[sub[1]] = { line: i + 1, ...splitValue(sub[2]) };
  }
  endStep(lines.length);
  return { lines, jobs };
}

const stepsOf = (wf) => wf.jobs.flatMap((job) => job.steps.map((step) => ({ job, step })));
const usesOf = (step) => step.keys.uses?.value ?? "";
const actionOf = (step) => usesOf(step).split("@")[0];

// ── Rules on one workflow file ──────────────────────────────────────────────

/** supply-01: every `uses:` names a full commit SHA and says which version it is. */
function pinnedUses(file, wf, report) {
  wf.lines.forEach((text, i) => {
    const m = /^\s*(?:-\s+)?(uses):(.*)$/i.exec(text);
    if (!m) return;
    if (m[1] !== "uses") report(file, i + 1, "pinned-uses", "write the key as `uses`");
    const { value, comment } = splitValue(m[2]);
    if (value.startsWith("./")) return; // an action from this repository
    if (/^docker:\/\/[^@\s]+@sha256:[0-9a-f]{64}$/.test(value)) return;
    const at = value.lastIndexOf("@");
    const ref = at < 0 ? "" : value.slice(at + 1);
    if (
      !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+(\/[^@\s]+)?$/.test(value.slice(0, at)) ||
      !SHA40.test(ref)
    ) {
      report(
        file,
        i + 1,
        "pinned-uses",
        `\`${value}\` is not pinned to a full 40-character commit SHA`,
      );
    } else if (!comment) {
      report(
        file,
        i + 1,
        "pinned-uses",
        `\`${value}\` needs a trailing \`# vX.Y.Z\` comment naming the pinned version`,
      );
    }
  });
}

/** supply-01: checkout leaves no token in .git/config for later steps to read. */
function checkoutCredentials(file, wf, report) {
  const seen = new Set(stepsOf(wf).map(({ step }) => step.keys.uses?.line));
  wf.lines.forEach((text, i) => {
    if (/^\s*(?:-\s+)?uses:\s*["']?actions\/checkout\b/.test(text) && !seen.has(i + 1)) {
      report(file, i + 1, "checkout-credentials", "this checkout is not a step the check can read");
    }
  });
  for (const { step } of stepsOf(wf)) {
    if (actionOf(step) !== "actions/checkout") continue;
    if (step.with["persist-credentials"]?.value !== "false") {
      report(
        file,
        step.line,
        "checkout-credentials",
        "actions/checkout needs `persist-credentials: false`",
      );
    }
  }
}

/** supply-01: sccache-action installs an exact sccache release, never "latest". */
function sccacheVersion(file, wf, report) {
  for (const { step } of stepsOf(wf)) {
    if (actionOf(step) !== "mozilla/sccache-action") continue;
    if (!/^v\d+\.\d+\.\d+$/.test(step.with.version?.value ?? "")) {
      report(file, step.line, "sccache-version", "mozilla/sccache-action needs `version: vX.Y.Z`");
    }
  }
}

/** Keys and actions that bring a compiled or installed tree in from the Actions cache. */
const RESTORES_CACHE =
  /mozilla\/sccache-action|RUSTC_WRAPPER|Swatinem\/rust-cache|cache-apt-pkgs-action|actions\/cache\b|^\s*cache(?:-[\w-]+)?:/;

/**
 * supply-01: the release workflow restores nothing from the Actions cache and
 * runs no compiler wrapper. ci.yml and e2e.yml hold a token that can write that
 * cache on main, and a restored object would be linked into a signed release.
 */
function releaseRestoresNothing(file, wf, report) {
  if (!isReleaseWorkflow(file)) return;
  wf.lines.forEach((text, i) => {
    if (isStructural(text) && RESTORES_CACHE.test(text)) {
      report(
        file,
        i + 1,
        "release-cache",
        "the release workflow must not restore from the Actions cache or set a RUSTC_WRAPPER",
      );
    }
  });
}

/** supply-01: the Rust toolchain is an exact version, not a channel that moves. */
function rustToolchain(file, wf, report, seen) {
  for (const { step } of stepsOf(wf)) {
    if (actionOf(step) !== "dtolnay/rust-toolchain") continue;
    const toolchain = step.with.toolchain?.value ?? "";
    if (!/^\d+\.\d+\.\d+$/.test(toolchain)) {
      report(file, step.line, "rust-toolchain", "dtolnay/rust-toolchain needs `toolchain: X.Y.Z`");
    } else {
      seen.toolchains.push({ file, line: step.line, toolchain });
    }
  }
}

/** supply-01: install-action installs an exact, checksummed tool version. */
function installActionVersion(file, wf, report) {
  for (const { step } of stepsOf(wf)) {
    if (actionOf(step) !== "taiki-e/install-action") continue;
    const tools = (step.with.tool?.value ?? "").split(/[\s,]+/).filter(Boolean);
    if (tools.length === 0 || !tools.every((t) => /@\d+\.\d+\.\d+$/.test(t))) {
      report(
        file,
        step.line,
        "install-action",
        "taiki-e/install-action needs `tool: <name>@X.Y.Z`",
      );
    }
    if (step.with.fallback?.value !== "none") {
      report(file, step.line, "install-action", "taiki-e/install-action needs `fallback: none`");
    }
  }
}

const SIGNING_KEY = /TAURI_SIGNING_PRIVATE_KEY/;
const SIGNING_SECRETS = new Set([
  "TAURI_SIGNING_PRIVATE_KEY",
  "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
]);
const SIGNING_JOB = "sign";
const SIGNING_STEP = "Sign every release file";
const RELEASE_ENVIRONMENT = "release";
const environmentOf = (job) =>
  job.keys["environment.name"]?.value ?? job.keys.environment?.value ?? "";

// `secrets.<name>` or `secrets['<name>']`. GitHub reads context and secret names
// in any case, and an expression may break across lines.
const secretRef = () => /\bsecrets\s*(?:\.\s*([A-Za-z_][\w-]*)|\[\s*(['"])([^'"\]]*)\2\s*\])/gi;
const lineAt = (text, index) => text.slice(0, index).split("\n").length;

/** The secrets a workflow text names, upper-cased, with the line of each reference. */
export function namedSecrets(text) {
  return [...text.matchAll(secretRef())].map((m) => ({
    name: (m[1] ?? m[3]).toUpperCase(),
    line: lineAt(text, m.index),
  }));
}

/**
 * Lines that reach secrets without a literal name: `toJSON(secrets)`, `secrets.*`,
 * `secrets[expression]`, `format('{0}', secrets)`, `secrets: inherit`. Any of
 * these can carry the signing key without naming it.
 */
export function wholesaleSecrets(text) {
  const named = text.replace(secretRef(), (m) => m.replaceAll(/[^\n]/g, " "));
  const at = new Set();
  for (const m of named.matchAll(/\bsecrets\s*\[|^\s*secrets:\s*inherit\b/gim)) {
    at.add(lineAt(named, m.index));
  }
  for (const expr of named.matchAll(/\$\{\{[\s\S]*?\}\}/g)) {
    const inner = expr[0].search(/\bsecrets\b/i);
    if (inner >= 0) at.add(lineAt(named, expr.index + inner));
  }
  for (const m of named.matchAll(/toJSON\s*\(\s*secrets\b/gi)) at.add(lineAt(named, m.index));
  // An `if:` is an expression with or without ${{ }}, on its line or below it.
  const lines = named.split("\n");
  lines.forEach((line, i) => {
    const m = /^(\s*)(?:-\s+)?if:(.*)$/i.exec(line);
    if (!m) return;
    const inValue = (text) => /\bsecrets\b/i.test(text.replace(/\s#.*$/, ""));
    if (inValue(m[2])) at.add(i + 1);
    const keyAt = line.length - line.trimStart().length;
    for (let j = i + 1; j < lines.length; j++) {
      if (
        lines[j].trim() !== "" &&
        indentOf(lines[j]) <= keyAt + (line.trimStart().startsWith("-") ? 2 : 0)
      )
        break;
      if (inValue(lines[j])) at.add(j + 1);
    }
  });
  return [...at].sort((x, y) => x - y);
}

/**
 * supply-01, supply-02: the updater signing key reaches one step of release.yml,
 * in the sign job, which runs in the `release` environment. The name may not
 * appear anywhere else in .github, comments included, and no workflow may hand
 * over every secret at once.
 */
function signingKeyInOneStep(file, wf, report) {
  const text = wf.lines.join("\n");
  for (const n of wholesaleSecrets(text)) {
    report(
      file,
      n,
      "signing-key",
      "no workflow may reach secrets without naming one (toJSON(secrets), secrets.*, secrets[expression], secrets: inherit)",
    );
  }
  const inTheSigningStep = (n) => {
    const owner = stepsOf(wf).find(({ step }) => n >= step.line && n <= step.end);
    return (
      isReleaseWorkflow(file) &&
      owner !== undefined &&
      owner.job.id === SIGNING_JOB &&
      owner.step.keys.name?.value === SIGNING_STEP &&
      environmentOf(owner.job) === RELEASE_ENVIRONMENT
    );
  };
  const mentions = wf.lines.flatMap((line, i) => (SIGNING_KEY.test(line) ? [i + 1] : []));
  const references = namedSecrets(text)
    .filter((r) => SIGNING_SECRETS.has(r.name))
    .map((r) => r.line);
  for (const n of new Set([...mentions, ...references])) {
    if (inTheSigningStep(n)) continue;
    report(
      file,
      n,
      "signing-key",
      "the updater signing secrets may appear only in the signing step of release.yml's sign job, with `environment: release`",
    );
  }
}

const YAML_QUOTED = String.raw`"(?:[^"\\]|\\.)*"|'(?:[^']|'')*'`;
const YAML_KEY = new RegExp(String.raw`^(${YAML_QUOTED}|[^\s'"\[{&*!|>#][^#]*?)\s*:(?:\s+|$)`);
const YAML_QUOTED_LINE = new RegExp(String.raw`^(?:${YAML_QUOTED})\s*(?:#.*)?$`);
const YAML_QUOTED_ANY = new RegExp(YAML_QUOTED, "g");
const YAML_TAGS = /^(?:!\S*\s+)+/;

// Line breaks to YAML that split(/\r?\n/) does not treat as breaks.
const YAML_OTHER_BREAK = /\r(?!\n|$)|[\u0085\u2028\u2029]/;

/**
 * What the line-based reader above cannot follow: anchors, aliases and merge
 * keys let one step's `env:` come from somewhere else, and a flow mapping, a
 * quoted or complex key, a tag or a double-quoted escape make GitHub read a name
 * the scan does not see. Returns `[{ line, message }]`. The same characters
 * count at the start of a node (or inside a flow collection), never in plain
 * text, single-quoted strings or block scalars. Anything that would need more
 * than one line to tell apart (a quoted string or flow collection left open, a
 * block scalar header without its key, an item whose content starts on the next
 * line) is a finding too.
 */
export function yamlIndirection(lines) {
  const found = [];
  const add = (i, message) => found.push({ line: i + 1, message });
  const noEscape = (i, quoted) => {
    if (quoted.startsWith('"') && quoted.includes("\\")) {
      add(i, "double-quoted escapes are not allowed in a workflow; GitHub reads the decoded text");
    }
  };
  let blockParent = null;
  lines.forEach((raw, i) => {
    if (raw.trim() === "") return;
    if (YAML_OTHER_BREAK.test(raw)) add(i, "a line break other than LF or CRLF is not allowed");
    let col = indentOf(raw);
    if (blockParent !== null) {
      if (col > blockParent) return;
      blockParent = null;
    }
    let rest = raw.trimStart();
    let parent = null;
    for (let m; (m = /^[-?:](?:\s+|$)/.exec(rest)); rest = rest.slice(m[0].length)) {
      if (m[0].trim() !== "-") add(i, "complex keys (`? `, `: `) are not allowed in a workflow");
      parent = col;
      col += m[0].length;
    }
    if (rest === "" || rest.startsWith("#")) {
      if (parent !== null) add(i, "an item's content must start on the `- ` line");
      return;
    }
    const tag = () => {
      if (YAML_TAGS.test(rest)) add(i, "YAML tags are not allowed in a workflow");
      rest = rest.replace(YAML_TAGS, "");
    };
    tag();
    const key = YAML_KEY.exec(rest);
    if (key) {
      if (/<<["']?$/.test(key[1])) add(i, "merge keys (`<<:`) are not allowed in a workflow");
      if (/^["']/.test(key[1])) add(i, "quoted keys are not allowed in a workflow");
      if (/\s:/.test(key[0])) add(i, "a key's colon must follow it directly");
      parent = col;
      rest = rest.slice(key[0].length);
      tag();
    }
    if (/^[&*]/.test(rest)) {
      add(i, "YAML anchors and aliases are not allowed in a workflow");
    } else if (/^["']/.test(rest)) {
      if (!YAML_QUOTED_LINE.test(rest)) add(i, "a quoted string must close on its own line");
      noEscape(i, rest);
    } else if (/^[[{]/.test(rest)) {
      for (const q of rest.match(YAML_QUOTED_ANY) ?? []) noEscape(i, q);
      const bare = rest.replace(YAML_QUOTED_ANY, '""').replace(/\s#.*$/, "");
      if (bare.includes("{")) add(i, "flow mappings are not allowed in a workflow");
      const depth = bare.replace(/[^[{]/g, "").length - bare.replace(/[^\]}]/g, "").length;
      if (depth !== 0) add(i, "a flow collection must close on its own line");
      if (/(?:^|[[{,:\s])[&*]\S|<<\s*:/.test(bare)) {
        add(i, "YAML anchors, aliases and merge keys are not allowed in a workflow");
      }
    } else if (/^[|>][-+0-9]*\s*(?:#.*)?$/.test(rest)) {
      if (parent === null) add(i, "a block scalar header must follow its key on the same line");
      else blockParent = parent;
    }
  });
  return found;
}

/** supply-01, supply-02: every rule here sees what GitHub runs, so nothing is indirect. */
function noYamlIndirection(file, wf, report) {
  for (const f of yamlIndirection(wf.lines)) report(file, f.line, "yaml-indirection", f.message);
}

// cargo subcommands that resolve dependencies; `fmt` and `audit` do not.
const CARGO_RESOLVES =
  /\bcargo\s+(?:\+\S+\s+)?(build|check|clippy|test|nextest|run|install|doc|bench|fetch|tree|metadata)\b/;
const CARGO_INSTALL = /\bcargo\s+(?:\+\S+\s+)?install\b/;
const EXACT_VERSION = /\s--version[\s=]+=?\d+\.\d+\.\d+(?=\s|$)|@\d+\.\d+\.\d+(?=\s|$)/;
const CARGO_BUILD = /\bcargo\s+(?:\+\S+\s+)?build\b/;
const LOCKED = /--locked\b/;
const TAURI_BUILD = /\btauri\s+build\b/;
// `tauri build` hands everything after `--` to cargo.
const TAURI_LOCKED = /\s--\s(?:.*\s)?--locked(?=\s|$)/;
// A project install on the developer's machine; `npm i -g <tool>` is a hint to the reader.
const NPM_INSTALL = /\bnpm\s+(?:ci|install|i)\b(?!\s+(?:-g|--global)\b)/;

/** supply-07: CI and release builds compile exactly what Cargo.lock pins. */
function cargoLockedInWorkflow(file, wf, report) {
  wf.lines.forEach((text, i) => {
    if (!isStructural(text)) return;
    if (CARGO_RESOLVES.test(text) && !LOCKED.test(text)) {
      report(file, i + 1, "cargo-locked", "this cargo command needs --locked");
    }
    if (TAURI_BUILD.test(text) && !TAURI_LOCKED.test(text)) {
      report(file, i + 1, "cargo-locked", "tauri build needs `-- --locked` (passed on to cargo)");
    }
    if (CARGO_INSTALL.test(text) && !EXACT_VERSION.test(text)) {
      report(file, i + 1, "cargo-install", "cargo install needs an exact `--version X.Y.Z`");
    }
  });
}

/**
 * supply-07: the same in the scripts a build runs (comments are not commands).
 * supply-06: no npm install runs dependencies' install scripts.
 */
export function checkBuildScript(file, text) {
  const findings = [];
  text.split(/\r?\n/).forEach((line, i) => {
    if (/^\s*(#|\/\/|\*|\/\*)/.test(line)) return;
    if (CARGO_BUILD.test(line) && !LOCKED.test(line)) {
      findings.push({
        file,
        line: i + 1,
        rule: "cargo-locked",
        message: "cargo build needs --locked",
      });
    }
    if (TAURI_BUILD.test(line) && !TAURI_LOCKED.test(line)) {
      findings.push({
        file,
        line: i + 1,
        rule: "cargo-locked",
        message: "tauri build needs `-- --locked` (passed on to cargo)",
      });
    }
    if (NPM_INSTALL.test(line) && !/--ignore-scripts\b/.test(line)) {
      findings.push({
        file,
        line: i + 1,
        rule: "npm-scripts",
        message: "npm install and npm ci need --ignore-scripts",
      });
    }
  });
  return findings;
}

const stepText = (step) =>
  step.lines
    .filter((l) => isStructural(l.text))
    .map((l) => l.text.trim())
    .join("\n");

/**
 * The jobs of release.yml and what each may hold. No other job, and none of them
 * may run conditionally or carry on after a failure.
 */
export const RELEASE_JOBS = {
  prepare: { permissions: { contents: "write", checks: "read" } },
  build: { needs: "prepare", permissions: { contents: "read" } },
  sign: {
    needs: "[prepare, build]",
    environment: RELEASE_ENVIRONMENT,
    permissions: { contents: "read" },
  },
  publish: {
    needs: "[prepare, build, sign]",
    permissions: { contents: "write", "id-token": "write", attestations: "write" },
  },
};
// The sign job runs GitHub's own actions and these steps, nothing else.
const SIGN_JOB_ACTION = /^actions\/(checkout|download-artifact|upload-artifact)$/;

/**
 * Steps of release.yml that must stay exactly as written, in this order within
 * their job: what they check, on which values, with no way to carry on past a
 * failure. Comment lines do not count. A real change to one changes this list in
 * the same commit, in front of a reviewer.
 */
export const RELEASE_GUARDS = [
  {
    finding: "supply-08",
    job: "prepare",
    text: String.raw`- name: Refuse a commit that is not on main with green CI
env:
GH_TOKEN: ${"$"}{{ secrets.GITHUB_TOKEN }}
GH_REPO: ${"$"}{{ github.repository }}
SHA: ${"$"}{{ github.sha }}
run: |
set -euo pipefail
status="$(gh api "repos/$GH_REPO/compare/main...$SHA" --jq .status)"
gh api "repos/$GH_REPO/commits/$SHA/check-runs?per_page=100" \
--jq '.check_runs[] | select(.app.slug == "github-actions") | [.name, (.conclusion // "")] | @tsv' > check-runs.tsv
node scripts/release-gate.mjs "$status" check-runs.tsv`,
    before: /\bgh release create\b/,
  },
  {
    finding: "supply-03",
    job: "prepare",
    text: `- name: Create draft release
id: draft
env:
GH_TOKEN: \${{ secrets.GITHUB_TOKEN }}
VERSION: \${{ steps.resolve.outputs.version }}
PRERELEASE: \${{ steps.resolve.outputs.prerelease }}
SHA: \${{ github.sha }}
run: |
set -euo pipefail
TAG="v$VERSION"
PRE_FLAG=""
[ "$PRERELEASE" = "true" ] && PRE_FLAG="--prerelease"
if ! gh release view "$TAG" >/dev/null 2>&1; then
gh release create "$TAG" \\
--target "$SHA" \\
--draft \\
--title "Namehold $TAG" \\
--generate-notes \\
$PRE_FLAG
fi
[ "$(gh release view "$TAG" --json isDraft --jq .isDraft)" = "true" ] || { echo "::error::$TAG is already published"; exit 1; }
RID="$(gh release view "$TAG" --json databaseId --jq .databaseId)"
echo "release_id=$RID" >> "$GITHUB_OUTPUT"`,
  },
  {
    finding: "supply-01",
    job: "build",
    text: `- name: Build bundles
shell: bash
run: pnpm tauri build --no-sign \${{ matrix.args }} -- --locked`,
  },
  {
    finding: "supply-01",
    job: "build",
    text: `- name: Collect the release files
run: node scripts/release-files.mjs stage src-tauri/target release-files`,
  },
  {
    finding: "supply-01",
    job: "sign",
    text: `- name: Install the Tauri CLI
working-directory: scripts/release-signer
run: npm ci --ignore-scripts --no-audit --no-fund`,
  },
  {
    finding: "supply-01",
    job: "sign",
    text: `- name: ${SIGNING_STEP}
env:
TAURI_SIGNING_PRIVATE_KEY: \${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
TAURI_SIGNING_PRIVATE_KEY_PASSWORD: \${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
VERSION: \${{ needs.prepare.outputs.version }}
run: |
set -euo pipefail
signer="$PWD/scripts/release-signer/node_modules/.bin/tauri"
cd release-files
sha256sum -- * > SHA256SUMS
for file in *; do
"$signer" signer sign --app-version "$VERSION" "./$file"
done`,
  },
  {
    finding: "supply-03",
    job: "publish",
    text: `- name: Check every file and signature, write latest.json
env:
GH_REPO: \${{ github.repository }}
VERSION: \${{ needs.prepare.outputs.version }}
RELEASE_NOTES: \${{ needs.prepare.outputs.release_notes }}
run: node scripts/release-files.mjs check release-files src-tauri/tauri.conf.json "$VERSION" "$GH_REPO"`,
    before: /\bgh release upload\b/,
  },
  {
    finding: "supply-03",
    job: "publish",
    text: String.raw`- name: Compare the draft with the checked files
env:
GH_TOKEN: ${"$"}{{ secrets.GITHUB_TOKEN }}
GH_REPO: ${"$"}{{ github.repository }}
RELEASE_ID: ${"$"}{{ needs.prepare.outputs.release_id }}
run: |
set -euo pipefail
gh api --paginate "repos/$GH_REPO/releases/$RELEASE_ID/assets?per_page=100" \
--jq '.[] | [.name, (.digest // ""), .state] | @tsv' > release-assets.tsv
node scripts/release-files.mjs compare release-files release-assets.tsv`,
    before: /\bdraft=false\b/,
  },
];

/** supply-01, supply-03, supply-08: release.yml keeps its jobs and its checks. */
function releaseShape(file, wf, report) {
  if (!isReleaseWorkflow(file)) return;
  const finding = (line, message) => report(file, line, "release-shape", message);

  const jobsAt = wf.lines.findIndex((l) => /^jobs:/.test(l));
  const top = wf.lines
    .slice(0, jobsAt)
    .join("\n")
    .match(/^permissions:.*(?:\n(?: .*|#.*|))*/m);
  const topLines = (top?.[0] ?? "").split("\n").filter((l) => /^ *[a-z-]+:/.test(l));
  if (topLines.map((l) => l.trim()).join() !== "permissions:,contents: read") {
    finding(1, "release.yml's top-level permissions must be exactly `contents: read`");
  }

  for (const job of wf.jobs) {
    const want = RELEASE_JOBS[job.id];
    if (!want) {
      finding(job.line, `job \`${job.id}\` is not one release.yml may have`);
      continue;
    }
    for (const key of ["if", "continue-on-error", "env", "uses", "secrets"]) {
      if (job.keys[key]) finding(job.keys[key].line, `job \`${job.id}\` may not set \`${key}\``);
    }
    const permissions = Object.entries(job.keys)
      .filter(([k]) => k.startsWith("permissions."))
      .map(([k, v]) => `${k.slice("permissions.".length)}=${v.value}`)
      .sort()
      .join();
    const allowed = Object.entries(want.permissions)
      .map(([k, v]) => `${k}=${v}`)
      .sort()
      .join();
    if (permissions !== allowed) {
      finding(job.line, `job \`${job.id}\` must have exactly these permissions: ${allowed}`);
    }
    if ((job.keys.needs?.value ?? "") !== (want.needs ?? "")) {
      finding(job.line, `job \`${job.id}\` must need ${want.needs ?? "nothing"}`);
    }
    if (environmentOf(job) !== (want.environment ?? "")) {
      finding(job.line, `job \`${job.id}\` must run in environment "${want.environment ?? ""}"`);
    }
    for (const step of job.steps) {
      if (step.keys["continue-on-error"]) {
        finding(
          step.keys["continue-on-error"].line,
          "no step of release.yml may carry on after a failure",
        );
      }
    }
  }

  const texts = (job) => job.steps.map(stepText);
  let previous = { job: null, at: -1 };
  for (const guard of RELEASE_GUARDS) {
    const job = wf.jobs.find((j) => j.id === guard.job);
    const at = job ? texts(job).indexOf(guard.text) : -1;
    const name = guard.text.split("\n")[0].replace("- name: ", "");
    if (at < 0) {
      finding(
        job?.line ?? 1,
        `${guard.finding}: job \`${guard.job}\` must keep "${name}" as written`,
      );
    } else {
      if (previous.job === guard.job && at <= previous.at) {
        finding(job.steps[at].line, `${guard.finding}: "${name}" is out of order`);
      }
      previous = { job: guard.job, at };
    }
    if (!guard.before) continue;
    // Wherever the guarded command runs, the guard ran earlier in that job.
    for (const other of wf.jobs) {
      const anchor = other.steps.findIndex((s) =>
        s.lines.some((l) => isStructural(l.text) && guard.before.test(l.text)),
      );
      if (anchor >= 0 && (other !== job || at < 0 || at > anchor)) {
        finding(other.steps[anchor].line, `${guard.finding}: "${name}" must run before this step`);
      }
    }
  }

  const sign = wf.jobs.find((j) => j.id === SIGNING_JOB);
  const signGuards = RELEASE_GUARDS.filter((g) => g.job === SIGNING_JOB).map((g) => g.text);
  for (const step of sign?.steps ?? []) {
    if (signGuards.includes(stepText(step))) continue;
    if (!SIGN_JOB_ACTION.test(actionOf(step)) || step.keys.run) {
      finding(
        step.line,
        "the sign job runs only GitHub's artifact and checkout actions and its own steps",
      );
    }
  }
}

/** A workflow whose jobs the reader cannot find would pass every rule above. */
function hasReadableJobs(file, wf, report) {
  if (file.replaceAll(sep, "/").includes("/workflows/") && wf.jobs.length === 0) {
    report(file, 1, "yaml-indirection", "the check cannot read this workflow's jobs");
  }
}

const isReleaseWorkflow = (file) => file.replaceAll(sep, "/") === ".github/workflows/release.yml";

export const WORKFLOW_RULES = [
  pinnedUses,
  checkoutCredentials,
  sccacheVersion,
  releaseRestoresNothing,
  rustToolchain,
  installActionVersion,
  signingKeyInOneStep,
  cargoLockedInWorkflow,
  releaseShape,
  noYamlIndirection,
  hasReadableJobs,
];

/** Check workflow and action files given as [{ file, text }] (paths relative to the repo). */
export function checkWorkflows(files) {
  const findings = [];
  const report = (file, line, rule, message) => findings.push({ file, line, rule, message });
  const seen = { toolchains: [] };
  for (const { file, text } of files) {
    const wf = readWorkflow(text);
    for (const rule of WORKFLOW_RULES) rule(file, wf, report, seen);
  }
  const versions = new Set(seen.toolchains.map((t) => t.toolchain));
  if (versions.size > 1) {
    for (const t of seen.toolchains) {
      report(
        t.file,
        t.line,
        "rust-toolchain",
        `toolchains differ across workflows: ${[...versions].join(", ")}`,
      );
    }
  }
  return findings;
}

// ── The repository ──────────────────────────────────────────────────────────

function listYaml(dir, root) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    if (e.isDirectory()) return listYaml(p, root);
    return /\.ya?ml$/.test(e.name) ? [relative(root, p)] : [];
  });
}

function listFiles(dir, root, pattern, recursive) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    if (e.isDirectory())
      return recursive && e.name !== "node_modules" ? listFiles(p, root, pattern, true) : [];
    return pattern.test(e.name) ? [relative(root, p)] : [];
  });
}

const SCRIPT = /\.(sh|mjs|cjs|js|ts)$/;
// The checker and its tests quote the commands they look for.
const SELF = new Set([
  "scripts/check-supply-chain.mjs",
  "scripts/check-supply-chain.node-test.mjs",
]);

/** Scripts a build or a developer runs: scripts/, src-tauri/*.{sh,mjs,...}, justfile. */
export function buildScripts(root) {
  return listFiles(join(root, "scripts"), root, SCRIPT, true)
    .concat(listFiles(join(root, "src-tauri"), root, SCRIPT, false))
    .concat(existsSync(join(root, "justfile")) ? ["justfile"] : [])
    .filter((file) => !SELF.has(file.replaceAll(sep, "/")));
}

export const VECTORS_LOCKFILE = "src-tauri/tests/vectors/package-lock.json";
export const VECTORS_GENERATOR = "src-tauri/tests/vectors/gen_hsd_vectors.js";
const VECTORS_COMMAND = "npm ci --ignore-scripts && NODE_BACKEND=js node gen_hsd_vectors.js";
export const VECTORS_PACKAGE = "src-tauri/tests/vectors/package.json";
export const SHAKEDEX_SCRIPT = "scripts/shakedex-cli-sell.sh";
export const SIGNER_PACKAGE = "scripts/release-signer/package.json";
export const SIGNER_LOCKFILE = "scripts/release-signer/package-lock.json";
export const TOOLCHAIN_FILE = "rust-toolchain.toml";

/** supply-06, supply-09: what git keeps and leaves out, and the documented generator command. */
export function checkRepoHygiene(root) {
  const findings = [];
  const git = (...args) => spawnSync("git", args, { cwd: root, stdio: "ignore" }).status === 0;
  if (!git("ls-files", "--error-unmatch", VECTORS_LOCKFILE)) {
    findings.push({
      file: VECTORS_LOCKFILE,
      line: 1,
      rule: "npm-scripts",
      message: "the vector generator's lockfile must be tracked",
    });
  }
  const generator = join(root, VECTORS_GENERATOR);
  if (!existsSync(generator) || !readFileSync(generator, "utf8").includes(VECTORS_COMMAND)) {
    findings.push({
      file: VECTORS_GENERATOR,
      line: 1,
      rule: "npm-scripts",
      message: `the generator must document \`${VECTORS_COMMAND}\``,
    });
  }
  // NODE_BACKEND=js keeps bcrypto's native addon out of the generator and the CLI.
  const read = (rel) => (existsSync(join(root, rel)) ? readFileSync(join(root, rel), "utf8") : "");
  for (const [file, re] of [
    [VECTORS_PACKAGE, /"gen":\s*"NODE_BACKEND=js node /],
    [SHAKEDEX_SCRIPT, /^[^#\n]*NODE_BACKEND=js\b/m],
  ]) {
    if (!re.test(read(file))) {
      findings.push({
        file,
        line: 1,
        rule: "npm-scripts",
        message: "this must run with NODE_BACKEND=js",
      });
    }
  }
  // The sign job installs the Tauri CLI from this lockfile: the same version as
  // the build, one that pnpm-lock.yaml also pins, and nothing else.
  const signer = /"@tauri-apps\/cli":\s*"(\d+\.\d+\.\d+)"/.exec(read(SIGNER_PACKAGE))?.[1];
  if (
    !git("ls-files", "--error-unmatch", SIGNER_LOCKFILE) ||
    !signer ||
    !read(SIGNER_LOCKFILE).includes(`"version": "${signer}"`) ||
    !read("pnpm-lock.yaml").includes(`'@tauri-apps/cli@${signer}':`)
  ) {
    findings.push({
      file: SIGNER_LOCKFILE,
      line: 1,
      rule: "npm-scripts",
      message:
        "the signer's lockfile must be tracked and pin the Tauri CLI version pnpm-lock.yaml pins",
    });
  }
  if (!/^channel\s*=\s*"\d+\.\d+\.\d+"/m.test(read(TOOLCHAIN_FILE))) {
    findings.push({
      file: TOOLCHAIN_FILE,
      line: 1,
      rule: "rust-toolchain",
      message: 'this file must set `channel = "X.Y.Z"`',
    });
  }
  // A HAR capture of a logged-in session holds its cookies. check-ignore -q
  // passes if any one path is ignored, so each is asked on its own.
  for (const path of ["capture.har", "sub/dir/capture.har"]) {
    if (!git("check-ignore", "-q", "--no-index", path)) {
      findings.push({
        file: ".gitignore",
        line: 1,
        rule: "ignored-files",
        message: `${path} is not ignored; HAR captures hold session cookies`,
      });
    }
  }
  return findings;
}

/** Every finding for the repository at `root`. */
export function checkRepo(root) {
  const workflowFiles = listYaml(join(root, ".github", "workflows"), root)
    .concat(listYaml(join(root, ".github", "actions"), root))
    .map((file) => ({ file, text: readFileSync(join(root, file), "utf8") }));
  const scripts = buildScripts(root).flatMap((file) =>
    checkBuildScript(file, readFileSync(join(root, file), "utf8")),
  );
  const channel = existsSync(join(root, TOOLCHAIN_FILE))
    ? /channel\s*=\s*"([^"]+)"/.exec(readFileSync(join(root, TOOLCHAIN_FILE), "utf8"))?.[1]
    : undefined;
  const toolchains = workflowFiles.flatMap(({ file, text }) =>
    [...text.matchAll(/^\s*toolchain:\s*(\S+)/gm)]
      .filter((m) => channel !== undefined && m[1] !== channel)
      .map((m) => ({
        file,
        line: lineAt(text, m.index),
        rule: "rust-toolchain",
        message: `toolchain ${m[1]} differs from ${TOOLCHAIN_FILE}'s ${channel}`,
      })),
  );
  return checkWorkflows(workflowFiles).concat(scripts, toolchains, checkRepoHygiene(root));
}

export const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const findings = checkRepo(repoRoot);
  for (const f of findings) console.error(`${f.file}:${f.line}: [${f.rule}] ${f.message}`);
  if (findings.length > 0) {
    console.error(`check-supply-chain: ${findings.length} finding(s)`);
    process.exit(1);
  }
  console.log("check-supply-chain: no findings");
}
