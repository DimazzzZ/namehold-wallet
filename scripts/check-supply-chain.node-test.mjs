// Tests for check-supply-chain.mjs (node --test). Each rule is fed a small
// workflow that breaks it and one that keeps it; the last test runs every rule
// on this repository.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  buildScripts,
  checkBuildScript,
  checkRepo,
  checkRepoHygiene,
  checkWorkflows,
  readWorkflow,
  RELEASE_GUARDS,
  RELEASE_JOBS,
  repoRoot,
  SHAKEDEX_SCRIPT,
  SIGNER_LOCKFILE,
  SIGNER_PACKAGE,
  TOOLCHAIN_FILE,
  VECTORS_GENERATOR,
  VECTORS_LOCKFILE,
  VECTORS_PACKAGE,
  wholesaleSecrets,
  yamlIndirection,
} from "./check-supply-chain.mjs";

const SHA = "3d3c42e5aac5ba805825da76410c181273ba90b1";
const CHECKOUT = `actions/checkout@${SHA} # v7.0.1`;

/** A workflow with one job whose steps are the given YAML lines (indented 6). */
const workflow = (steps, jobKeys = "", jobId = "build") =>
  [
    "name: T",
    "on: push",
    "jobs:",
    `  ${jobId}:`,
    "    runs-on: ubuntu-latest",
    ...(jobKeys ? [jobKeys] : []),
    "    steps:",
    ...steps,
    "",
  ].join("\n");

const run = (text, file = ".github/workflows/t.yml") => checkWorkflows([{ file, text }]);
const rules = (findings) => findings.map((f) => f.rule);

const safeCheckout = [
  `      - uses: ${CHECKOUT}`,
  "        with:",
  "          persist-credentials: false",
];

test("the reader finds jobs, steps, their keys and their with: inputs", () => {
  const wf = readWorkflow(
    workflow(
      [
        ...safeCheckout,
        "      - name: Build",
        "        run: |",
        "          - not a step",
        "          echo hi",
      ],
      "    environment:\n      name: release",
    ),
  );
  assert.equal(wf.jobs.length, 1);
  const [job] = wf.jobs;
  assert.equal(job.keys["environment.name"].value, "release");
  assert.equal(job.steps.length, 2);
  assert.equal(job.steps[0].keys.uses.value, `actions/checkout@${SHA}`);
  assert.equal(job.steps[0].keys.uses.comment, "v7.0.1");
  assert.equal(job.steps[0].with["persist-credentials"].value, "false");
  assert.equal(job.steps[1].keys.name.value, "Build");
});

const NOT_A_SHA = /not pinned to a full 40-character commit SHA/;

test("a tag, a branch or a short SHA in uses: is refused, with or without a comment", () => {
  for (const ref of [
    "actions/checkout@v7",
    "actions/checkout@v7 # v7",
    "some-org/some-action@main # main",
    "actions/checkout@3d3c42e # v7.0.1",
    "actions/checkout # v7",
  ]) {
    const findings = run(
      workflow([`      - uses: ${ref}`, "        with:", "          persist-credentials: false"]),
    );
    assert.deepEqual(rules(findings), ["pinned-uses"], ref);
    assert.match(findings[0].message, NOT_A_SHA, ref);
  }
});

test("the tag the release once used for tauri-action is refused", () => {
  const findings = run(
    workflow([
      "      - name: Build + upload bundles",
      "        uses: tauri-apps/tauri-action@v1",
      "        with:",
      "          args: -- --locked",
    ]),
  );
  assert.deepEqual(rules(findings), ["pinned-uses"]);
  assert.match(findings[0].message, NOT_A_SHA);
});

test("a reusable workflow called by tag is refused", () => {
  const text = [
    "on: push",
    "jobs:",
    "  call:",
    "    uses: o/r/.github/workflows/x.yml@v1 # v1",
    "",
  ].join("\n");
  const findings = run(text);
  assert.deepEqual(rules(findings), ["pinned-uses"]);
  assert.match(findings[0].message, NOT_A_SHA);
});

test("a full SHA needs the version comment", () => {
  const findings = run(
    workflow([
      `      - uses: actions/checkout@${SHA}`,
      "        with:",
      "          persist-credentials: false",
    ]),
  );
  assert.deepEqual(rules(findings), ["pinned-uses"]);
  assert.match(findings[0].message, /comment/);
});

test("a pinned action, a local action and a docker digest pass", () => {
  const digest = "a".repeat(64);
  const findings = run(
    workflow([
      ...safeCheckout,
      `      - uses: "pnpm/action-setup@${SHA}" # v6.0.10`,
      "      - uses: ./.github/actions/local",
      `      - uses: docker://alpine@sha256:${digest}`,
    ]),
  );
  assert.deepEqual(findings, []);
});

test("a checkout that keeps its token in .git/config is refused", () => {
  assert.deepEqual(rules(run(workflow([`      - uses: ${CHECKOUT}`]))), ["checkout-credentials"]);
  const kept = workflow([
    `      - uses: ${CHECKOUT}`,
    "        with:",
    "          persist-credentials: true",
  ]);
  assert.deepEqual(rules(run(kept)), ["checkout-credentials"]);
  assert.deepEqual(run(workflow(safeCheckout)), []);
});

