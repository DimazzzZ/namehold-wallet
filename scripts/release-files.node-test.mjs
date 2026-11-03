// Tests for release-files.mjs (node --test).
import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, randomBytes, sign as edSign } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  checkFiles,
  compareAssets,
  parseAssets,
  parsePublicKey,
  stage,
  verifySignature,
} from "./release-files.mjs";

const SCRIPT = fileURLToPath(new URL("./release-files.mjs", import.meta.url));
const sha = (b) => createHash("sha256").update(b).digest("hex");
const b64 = (b) => Buffer.from(b).toString("base64");

// Made with `tauri signer` 2.11.5 from a throwaway key that was then deleted.
const CLI_PUBKEY =
  "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDVDQUY0MzFENzc4NThCM0YKUldRL2k0VjNIVU92WEY4OU9tbFRuakJMZkVNUWY4VEJPTjVCYVpBV3VZMHRQTmloTWVWVGMyWDEK";
const CLI_SIG =
  "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRL2k0VjNIVU92WEVaVFY3bTgxNWhPa0t4TU5pcm1peGpGM0JrY1AxVE9BUmFQa0RTSU5IQ0JnSGFKbWRmd3ZLelhZZUt0a1RpcEZnbFlicHM1dm82N2dyMHBSeFErZVEwPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzNjQ1MTMyCWZpbGU6Zi50eHQJdmVyc2lvbjoxLjIuMy1yYy4xClovNUhKTUhFbURVc3NMYW1sMEpiSnk3dmE3QWN5SnZ0UWJWK3FZbjNKUE8rSjFVbFdQNkw2R1FtY3pSN0lqQWdjQ3EwT3QwYmVEbGlZblpxUzh1SEJRPT0K";
const CLI_FILE = Buffer.from("hello namehold\n");

/** A key and a signer that write what `tauri signer sign` writes. */
function keypair() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const id = randomBytes(8);
  const raw = publicKey.export({ format: "der", type: "spki" }).subarray(-32);
  const pubBox = b64(Buffer.concat([Buffer.from("Ed"), id, raw]));
  const pubkey = b64(`untrusted comment: minisign public key: X\n${pubBox}\n`);
  const sign = (bytes, trusted, { alg = "ED", keyId = id } = {}) => {
    const digest = alg === "ED" ? createHash("blake2b512").update(bytes).digest() : bytes;
    const signature = edSign(null, digest, privateKey);
    const global = edSign(null, Buffer.concat([signature, Buffer.from(trusted)]), privateKey);
    const box = b64(Buffer.concat([Buffer.from(alg), keyId, signature]));
    return b64(
      `untrusted comment: signature from tauri secret key\n${box}\ntrusted comment: ${trusted}\n${b64(global)}\n`,
    );
  };
  return { pubkey, sign };
}

const VERSION = "1.2.3-rc.1";
const INSTALLERS = [
  "Namehold_1.2.3-rc.1_universal.dmg",
  "Namehold_1.2.3-rc.1_universal.app.tar.gz",
  "Namehold_1.2.3-rc.1_amd64.AppImage",
  "Namehold_1.2.3-rc.1_amd64.deb",
  "Namehold-1.2.3.rc.1-1.x86_64.rpm",
  "Namehold_1.2.3-rc.1_x64-setup.exe",
];
const REPO = "o/r";

/** A complete, valid directory of release files, as the sign job leaves it. */
function release() {
  const keys = keypair();
  const files = new Map(INSTALLERS.map((n) => [n, Buffer.from(`bytes of ${n}`)]));
  const sums = INSTALLERS.map((n) => `${sha(files.get(n))}  ${n}\n`).join("");
  files.set("SHA256SUMS", Buffer.from(sums));
  for (const n of [...files.keys()]) {
    const trusted = `timestamp:1\tfile:${n}\tversion:${VERSION}`;
    files.set(`${n}.sig`, Buffer.from(keys.sign(files.get(n), trusted)));
  }
  const opts = { pubkey: keys.pubkey, version: VERSION, repo: REPO, notes: "what is new" };
  return { files, opts, keys };
}

const problemsOf = (files, opts) => checkFiles(files, opts).problems;

test("a signature the real tauri CLI wrote verifies, and any change to the file does not", () => {
  const key = parsePublicKey(CLI_PUBKEY);
  const fields = verifySignature(CLI_FILE, CLI_SIG, key);
  assert.equal(fields.file, "f.txt");
  assert.equal(fields.version, "1.2.3-rc.1");
  assert.throws(
    () => verifySignature(Buffer.from("hello namehold!\n"), CLI_SIG, key),
    /bad signature/,
  );
});

