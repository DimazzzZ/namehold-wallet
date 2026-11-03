#!/usr/bin/env node
// The files of a release, from the build jobs to the published draft.
//
//   stage <target-dir> <out-dir>
//     In a build job: copy each installer out of the bundle directories under
//     their release asset names. tauri build's own .sig files are not copied.
//   check <dir> <tauri.conf.json> <version> <owner/repo>
//     In the publish job: every installer kind exactly once, nothing unknown,
//     every file signed for this version by the key the app ships with, and
//     SHA256SUMS listing exactly the installers. Writes <dir>/latest.json from
//     the signatures that passed; its notes come from $RELEASE_NOTES.
//   compare <dir> <assets.tsv>
//     In the publish job: the release's assets (`name<TAB>digest<TAB>state` from
//     the releases API) are exactly the files of <dir>, byte for byte.
import { createHash, createPublicKey, verify as edVerify } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

/** Bundle directory of each installer kind, and the file name ending that identifies it. */
const BUNDLE_KINDS = [
  ["dmg", ".dmg"],
  ["macos", ".app.tar.gz"],
  ["appimage", ".AppImage"],
  ["deb", ".deb"],
  ["rpm", ".rpm"],
  ["nsis", "-setup.exe"],
];
const INSTALLERS = BUNDLE_KINDS.map(([, suffix]) => suffix);
const SUMS = "SHA256SUMS";
const MANIFEST = "latest.json";

// The updater looks up its platform key in latest.json; these are the keys
// tauri-action wrote up to 0.6.1, so installs of any earlier version keep finding one.
const PLATFORMS = {
  "windows-x86_64": "-setup.exe",
  "windows-x86_64-nsis": "-setup.exe",
  "linux-x86_64": ".AppImage",
  "linux-x86_64-appimage": ".AppImage",
  "linux-x86_64-deb": ".deb",
  "linux-x86_64-rpm": ".rpm",
  "darwin-aarch64": ".app.tar.gz",
  "darwin-x86_64": ".app.tar.gz",
  "darwin-universal": ".app.tar.gz",
  "darwin-aarch64-app": ".app.tar.gz",
  "darwin-x86_64-app": ".app.tar.gz",
  "darwin-universal-app": ".app.tar.gz",
};

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const unbase64 = (text) => Buffer.from(text.trim(), "base64");

/** Bundle directories under a cargo target dir: release/bundle and <triple>/release/bundle. */
function bundleDirs(targetDir) {
  const roots = [join(targetDir, "release", "bundle")];
  for (const e of readdirSync(targetDir, { withFileTypes: true })) {
    if (e.isDirectory()) roots.push(join(targetDir, e.name, "release", "bundle"));
  }
  return roots.filter((p) => existsSync(p) && statSync(p).isDirectory());
}

/** Copy the installers to outDir; returns their asset names. */
export function stage(targetDir, outDir) {
  mkdirSync(outDir, { recursive: true });
  const staged = [];
  for (const bundle of bundleDirs(targetDir)) {
    const inKind = (kind, suffix) =>
      existsSync(join(bundle, kind))
        ? readdirSync(join(bundle, kind)).filter((f) => f.endsWith(suffix))
        : [];
    for (const [kind, suffix] of BUNDLE_KINDS) {
      for (const file of inKind(kind, suffix)) {
        // The updater archive is plain `Namehold.app.tar.gz`; it takes the dmg's
        // version and architecture so that every release's name differs.
        let name = file;
        if (suffix === ".app.tar.gz") {
          const dmg = inKind("dmg", ".dmg");
          if (dmg.length !== 1) throw new Error(`${file}: expected one dmg, found ${dmg.length}`);
          name = dmg[0].replace(/\.dmg$/, suffix);
        }
        // GitHub renames an asset with other characters, and latest.json must match.
        name = name.replace(/[^a-zA-Z0-9_.-]/g, ".");
        copyFileSync(join(bundle, kind, file), join(outDir, name));
        staged.push(name);
      }
    }
  }
  if (staged.length === 0) throw new Error(`no installers under ${targetDir}`);
  return staged;
}

const ED25519_SPKI = Buffer.from("302a300506032b6570032100", "hex");

/** The minisign public key inside tauri.conf.json's `plugins.updater.pubkey`. */
export function parsePublicKey(pubkey) {
  const raw = unbase64(unbase64(pubkey).toString("utf8").split("\n")[1] ?? "");
  if (raw.length !== 42 || raw.subarray(0, 2).toString() !== "Ed") {
    throw new Error("not a minisign Ed25519 public key");
  }
  return {
    id: raw.subarray(2, 10),
    key: createPublicKey({
      key: Buffer.concat([ED25519_SPKI, raw.subarray(10)]),
      format: "der",
      type: "spki",
    }),
  };
}

/**
 * Check a `.sig` file's text against a file's bytes, as the updater does: a
 * prehashed minisign signature, then the signature over the trusted comment.
 * Returns the trusted comment's `key:value` fields; throws if anything is off.
 */
export function verifySignature(bytes, sigText, publicKey) {
  const [, box = "", comment = "", global = ""] = unbase64(sigText).toString("utf8").split("\n");
  const raw = unbase64(box);
  if (raw.length !== 74 || raw.subarray(0, 2).toString() !== "ED") {
    throw new Error("not a prehashed minisign signature");
  }
  if (!raw.subarray(2, 10).equals(publicKey.id)) throw new Error("signed by another key");
  const signature = raw.subarray(10);
  const digest = createHash("blake2b512").update(bytes).digest();
  if (!edVerify(null, digest, publicKey.key, signature)) throw new Error("bad signature");
  const PREFIX = "trusted comment: ";
  if (!comment.startsWith(PREFIX)) throw new Error("no trusted comment");
  const trusted = comment.slice(PREFIX.length);
  const signed = Buffer.concat([signature, Buffer.from(trusted)]);
  if (!edVerify(null, signed, publicKey.key, unbase64(global))) {
    throw new Error("bad signature over the trusted comment");
  }
  return Object.fromEntries(
    trusted.split("\t").map((field) => {
      const at = field.indexOf(":");
      return [field.slice(0, at), field.slice(at + 1)];
    }),
  );
}