test("sccache-action without an exact version is refused", () => {
  const step = `      - uses: mozilla/sccache-action@${SHA} # v0.0.11`;
  assert.deepEqual(rules(run(workflow([step]))), ["sccache-version"]);
  assert.deepEqual(rules(run(workflow([step, "        with:", "          version: latest"]))), [
    "sccache-version",
  ]);
  assert.deepEqual(run(workflow([step, "        with:", "          version: v0.18.0"])), []);
});

test("the release workflow restores nothing from the Actions cache and sets no wrapper", () => {
  const release = ".github/workflows/release.yml";
  const cacheFindings = (text, file = release) =>
    rules(run(text, file)).filter((r) => r === "release-cache");
  const wrapper = workflow(
    ["      - run: cargo build --locked"],
    "    env:\n      RUSTC_WRAPPER: sccache",
  );
  assert.deepEqual(cacheFindings(wrapper), ["release-cache"]);
  for (const action of [
    "mozilla/sccache-action",
    "Swatinem/rust-cache",
    "awalsh128/cache-apt-pkgs-action",
    "actions/cache",
    "actions/cache/restore",
  ]) {
    const step = workflow([`      - uses: ${action}@${SHA} # v1`]);
    assert.deepEqual(cacheFindings(step), ["release-cache"], action);
  }
  const nodeCache = workflow([
    `      - uses: actions/setup-node@${SHA} # v7`,
    "        with:",
    "          cache: pnpm",
  ]);
  assert.deepEqual(cacheFindings(nodeCache), ["release-cache"]);
  // Elsewhere these are allowed, and a comment that names one is not a use.
  assert.deepEqual(cacheFindings(wrapper, ".github/workflows/ci.yml"), []);
  const comment = workflow(["      # No RUSTC_WRAPPER here.", "      - run: cargo build --locked"]);
  assert.deepEqual(cacheFindings(comment), []);
});

test("a toolchain channel is refused and exact toolchains must agree", () => {
  const step = (toolchain) => [
    `      - uses: dtolnay/rust-toolchain@${SHA} # stable`,
    "        with:",
    `          toolchain: ${toolchain}`,
  ];
  assert.deepEqual(rules(run(workflow(step("stable")))), ["rust-toolchain"]);
  assert.deepEqual(rules(run(workflow([`      - uses: dtolnay/rust-toolchain@${SHA} # stable`]))), [
    "rust-toolchain",
  ]);
  assert.deepEqual(run(workflow(step("1.99.1"))), []);
  const two = checkWorkflows([
    { file: "a.yml", text: workflow(step("1.99.1")) },
    { file: "b.yml", text: workflow(step("1.98.0")) },
  ]);
  assert.deepEqual(rules(two), ["rust-toolchain", "rust-toolchain"]);
});

test("install-action needs an exact tool version and no fallback", () => {
  const step = `      - uses: taiki-e/install-action@${SHA} # v2.87.29`;
  const floating = run(
    workflow([step, "        with:", "          tool: nextest", "          fallback: none"]),
  );
  assert.deepEqual(rules(floating), ["install-action"]);
  const fallback = run(workflow([step, "        with:", "          tool: cargo-nextest@0.9.148"]));
  assert.deepEqual(rules(fallback), ["install-action"]);
  const exact = run(
    workflow([
      step,
      "        with:",
      "          tool: cargo-nextest@0.9.148",
      "          fallback: none",
    ]),
  );
  assert.deepEqual(exact, []);
});

const RELEASE = ".github/workflows/release.yml";
const KEY_ENV = [
  "        env:",
  "          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}",
  "          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
  "          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}",
];
const signingStep = ["      - name: Sign every release file", ...KEY_ENV, "        run: sign"];
const signingJob = (steps, keys = "    environment: release", id = "sign") =>
  workflow(steps, keys, id);
const signingFindings = (text, file = RELEASE) =>
  run(text, file).filter((f) => f.rule === "signing-key");

test("the signing key in the signing step of the sign job, in the release environment, passes", () => {
  assert.deepEqual(signingFindings(signingJob([...safeCheckout, ...signingStep])), []);
  const nested = signingJob(signingStep, "    environment:\n      name: release");
  assert.deepEqual(signingFindings(nested), []);
});

test("the signing key in a job outside the release environment is refused", () => {
  assert.equal(signingFindings(signingJob(signingStep, "")).length, 2);
  assert.equal(signingFindings(signingJob(signingStep, "    environment: staging")).length, 2);
});

test("the build job holds no signing secret, even in a step named like the signing step", () => {
  const inBuild = signingJob(signingStep, "    environment: release", "build");
  assert.equal(signingFindings(inBuild).length, 2);
});