test("a key that is not a minisign Ed25519 key is refused", () => {
  assert.throws(() => parsePublicKey(b64("comment\nnot a key\n")), /not a minisign/);
  assert.throws(() => parsePublicKey(""), /not a minisign/);
});

test("the signature is refused if signed by another key, not prehashed, or malformed", () => {
  const { pubkey, sign } = keypair();
  const key = parsePublicKey(pubkey);
  const data = Buffer.from("data");
  assert.equal(verifySignature(data, sign(data, "file:a"), key).file, "a");
  assert.throws(() => verifySignature(data, keypair().sign(data, "file:a"), key), /another key/);
  assert.throws(() => verifySignature(data, sign(data, "file:a", { alg: "Ed" }), key), /prehashed/);
  assert.throws(() => verifySignature(data, "", key), /prehashed/);
  assert.throws(() => verifySignature(data, b64("a\nb\nc\nd\n"), key), /prehashed/);
});

test("a changed trusted comment is refused", () => {
  const { pubkey, sign } = keypair();
  const key = parsePublicKey(pubkey);
  const data = Buffer.from("data");
  const text = Buffer.from(sign(data, "file:a\tversion:1"), "base64").toString();
  const swapped = b64(text.replace("version:1", "version:2"));
  assert.throws(() => verifySignature(data, swapped, key), /trusted comment/);
  const none = b64(text.replace(/^trusted comment: /m, ""));
  assert.throws(() => verifySignature(data, none, key), /no trusted comment/);
});

test("a complete release passes and latest.json serves each platform its signed file", () => {
  const { files, opts } = release();
  const { problems, manifest } = checkFiles(files, opts);
  assert.deepEqual(problems, []);
  assert.equal(manifest.version, VERSION);
  assert.equal(manifest.notes, "what is new");
  assert.ok(!Number.isNaN(Date.parse(manifest.pub_date)));
  const exe = "Namehold_1.2.3-rc.1_x64-setup.exe";
  assert.deepEqual(manifest.platforms["windows-x86_64"], {
    signature: files.get(`${exe}.sig`).toString().trim(),
    url: `https://github.com/o/r/releases/download/v1.2.3-rc.1/${exe}`,
  });
});

test("latest.json has every platform key v0.6.1 published", () => {
  const { files, opts } = release();
  const { platforms } = checkFiles(files, opts).manifest;
  assert.deepEqual(Object.keys(platforms).sort(), [
    "darwin-aarch64",
    "darwin-aarch64-app",
    "darwin-universal",
    "darwin-universal-app",
    "darwin-x86_64",
    "darwin-x86_64-app",
    "linux-x86_64",
    "linux-x86_64-appimage",
    "linux-x86_64-deb",
    "linux-x86_64-rpm",
    "windows-x86_64",
    "windows-x86_64-nsis",
  ]);
  for (const [platform, entry] of Object.entries(platforms)) {
    const [, suffix] = {
      darwin: [0, ".app.tar.gz"],
      windows: [0, "-setup.exe"],
    }[platform.split("-")[0]] ?? [
      0,
      platform.endsWith("deb") ? ".deb" : platform.endsWith("rpm") ? ".rpm" : ".AppImage",
    ];
    assert.ok(entry.url.endsWith(suffix), `${platform} -> ${entry.url}`);
  }
});

test("a missing installer of any kind stops the release", () => {
  for (const name of INSTALLERS) {
    const { files, opts } = release();
    files.delete(name);
    files.delete(`${name}.sig`);
    assert.ok(
      problemsOf(files, opts).some((p) => p.startsWith("expected one")),
      name,
    );
  }
});

test("a second file of a kind or an unknown file stops the release", () => {
  const twice = release();
  twice.files.set("Other.deb", Buffer.from("x"));
  assert.ok(
    problemsOf(twice.files, twice.opts).some((p) => /expected one \*\.deb, found 2/.test(p)),
  );
  const unknown = release();
  unknown.files.set("evil.sh", Buffer.from("x"));
  assert.ok(problemsOf(unknown.files, unknown.opts).some((p) => /evil\.sh: not a file/.test(p)));
});

test("a file with no signature, a dmg and SHA256SUMS included, stops the release", () => {
  for (const name of [...INSTALLERS, "SHA256SUMS"]) {
    const { files, opts } = release();
    files.delete(`${name}.sig`);
    assert.ok(problemsOf(files, opts).includes(`${name}: no .sig`), name);
  }
});