/**
 * Everything wrong with a directory of release files. `files` maps each file's
 * name to its bytes. Returns `{ problems, manifest }`; the manifest is only
 * meaningful when there are no problems.
 */
export function checkFiles(files, { pubkey, version, repo, notes }) {
  const problems = [];
  const names = [...files.keys()];
  const signed = names.filter((n) => !n.endsWith(".sig"));
  const installer = {};
  for (const suffix of INSTALLERS) {
    const found = signed.filter((n) => n.endsWith(suffix));
    if (found.length !== 1) {
      problems.push(`expected one *${suffix}, found ${found.length}: ${found.join(", ")}`);
    }
    installer[suffix] = found[0];
  }
  for (const name of signed) {
    if (name !== SUMS && !INSTALLERS.some((s) => name.endsWith(s))) {
      problems.push(`${name}: not a file a release should hold`);
    }
  }
  if (!files.has(SUMS)) problems.push(`${SUMS} is missing`);

  const publicKey = parsePublicKey(pubkey);
  for (const name of signed) {
    const sig = files.get(`${name}.sig`);
    if (!sig) {
      problems.push(`${name}: no .sig`);
      continue;
    }
    try {
      const fields = verifySignature(files.get(name), sig.toString("utf8"), publicKey);
      if (fields.version !== version) {
        problems.push(`${name}: signed for version "${fields.version}", not "${version}"`);
      }
      if (fields.file !== name) problems.push(`${name}: signed as "${fields.file}"`);
    } catch (e) {
      problems.push(`${name}: ${e.message}`);
    }
  }
  for (const name of names) {
    if (name.endsWith(".sig") && !files.has(name.slice(0, -4))) {
      problems.push(`${name}: signs a file that is not there`);
    }
  }

  if (files.has(SUMS)) {
    const want = signed
      .filter((n) => n !== SUMS)
      .map((n) => `${sha256(files.get(n))}  ${n}`)
      .sort();
    const got = files.get(SUMS).toString("utf8").split("\n").filter(Boolean).sort();
    if (want.join("\n") !== got.join("\n")) problems.push(`${SUMS} does not match the files`);
  }

  const platforms = {};
  for (const [platform, suffix] of Object.entries(PLATFORMS)) {
    const name = installer[suffix];
    if (!name || !files.has(`${name}.sig`)) continue;
    platforms[platform] = {
      signature: files.get(`${name}.sig`).toString("utf8").trim(),
      url: `https://github.com/${repo}/releases/download/v${version}/${encodeURIComponent(name)}`,
    };
  }
  const manifest = { version, notes, pub_date: new Date().toISOString(), platforms };
  return { problems, manifest };
}

/** Parse `name<TAB>digest<TAB>state` lines from the releases API. */
export function parseAssets(text) {
  return text
    .split(/\r?\n/)
    .filter((l) => l !== "")
    .map((l) => {
      const [name, digest, state] = l.split("\t");
      return { name, digest, state };
    });
}

/** Everything that differs between the release's assets and the checked files. */
export function compareAssets(files, assets) {
  const problems = [];
  const onRelease = new Set(assets.map((a) => a.name));
  for (const name of files.keys()) {
    if (!onRelease.has(name)) problems.push(`${name}: not on the release`);
  }
  for (const a of assets) {
    if (!files.has(a.name)) {
      problems.push(`${a.name}: on the release, but not a checked file`);
    } else if (a.state !== "uploaded") {
      problems.push(`${a.name}: upload state is "${a.state}", not "uploaded"`);
    } else if (a.digest !== `sha256:${sha256(files.get(a.name))}`) {
      problems.push(`${a.name}: the release's digest "${a.digest}" is not this file's`);
    }
  }
  return problems;
}

function readDir(dir) {
  return new Map(readdirSync(dir).map((name) => [name, readFileSync(join(dir, name))]));
}

function fail(problems) {
  if (problems.length > 0) throw new Error(problems.join("\n"));
}

function main([cmd, ...args]) {
  if (cmd === "stage" && args.length === 2) {
    console.log(stage(args[0], args[1]).join("\n"));
    return;
  }
  if (cmd === "check" && args.length === 4) {
    const [dir, conf, version, repo] = args;
    const pubkey = JSON.parse(readFileSync(conf, "utf8")).plugins.updater.pubkey;
    const notes = process.env.RELEASE_NOTES ?? "";
    const { problems, manifest } = checkFiles(readDir(dir), { pubkey, version, repo, notes });
    fail(problems);
    writeFileSync(join(dir, MANIFEST), `${JSON.stringify(manifest, null, 2)}\n`);
    return;
  }
  if (cmd === "compare" && args.length === 2) {
    fail(compareAssets(readDir(args[0]), parseAssets(readFileSync(args[1], "utf8"))));
    return;
  }
  throw new Error(
    "usage: release-files.mjs stage <target-dir> <out-dir> | check <dir> <tauri.conf.json> <version> <owner/repo> | compare <dir> <assets.tsv>",
  );
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  try {
    main(process.argv.slice(2));
  } catch (e) {
    console.error(`::error::${String(e.message).replaceAll("\n", "%0A")}`);
    process.exit(1);
  }
}