test("the signing key in any other step, job env or workflow is refused", () => {
  const echo = ["      - name: Leak", "        run: echo done", ...KEY_ENV];
  assert.equal(signingFindings(signingJob(echo)).length, 2);
  const jobEnv = signingJob(
    ["      - name: Sign every release file", "        run: sign"],
    "    environment: release\n    env:\n      TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
  );
  assert.equal(signingFindings(jobEnv).length, 1);
  assert.equal(signingFindings(signingJob(signingStep), ".github/workflows/ci.yml").length, 2);
  const after = signingJob([
    ...signingStep,
    "      - name: After",
    "        run: echo done",
    ...KEY_ENV,
  ]);
  const late = signingFindings(after);
  assert.equal(late.length, 2);
  assert.ok(late.every((f) => f.line > 8 + KEY_ENV.length));
  const comment = workflow(["      # needs TAURI_SIGNING_PRIVATE_KEY", "      - run: true"]);
  assert.deepEqual(rules(run(comment, ".github/workflows/e2e.yml")), ["signing-key"]);
});

test("a workflow that hands over every secret is refused", () => {
  for (const line of [
    "        run: echo '${{ toJSON(secrets) }}'",
    "        run: echo '${{ secrets[matrix.name] }}'",
  ]) {
    assert.deepEqual(rules(run(workflow(["      - name: Dump", line]))), ["signing-key"], line);
  }
  const inherit = [
    "on: push",
    "jobs:",
    "  call:",
    `    uses: o/r/.github/workflows/x.yml@${SHA} # v1`,
    "    secrets: inherit",
    "",
  ];
  assert.deepEqual(rules(run(inherit.join("\n"))), ["signing-key"]);
});

test("a signing secret named in any case or form outside the signing step is refused", () => {
  for (const ref of [
    "secrets.tauri_signing_private_key",
    "SECRETS.Tauri_Signing_Private_Key_Password",
    "secrets . TAURI_signing_PRIVATE_key",
    "secrets['tauri_signing_private_key']",
    'Secrets["tauri_signing_private_key_password"]',
    "secrets [ 'Tauri_Signing_Private_Key' ]",
  ]) {
    const steps = ["      - name: Leak", `        env:\n          X: \${{ ${ref} }}`];
    const findings = run(workflow(steps), ".github/workflows/ci.yml");
    assert.deepEqual(rules(findings), ["signing-key"], ref);
    assert.equal(findings[0].line, 9, ref);
  }
  // Outside ${{ }}, in an `if:`, only the parsed name can tell.
  const bare = workflow([
    "      - name: Leak",
    "        if: SECRETS.tauri_signing_private_key != ''",
    "        run: true",
  ]);
  assert.deepEqual(rules(run(bare)), ["signing-key"]);
  // The reference may break across lines of an expression.
  const broken = workflow([
    "      - name: Leak",
    "        run: |",
    "          echo ${{ secrets.",
    "            tauri_signing_private_key }}",
  ]);
  assert.deepEqual(rules(run(broken)), ["signing-key"]);
});

test("a signing secret in lower case inside the signing step passes, other secrets anywhere", () => {
  const lower = signingStep.map((l) => l.replace(/secrets\.TAURI\w+/, (m) => m.toLowerCase()));
  assert.deepEqual(signingFindings(signingJob(lower)), []);
  const other = workflow([
    "      - name: Other",
    "        env:",
    "          A: ${{ secrets.GITHUB_TOKEN }}",
    "          B: ${{ secrets['SOME_OTHER'] }}",
    "          C: ${{ SECRETS.SOME_OTHER }}",
    "        run: true",
  ]);
  assert.deepEqual(run(other), []);
});

test("reaching secrets without a literal name is refused, even inside the signing step", () => {
  for (const expr of [
    "secrets.*",
    "join(secrets.*)",
    "format('{0}', secrets)",
    "SECRETS[format('TAURI_{0}', 'X')]",
    "toJson(SECRETS)",
  ]) {
    const line = `        run: echo '\${{ ${expr} }}'`;
    assert.deepEqual(rules(run(workflow(["      - name: Dump", line]))), ["signing-key"], expr);
  }
  const split = [
    "      - name: Dump",
    "        run: |",
    "          echo ${{ toJSON(",
    "secrets) }}",
  ];
  assert.deepEqual(rules(run(workflow(split))).includes("signing-key"), true);
  assert.deepEqual(
    rules(
      run(workflow(["      - name: Dump", "        if: toJSON(secrets) != ''", "        run: x"])),
    ),
    ["signing-key"],
  );
  assert.deepEqual(
    rules(
      run(
        workflow(["      - name: Dump", "        if: secrets[matrix.x] != ''", "        run: x"]),
      ),
    ),
    ["signing-key"],
  );
  // An `if:` is an expression without ${{ }}, on its line or in a block below it.
  for (const lines of [
    ["if: format('{0}', secrets) != ''"],
    ["if: secrets"],
    ["if: >-", "  format('{0}',", "  secrets) != ''"],
    ["if: fromJSON(SECRETS) != ''"],
    ["- if: secrets != ''", "  run: x"],
  ]) {
    assert.ok(wholesaleSecrets(lines.join("\n")).length > 0, lines.join(" / "));
  }
  assert.deepEqual(wholesaleSecrets("if: secrets.A != ''\nrun: echo secrets"), []);
  assert.deepEqual(wholesaleSecrets("if: a # the secrets\nrun: echo secrets"), []);
  assert.deepEqual(wholesaleSecrets("if: a\nname: secrets"), []);
  assert.deepEqual(wholesaleSecrets("- if: a\n  name: secrets"), []);
  // The word in prose is not an access.
  assert.deepEqual(
    run(workflow(["      # the secrets of this job", "      - run: echo secrets"])),
    [],
  );
});