test("a signature from another key, for another version or as another file stops the release", () => {
  const name = INSTALLERS[3];
  const other = release();
  other.files.set(
    `${name}.sig`,
    Buffer.from(
      keypair().sign(other.files.get(name), `timestamp:1\tfile:${name}\tversion:${VERSION}`),
    ),
  );
  assert.ok(problemsOf(other.files, other.opts).includes(`${name}: signed by another key`));

  const version = release();
  version.files.set(
    `${name}.sig`,
    Buffer.from(
      version.keys.sign(version.files.get(name), `timestamp:1\tfile:${name}\tversion:1.0.0`),
    ),
  );
  assert.ok(
    problemsOf(version.files, version.opts).some((p) => /signed for version "1\.0\.0"/.test(p)),
  );

  const unversioned = release();
  unversioned.files.set(
    `${name}.sig`,
    Buffer.from(unversioned.keys.sign(unversioned.files.get(name), `timestamp:1\tfile:${name}`)),
  );
  assert.ok(
    problemsOf(unversioned.files, unversioned.opts).some((p) => /signed for version/.test(p)),
  );

  const asOther = release();
  asOther.files.set(
    `${name}.sig`,
    Buffer.from(
      asOther.keys.sign(asOther.files.get(name), `timestamp:1\tfile:x.deb\tversion:${VERSION}`),
    ),
  );
  assert.ok(problemsOf(asOther.files, asOther.opts).some((p) => /signed as "x\.deb"/.test(p)));
});

test("an installer changed after signing stops the release, and so does a stray signature", () => {
  const changed = release();
  changed.files.set(INSTALLERS[0], Buffer.from("other bytes"));
  const problems = problemsOf(changed.files, changed.opts);
  assert.ok(problems.includes(`${INSTALLERS[0]}: bad signature`));
  assert.ok(problems.includes("SHA256SUMS does not match the files"));
  const stray = release();
  stray.files.set("gone.deb.sig", Buffer.from("x"));
  assert.ok(
    problemsOf(stray.files, stray.opts).some((p) => /signs a file that is not there/.test(p)),
  );
});

test("SHA256SUMS missing or not matching the files stops the release", () => {
  const missing = release();
  missing.files.delete("SHA256SUMS");
  missing.files.delete("SHA256SUMS.sig");
  assert.ok(problemsOf(missing.files, missing.opts).includes("SHA256SUMS is missing"));
  const edited = release();
  const lines = edited.files.get("SHA256SUMS").toString().split("\n").slice(1).join("\n");
  edited.files.set("SHA256SUMS", Buffer.from(lines));
  edited.files.set(
    "SHA256SUMS.sig",
    Buffer.from(
      edited.keys.sign(Buffer.from(lines), `timestamp:1\tfile:SHA256SUMS\tversion:${VERSION}`),
    ),
  );
  assert.deepEqual(problemsOf(edited.files, edited.opts), ["SHA256SUMS does not match the files"]);
});

test("the draft must hold exactly the checked files, by name and by digest", () => {
  const files = new Map([
    ["a.dmg", Buffer.from("a")],
    ["latest.json", Buffer.from("{}")],
  ]);
  const good = [...files].map(([name, b]) => ({
    name,
    digest: `sha256:${sha(b)}`,
    state: "uploaded",
  }));
  assert.deepEqual(compareAssets(files, good), []);
  assert.deepEqual(compareAssets(files, good.slice(1)), ["a.dmg: not on the release"]);
  assert.deepEqual(compareAssets(files, [...good, { name: "b", digest: "x", state: "uploaded" }]), [
    "b: on the release, but not a checked file",
  ]);
  const swapped = [{ ...good[0], digest: good[1].digest }, good[1]];
  assert.match(compareAssets(files, swapped)[0], /digest/);
  const nameBound = [
    { ...good[0], name: "latest.json" },
    { ...good[1], name: "a.dmg" },
  ];
  assert.equal(compareAssets(files, nameBound).length, 2);
  const partial = [{ ...good[0], state: "starter" }, good[1]];
  assert.match(compareAssets(files, partial)[0], /upload state/);
  const noDigest = [{ ...good[0], digest: "" }, good[1]];
  assert.match(compareAssets(files, noDigest)[0], /digest/);
});

