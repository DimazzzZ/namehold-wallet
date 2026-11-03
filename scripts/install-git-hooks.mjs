#!/usr/bin/env node
// install-git-hooks.mjs — make git run the secret check before a commit and
// before a push, so a recovery phrase or a private key is refused while it is
// still only on this machine. It sets core.hooksPath to .githooks, where
//   pre-commit, pre-merge-commit
//               run scripts/check-secrets.mjs --staged (what the commit adds)
//   pre-push    runs scripts/check-secrets.mjs --push   (every commit the
//               remote does not have: the files it adds and its message)
//
// `pnpm install` runs it (package.json `prepare`). It does nothing in CI (CI is
// set), outside a git checkout, or when this package is not the top of the
// checkout. It fails, and with it the install, rather than silently replace
// hooks git already runs: another core.hooksPath, or hooks in .git/hooks that
// would stop running.
//
// With --check it changes nothing and fails when a checkout that should have the
// hooks does not: pnpm skips `prepare` when nothing needs installing, so an
// older clone would otherwise stay unprotected without anyone noticing.
// `pnpm lint:secrets` runs it.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, realpathSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const HOOKS_PATH = ".githooks";
const AGAIN = "then run node scripts/install-git-hooks.mjs again.";

const git = (...args) =>
  execFileSync("git", args, {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
  }).trim();

function install(check) {
  if (process.env.CI) return say("CI: git hooks not installed.");
  let top;
  try {
    top = git("rev-parse", "--show-toplevel");
  } catch {
    return say("not a git checkout: git hooks not installed.");
  }
  if (realpathSync(top) !== realpathSync(ROOT))
    return say(`${ROOT} is not the top of the git checkout ${top}: git hooks not installed.`);
  let current = null;
  try {
    current = git("config", "--get", "core.hooksPath");
  } catch {
    // Unset: `git config --get` exits 1.
  }
  if (current === HOOKS_PATH)
    return say("git hooks already installed (core.hooksPath is .githooks).");
  if (check)
    return refuse(
      `git does not run the secret check before a commit or a push here (core.hooksPath is ${current ?? "unset"}). Run node scripts/install-git-hooks.mjs.`,
    );
  if (current !== null)
    return refuse(
      `git already runs hooks from ${current} (core.hooksPath). Call the hooks in .githooks ` +
        `from there, or unset core.hooksPath and ${AGAIN}`,
    );
  const dir = resolve(ROOT, git("rev-parse", "--git-path", "hooks"));
  const own = existsSync(dir)
    ? readdirSync(dir).filter((f) => !f.endsWith(".sample") && !f.startsWith("."))
    : [];
  if (own.length)
    return refuse(
      `${dir} holds hooks that git would stop running once it reads .githooks: ${own.join(", ")}. ` +
        `Move them into .githooks or remove them, ${AGAIN}`,
    );
  git("config", "core.hooksPath", HOOKS_PATH);
  say("installed: git runs the secret check in .githooks before a commit and a push from now on.");
}

function say(message) {
  console.log(`install-git-hooks: ${message}`);
}

function refuse(message) {
  console.error(`install-git-hooks: not installed. ${message}`);
  process.exitCode = 1;
}

install(process.argv[2] === "--check");