const indirection = (...lines) => [...new Set(yamlIndirection(lines).map((f) => f.line))];

test("anchors, aliases and merge keys are refused wherever a node can start", () => {
  for (const line of [
    "env: &keys",
    "env: *keys",
    "<<: *keys",
    "<<: x",
    '"<<": { a: b }',
    "'<<': x",
    "<<: { a: b }",
    "- &step",
    "- *step",
    "- - *step",
    "- name: &n x",
    "- env: *keys",
    "? &k",
    ": *v",
    "&k key: value",
    "*k : value",
    "a: !!map &x",
    "!!merge <<: *x",
    "a: [b, *c]",
    "a: [*c]",
    "a: {b: *c}",
    "a: {<<: {b: c}}",
    "a: [&x b]",
    '"a b": *c',
    "'a': &c",
    "a: *c # comment",
    "a: &&",
  ]) {
    assert.deepEqual(indirection(line), [1], line);
  }
});

test("the same characters in text, quotes, comments and block scalars are not YAML indirection", () => {
  for (const line of [
    "if: a && b",
    "if: ${{ a && b }}",
    "run: echo *.txt & wait",
    "run: echo it's &x *y",
    "name: a &b *c",
    `cron: '0 * * * *'`,
    `- "**/*.rs"`,
    `a: "&x"`,
    `a: '*x'`,
    `a: 'it''s &x'`,
    `a: ["*b", '&c', d] # *e`,
    "# env: &keys",
    "a: b # <<: *c",
    "uses: a/b@c",
  ]) {
    assert.deepEqual(indirection(line), [], line);
  }
  assert.deepEqual(indirection("run: |", "  a: &b", "  <<: *c", "", "    *d"), []);
  assert.deepEqual(indirection("- run: >-", "    *d", "  # c", "  env: x"), []);
});

test("a block scalar ends where its key's indent returns, so what follows is still read", () => {
  assert.deepEqual(indirection("a: |", "  text", "b: &x"), [3]);
  assert.deepEqual(indirection("- run: |", "    text", "  env: *x"), [3]);
  assert.deepEqual(indirection("- |", "  text", "- *x"), [3]);
  assert.deepEqual(indirection("  a: |2", "      text", "  b: *x"), [3]);
  assert.deepEqual(indirection("a: !!str |", "  text", "b: *x"), [1, 3]);
  // Apostrophes in plain text open nothing that could hide the next line.
  assert.deepEqual(indirection("name: it's", "env: &x", "other: it's"), [2]);
});

test("what the scan cannot tell apart on one line is refused", () => {
  assert.deepEqual(indirection('a: "open', '  *x"'), [1, 2]);
  assert.deepEqual(indirection("a: 'open", "b: *x"), [1, 2]);
  assert.deepEqual(indirection("a: [b,", "  c, *d]"), [1]);
  assert.deepEqual(indirection("a: {b: 1,", "  c: 2}"), [1]);
  assert.deepEqual(indirection("a:", "  |", "  *x"), [2, 3]);
});

test("YAML that would make GitHub read a name the scan does not see is refused", () => {
  for (const line of [
    "- { uses: evil/act@main }",
    "- {uses: actions/checkout@v7}",
    "with: { persist-credentials: true }",
    "steps: [{ uses: evil/act@main }]",
    '- "uses": actions/checkout@v7',
    "- 'uses': evil/act@main",
    '"jobs":',
    "- ? uses",
    ": evil/act@main",
    "uses : evil/act@main",
    "uses: !!str evil/act@main",
    "!!map a: b",
    'K: "${{ secrets.TAURI_SIGNING_PRIVATE_K\\x45Y }}"',
    'K: "${{ secrets.TAURI_SIGNING_PRIVATE\\_KEY }}"',
    '"\\x75ses": a',
    'a: ["\\x41"]',
    "      -",
    "- # comment",
    "# a comment\r- uses: evil/act@main",
    "a: b\u2028c: d",
    "a: b\u0085c: d",
  ]) {
    assert.ok(indirection(line).length > 0, JSON.stringify(line));
  }
  // The same characters where YAML decodes nothing.
  for (const line of [
    "run: echo \\x45 'a' \"b\"",
    "a: 'b\\x45'",
    'a: "b c"',
    "if: ${{ a }}",
    "a: b\r",
    "cron: '0 0 * * 0'",
  ]) {
    assert.deepEqual(indirection(line), [], line);
  }
  assert.deepEqual(indirection("run: |", '  K: "\\x45"', "  { a: b }", '  "c": d', "  ? e"), []);
});