test("assets parse from name, digest and state", () => {
  assert.deepEqual(parseAssets("a\tsha256:x\tuploaded\nb\t\tstarter\n\n"), [
    { name: "a", digest: "sha256:x", state: "uploaded" },
    { name: "b", digest: "", state: "starter" },
  ]);
});

function put(root, rel, content) {
  const p = join(root, rel);
  mkdirSync(join(p, ".."), { recursive: true });
  writeFileSync(p, content);
}

test("stage copies only installers, without tauri's signatures, under their asset names", () => {
  const target = mkdtempSync(join(tmpdir(), "stage-"));
  const out = join(target, "out");
  const uni = "universal-apple-darwin/release/bundle";
  put(target, `${uni}/dmg/Namehold_1.0.0_universal.dmg`, "dmg");
  put(target, `${uni}/macos/Namehold.app.tar.gz`, "tar");
  put(target, `${uni}/macos/Namehold.app.tar.gz.sig`, "throwaway");
  put(target, `${uni}/macos/Namehold.app/Contents/Info.plist`, "inside");
  put(target, "release/bundle/deb/Namehold_1.0.0_amd64.deb", "deb");
  put(target, "release/bundle/deb/Namehold_1.0.0_amd64.deb.sig", "throwaway");
  put(target, "release/bundle/rpm/Namehold-1.0.0~rc-1.x86_64.rpm", "rpm");
  put(target, "release/bundle/appimage/Namehold_1.0.0_amd64.AppImage", "ai");
  put(target, "release/bundle/nsis/Namehold_1.0.0_x64-setup.exe", "exe");
  put(target, "release/bundle/nsis/Namehold_1.0.0_x64.nsis.zip", "other");
  put(target, "release/namehold-wallet", "binary");
  stage(target, out);
  assert.deepEqual(readdirSync(out).sort(), [
    "Namehold-1.0.0.rc-1.x86_64.rpm",
    "Namehold_1.0.0_amd64.AppImage",
    "Namehold_1.0.0_amd64.deb",
    "Namehold_1.0.0_universal.app.tar.gz",
    "Namehold_1.0.0_universal.dmg",
    "Namehold_1.0.0_x64-setup.exe",
  ]);
  assert.equal(readFileSync(join(out, "Namehold_1.0.0_universal.app.tar.gz"), "utf8"), "tar");
});

test("stage refuses a target without installers, or an updater archive with no dmg to name it", () => {
  const empty = mkdtempSync(join(tmpdir(), "stage-"));
  assert.throws(() => stage(empty, join(empty, "out")), /no installers/);
  const noDmg = mkdtempSync(join(tmpdir(), "stage-"));
  put(noDmg, "release/bundle/macos/Namehold.app.tar.gz", "tar");
  assert.throws(() => stage(noDmg, join(noDmg, "out")), /expected one dmg/);
});

test("the command line writes latest.json only for a release that passes", () => {
  const { files, opts } = release();
  const dir = mkdtempSync(join(tmpdir(), "files-"));
  const conf = join(dir, "..", `conf-${Date.now()}.json`);
  writeFileSync(conf, JSON.stringify({ plugins: { updater: { pubkey: opts.pubkey } } }));
  for (const [n, b] of files) writeFileSync(join(dir, n), b);
  const exec = (...args) =>
    spawnSync("node", [SCRIPT, ...args], {
      encoding: "utf8",
      env: { ...process.env, RELEASE_NOTES: "notes here" },
    });
  const check = () => exec("check", dir, conf, VERSION, REPO);

  writeFileSync(join(dir, INSTALLERS[0]), "changed");
  assert.equal(check().status, 1);
  assert.equal(existsSync(join(dir, "latest.json")), false);

  writeFileSync(join(dir, INSTALLERS[0]), files.get(INSTALLERS[0]));
  const ok = check();
  assert.equal(ok.status, 0, ok.stderr);
  const manifest = JSON.parse(readFileSync(join(dir, "latest.json"), "utf8"));
  assert.equal(manifest.notes, "notes here");

  const tsv = join(mkdtempSync(join(tmpdir(), "tsv-")), "assets.tsv");
  const lines = readdirSync(dir).map(
    (n) => `${n}\tsha256:${sha(readFileSync(join(dir, n)))}\tuploaded`,
  );
  writeFileSync(tsv, lines.join("\n"));
  assert.equal(exec("compare", dir, tsv).status, 0);
  writeFileSync(tsv, lines.slice(1).join("\n"));
  assert.equal(exec("compare", dir, tsv).status, 1);

  assert.equal(exec("nonsense").status, 1);
});