test("those forms fail the whole check, in a real workflow", () => {
  for (const [lines, wanted] of [
    [["      - { uses: tauri-apps/tauri-action@v1 }"], "yaml-indirection"],
    [['      - "uses": actions/checkout@v7'], "yaml-indirection"],
    [["      - ? uses", "        : evil/act@main"], "yaml-indirection"],
    [["      - Uses: evil/act@main"], "pinned-uses"],
    [
      [
        "      - name: x",
        "        env:",
        '          K: "${{ secrets.TAURI_SIGNING_PRIVATE_K\\x45Y }}"',
      ],
      "yaml-indirection",
    ],
  ]) {
    assert.ok(rules(run(workflow(lines))).includes(wanted), lines.join(" / "));
  }
  assert.ok(
    rules(run(`on: push\n"jobs":\n  a:\n    steps:\n      - uses: ${CHECKOUT}\n`)).includes(
      "yaml-indirection",
    ),
  );
  // A file whose jobs the reader cannot find is not a pass.
  assert.deepEqual(rules(run("on: push\n")), ["yaml-indirection"]);
});

test("a pinned action under a key in another case is still refused", () => {
  const findings = run(workflow([`      - Uses: pnpm/action-setup@${SHA} # v6`]));
  assert.deepEqual(rules(findings), ["pinned-uses"]);
  assert.match(findings[0].message, /write the key as/);
});

test("a checkout the reader cannot place in a step is refused", () => {
  const offLayout = `on: push\njobs:\n   a:\n      steps:\n         - uses: ${CHECKOUT}\n           with:\n              persist-credentials: true\n`;
  assert.ok(rules(run(offLayout)).includes("checkout-credentials"));
  const lone = `on: push\njobs:\n  a:\n    steps:\n      -\n        uses: ${CHECKOUT}\n`;
  assert.ok(rules(run(lone)).includes("checkout-credentials"));
});

test("a workflow using an anchor or a merge key fails the check", () => {
  const text = workflow([
    "      - name: One",
    "        env: &shared",
    "          A: b",
    "        run: true",
    "      - name: Two",
    "        env: *shared",
    "        run: true",
  ]);
  const findings = run(text);
  assert.deepEqual(rules(findings), ["yaml-indirection", "yaml-indirection"]);
  assert.deepEqual(
    findings.map((f) => f.line),
    [8, 12],
  );
});

test("a cargo command in a workflow that can resolve dependencies needs --locked", () => {
  for (const cmd of [
    "cargo build --release --bin namehold-syncd",
    "cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings",
    "cargo +nightly test --lib",
    "cargo install tauri-driver --version 2.1.0",
  ]) {
    assert.deepEqual(rules(run(workflow([`      - run: ${cmd}`]))), ["cargo-locked"], cmd);
    assert.deepEqual(run(workflow([`      - run: ${cmd} --locked`])), [], cmd);
  }
  // fmt and audit read no registry; a comment is not a command.
  const quiet = workflow([
    "      - run: cargo fmt --all -- --check",
    "      - run: cargo audit --deny warnings",
    "      # cargo build runs in the next step",
    "      - run: true",
  ]);
  assert.deepEqual(run(quiet), []);
});

test("cargo install in a workflow needs an exact version", () => {
  for (const cmd of [
    "cargo install tauri-driver --locked",
    "cargo install tauri-driver --version 2 --locked",
    "cargo install tauri-driver --version ^2.1.0 --locked",
    "cargo install tauri-driver@2 --locked",
  ]) {
    assert.deepEqual(rules(run(workflow([`      - run: ${cmd}`]))), ["cargo-install"], cmd);
  }
  for (const cmd of [
    "cargo install tauri-driver --version 2.1.0 --locked",
    "cargo install tauri-driver --version =2.1.0 --locked",
    "cargo install tauri-driver@2.1.0 --locked",
  ]) {
    assert.deepEqual(run(workflow([`      - run: ${cmd}`])), [], cmd);
  }
});

test("tauri build in a workflow needs -- --locked for cargo", () => {
  assert.deepEqual(rules(run(workflow(["      - run: pnpm tauri build --debug --no-bundle"]))), [
    "cargo-locked",
  ]);
  assert.deepEqual(rules(run(workflow(["      - run: pnpm tauri build --locked"]))), [
    "cargo-locked",
  ]);
  assert.deepEqual(
    run(workflow(["      - run: pnpm tauri build --debug --no-bundle -- --locked"])),
    [],
  );
});

test("cargo build and tauri build in a build script need --locked; comments do not count", () => {
  const found = (text) => rules(checkBuildScript("x", text));
  assert.deepEqual(found("cargo build --release --bin namehold-syncd\n"), ["cargo-locked"]);
  assert.deepEqual(found("execSync(`cargo build${cargoFlag} --bin namehold-syncd`);\n"), [
    "cargo-locked",
  ]);
  assert.deepEqual(found("    cd src-tauri && pnpm tauri build\n"), ["cargo-locked"]);
  assert.deepEqual(
    found("execSync(`cargo build --locked${cargoFlag} --bin namehold-syncd`);\n"),
    [],
  );
  assert.deepEqual(
    found("# `cargo build` above runs build.rs\n// `cargo build` puts it under target/\n"),
    [],
  );
});

test("npm installs in a build script need --ignore-scripts; a global-install hint does not", () => {
  const found = (text) => rules(checkBuildScript("x", text));
  assert.deepEqual(found("npm ci --no-audit\n"), ["npm-scripts"]);
  assert.deepEqual(found('  npm install --no-save "hsd@8"\n'), ["npm-scripts"]);
  assert.deepEqual(found("npm i hsd\n"), ["npm-scripts"]);
  assert.deepEqual(found("npm ci --ignore-scripts\n"), []);
  assert.deepEqual(found("npm install --ignore-scripts --no-save hsd\n"), []);
  assert.deepEqual(found("die 'install hsd: npm i -g hsd'\n"), []);
  assert.deepEqual(found("# run npm install first\n"), []);
});

test("the repository check wants the vector lockfile tracked, the command documented, HAR ignored", () => {
  const dir = mkdtempSync(join(tmpdir(), "hygiene-"));
  const git = (...args) => execFileSync("git", args, { cwd: dir, stdio: "ignore" });
  git("init", "-q");
  const put = (rel, text) => {
    mkdirSync(join(dir, rel, ".."), { recursive: true });
    writeFileSync(join(dir, rel), text);
  };
  const found = () => checkRepoHygiene(dir).map((f) => `${f.rule}:${f.file}`);
  const others = (...kept) => found().filter((f) => kept.some((k) => f === k));
  const lock = `npm-scripts:${VECTORS_LOCKFILE}`;
  const generator = `npm-scripts:${VECTORS_GENERATOR}`;
  const ignored = "ignored-files:.gitignore";
  const backend = `npm-scripts:${VECTORS_PACKAGE}`;
  const shakedex = `npm-scripts:${SHAKEDEX_SCRIPT}`;
  const signer = `npm-scripts:${SIGNER_LOCKFILE}`;
  const toolchain = `rust-toolchain:${TOOLCHAIN_FILE}`;
  const everything = [lock, generator, backend, shakedex, signer, toolchain, ignored, ignored];
  assert.deepEqual(found(), everything);
  assert.deepEqual(rules(checkRepo(dir)), rules(checkRepoHygiene(dir)), "checkRepo includes these");

  put(VECTORS_LOCKFILE, "{}");
  assert.deepEqual(found(), everything, "on disk is not tracked");
  git("add", VECTORS_LOCKFILE);
  assert.ok(!found().includes(lock));

  put(VECTORS_GENERATOR, "// npm install && node gen_hsd_vectors.js\n");
  assert.ok(found().includes(generator));
  put(VECTORS_GENERATOR, "// npm ci --ignore-scripts && NODE_BACKEND=js node gen_hsd_vectors.js\n");
  assert.ok(!found().includes(generator));

  put(VECTORS_PACKAGE, '{"scripts":{"gen":"node gen_hsd_vectors.js"}}');
  assert.ok(found().includes(backend));
  put(VECTORS_PACKAGE, '{"scripts":{"gen":"NODE_BACKEND=js node gen_hsd_vectors.js"}}');
  assert.ok(!found().includes(backend));

  put(SHAKEDEX_SCRIPT, "# NODE_BACKEND=js is a comment here\nnode shakedex\n");
  assert.ok(found().includes(shakedex));
  put(SHAKEDEX_SCRIPT, "NODE_BACKEND=js node shakedex\n");
  assert.ok(!found().includes(shakedex));

  put(TOOLCHAIN_FILE, '[toolchain]\nchannel = "stable"\n');
  assert.ok(found().includes(toolchain));
  put(TOOLCHAIN_FILE, '[toolchain]\nchannel = "1.99.1"\n');
  assert.ok(!found().includes(toolchain));
  const channelBreak = checkRepo(dir);
  assert.ok(!channelBreak.some((f) => f.rule === "rust-toolchain"));
  const wf = workflow([
    `      - uses: dtolnay/rust-toolchain@${SHA} # stable`,
    "        with:",
    "          toolchain: 1.98.0",
  ]);
  put(".github/workflows/x.yml", wf);
  assert.ok(
    checkRepo(dir).some((f) => f.rule === "rust-toolchain"),
    "a workflow on another version",
  );

  put(SIGNER_PACKAGE, '{"dependencies":{"@tauri-apps/cli":"^2.11.5"}}');
  put(SIGNER_LOCKFILE, '{"version": "2.11.5"}');
  put("pnpm-lock.yaml", "'@tauri-apps/cli@2.11.5':\n");
  git("add", SIGNER_LOCKFILE);
  assert.ok(found().includes(signer), "a range is not exact");
  put(SIGNER_PACKAGE, '{"dependencies":{"@tauri-apps/cli":"2.11.5"}}');
  assert.ok(!found().includes(signer));
  put("pnpm-lock.yaml", "'@tauri-apps/cli@2.11.4':\n");
  assert.ok(found().includes(signer), "another version than the build's");
  put("pnpm-lock.yaml", "'@tauri-apps/cli@2.11.5':\n");
  put(SIGNER_LOCKFILE, '{"version": "2.11.4"}');
  assert.ok(found().includes(signer), "the lockfile holds another version");
  put(SIGNER_LOCKFILE, '{"version": "2.11.5"}');
  git("rm", "-q", "--cached", SIGNER_LOCKFILE);
  assert.ok(found().includes(signer), "on disk is not tracked");
  git("add", SIGNER_LOCKFILE);

  put(".gitignore", "/capture.har\n");
  assert.deepEqual(others(ignored), [ignored], "a pattern for one path covers one path");
  put(".gitignore", "*.har\n");
  assert.deepEqual(found(), []);
});

test("the build scripts checked include the sidecar builds and the justfile", () => {
  const files = buildScripts(repoRoot);
  for (const f of [
    "scripts/stage-sidecar.sh",
    "src-tauri/build-sidecar.mjs",
    "src-tauri/build-sidecar.sh",
    "justfile",
  ]) {
    assert.ok(files.includes(f), f);
  }
  assert.ok(!files.includes("scripts/check-supply-chain.mjs"));
});

const realRelease = readFileSync(join(repoRoot, RELEASE), "utf8");
/** Rules that fire on the real release.yml after one edit (which must apply). */
const afterEdit = (from, to) => {
  const text = realRelease.replace(from, to);
  assert.ok(text !== realRelease, `edit did not apply: ${from}`);
  return rules(run(text, RELEASE));
};
const STEP_OF = (name) => `      - name: ${name}\n`;
const GATE = "Refuse a commit that is not on main with green CI";

test("release.yml's jobs keep their permissions, order and nothing conditional", () => {
  for (const [from, to] of [
    ["permissions:\n  contents: read\n", "permissions:\n  contents: write\n"],
    ["permissions:\n  contents: read\n", "permissions: write-all\n"],
    ["permissions:\n  contents: read\n", ""],
    [
      "needs: prepare\n    permissions:\n      contents: read",
      "needs: prepare\n    permissions:\n      contents: write",
    ],
    [
      "needs: prepare\n    permissions:\n      contents: read",
      "needs: prepare\n    permissions:\n      contents: read\n      actions: write",
    ],
    [
      "needs: prepare\n    permissions:\n      contents: read",
      "needs: prepare\n    permissions: write-all",
    ],
    ["needs: prepare\n", ""],
    ["needs: [prepare, build]\n", "needs: [prepare]\n"],
    ["needs: [prepare, build, sign]\n", "needs: [prepare, build]\n"],
    ["needs: [prepare, build, sign]\n", "needs: [prepare, build, sign]\n    if: always()\n"],
    ["needs: [prepare, build]\n", "needs: [prepare, build]\n    continue-on-error: true\n"],
    ["    environment: release\n", ""],
    ["    environment: release\n", "    environment: other\n"],
    ["needs: prepare\n", "needs: prepare\n    environment: release\n"],
    [
      "  publish:\n",
      "  extra:\n    runs-on: ubuntu-latest\n    steps:\n      - run: true\n\n  publish:\n",
    ],
    ["  publish:\n", "  extra:\n    uses: ./.github/workflows/x.yml\n\n  publish:\n"],
  ]) {
    assert.ok(afterEdit(from, to).includes("release-shape"), `${from} -> ${to}`);
  }
});

test("disabling a guard step of release.yml fails the check", () => {
  for (const [from, to] of [
    [STEP_OF(GATE), `${STEP_OF(GATE)}        continue-on-error: true\n`],
    [STEP_OF(GATE), `${STEP_OF(GATE)}        if: false\n`],
    [`check-runs.tsv\n\n`, `check-runs.tsv || true\n\n`],
    [`status="$(gh api "repos/$GH_REPO/compare/main...$SHA" --jq .status)"`, "status=identical"],
    ["compare/main...$SHA", "compare/$SHA...$SHA"],
    [`select(.app.slug == "github-actions")`, "."],
    [
      "SHA: ${{ github.sha }}\n        run: |\n          set -euo pipefail\n          status",
      "SHA: main\n        run: |\n          set -euo pipefail\n          status",
    ],
    [
      "pnpm tauri build --no-sign ${{ matrix.args }} -- --locked",
      "pnpm tauri build --no-sign ${{ matrix.args }} -- --locked || true",
    ],
    ["node scripts/release-files.mjs stage src-tauri/target release-files", "true"],
    [
      `--json isDraft --jq .isDraft)" = "true" ] ||`,
      `--json isDraft --jq .isDraft)" = "true" ] || true ||`,
    ],
    ["--ignore-scripts --no-audit", "--no-audit"],
    [`sha256sum -- * > SHA256SUMS`, `touch SHA256SUMS`],
    [`"$signer" signer sign`, `npx tauri signer sign`],
    [`--app-version "$VERSION" `, ""],
    [
      `check release-files src-tauri/tauri.conf.json "$VERSION" "$GH_REPO"`,
      `check release-files src-tauri/tauri.conf.json "$VERSION" "$GH_REPO" || true`,
    ],
    ["node scripts/release-files.mjs compare release-files release-assets.tsv", "true"],
    [".state] | @tsv", `"uploaded"] | @tsv`],
    ["RELEASE_ID: ${{ needs.prepare.outputs.release_id }}", "RELEASE_ID: 1"],
  ]) {
    assert.ok(afterEdit(from, to).includes("release-shape"), `${from} -> ${to}`);
  }
});

test("guards of one job must stay in their order", () => {
  const build = realRelease.indexOf(STEP_OF("Build bundles"));
  const check = realRelease.indexOf("      # The AppImage runs as whoever");
  const collect = realRelease.indexOf(STEP_OF("Collect the release files"));
  const upload = realRelease.indexOf("      - uses: actions/upload-artifact", collect);
  assert.ok(0 < build && build < check && check < collect && collect < upload);
  const swapped =
    realRelease.slice(0, build) +
    realRelease.slice(collect, upload) +
    realRelease.slice(build, collect) +
    realRelease.slice(upload);
  const findings = run(swapped, RELEASE);
  assert.deepEqual(rules(findings), ["release-shape"]);
  assert.match(findings[0].message, /out of order/);
});

test("no step of release.yml may carry on after a failure", () => {
  assert.ok(
    afterEdit(
      "      - name: Upload the files to the draft\n",
      "      - name: Upload the files to the draft\n        continue-on-error: true\n",
    ).includes("release-shape"),
  );
});

test("a guard must run before what it guards, in the same job", () => {
  const draft = realRelease.indexOf(STEP_OF("Create draft release"));
  const gate = realRelease.indexOf(STEP_OF(GATE));
  assert.ok(gate > 0 && draft > gate);
  const swapped =
    realRelease.slice(0, gate) +
    realRelease.slice(draft, realRelease.indexOf(STEP_OF("Set release notes from CHANGELOG.md"))) +
    realRelease.slice(gate, draft) +
    realRelease.slice(realRelease.indexOf(STEP_OF("Set release notes from CHANGELOG.md")));
  assert.ok(rules(run(swapped, RELEASE)).includes("release-shape"), "draft before the gate");

  const early = `      - run: gh release edit "v1" --draft=false\n`;
  assert.ok(
    afterEdit(
      STEP_OF("Check every file and signature, write latest.json"),
      `${early}${STEP_OF("Check every file and signature, write latest.json")}`,
    ).includes("release-shape"),
    "un-draft before the checks",
  );
  const viaApi = `      - run: gh api -X PATCH releases/1 -F draft=false\n`;
  assert.ok(
    afterEdit(
      STEP_OF("Upload the files to the draft"),
      `${viaApi}${STEP_OF("Upload the files to the draft")}`,
    ).includes("release-shape"),
    "un-draft through the API",
  );
  // In another job entirely.
  assert.ok(
    afterEdit(
      STEP_OF("Build bundles"),
      `      - run: gh release upload v1 x\n${STEP_OF("Build bundles")}`,
    ).includes("release-shape"),
    "upload from the build job",
  );
});

test("the sign job runs only GitHub's artifact actions and its own steps", () => {
  const inSign = `      - name: Install the Tauri CLI\n        working-directory`;
  for (const extra of [
    `      - uses: pnpm/action-setup@${SHA} # v6\n`,
    `      - uses: actions/setup-node@${SHA} # v7\n`,
    "      - run: ./anything.sh\n",
  ]) {
    assert.ok(afterEdit(inSign, `${extra}${inSign}`).includes("release-shape"), extra);
  }
});

test("every job the guards name is one release.yml may have, and the checks see each guard", () => {
  for (const guard of RELEASE_GUARDS) assert.ok(RELEASE_JOBS[guard.job], guard.job);
});

test("the tools ci.yml and e2e.yml install, and what the sign job installs, are exact", () => {
  const lock = JSON.parse(readFileSync(join(repoRoot, SIGNER_LOCKFILE), "utf8"));
  const pnpm = readFileSync(join(repoRoot, "pnpm-lock.yaml"), "utf8");
  const packages = Object.entries(lock.packages).filter(([k]) => k !== "");
  assert.ok(packages.length > 1);
  for (const [name, entry] of packages) {
    const id = name.replace("node_modules/", "");
    assert.match(entry.resolved, /^https:\/\/registry\.npmjs\.org\//, id);
    assert.equal(entry.hasInstallScript, undefined, id);
    assert.ok(id.startsWith("@tauri-apps/cli"), `the signer installs only the CLI, not ${id}`);
    // Byte for byte the package the build's own lockfile pins.
    assert.ok(
      pnpm.includes(`'${id}@${entry.version}':\n    resolution: {integrity: ${entry.integrity}}`),
      id,
    );
  }
});

test("this repository passes every rule", () => {
  assert.deepEqual(checkRepo(repoRoot), []);
});
