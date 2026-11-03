#!/usr/bin/env node
// check-secrets.mjs — refuse wallet secrets in the repository.
//
// GitHub's secret scanning does not recognise a BIP39 phrase, so this check does.
//
// It fails on:
//   - a BIP39 recovery phrase: 12, 15, 18, 21 or 24 consecutive English
//     wordlist words, in any letter case, whose checksum is valid. The words
//     may be separated by spaces, tabs, `,`, `;` or `|`, stand in a quoted word
//     array, or run across string concatenation, Rust line continuations and
//     escaped newlines. They may carry a list number, a bullet, comment or
//     quote marks, emphasis or an HTML tag (`1. word`, `1)word`, `1:word`,
//     `(1) word`, `#1 word`, `- word`, `// word`, `# word`, `> word`,
//     `1. **word**`, `| 1 | word |`, `<li>word</li>`), be joined throughout by
//     one of `-` `.` `_` `/` `:`, or be the string values of a JSON object
//     (`{"1":"word"}`, `{"i":0,"w":"word"}`). Zero-width characters are
//     ignored. The checksum keeps ordinary prose out: a run of wordlist words
//     that is not a phrase passes it 1 time in 16 to 256.
//   - text that decodes from base64 to any of the above.
//   - a seed, entropy or private key as hex: 16, 32 or 64 bytes within 40
//     characters of `seed`, `entropy`, `priv`, `secret`, `master` or `mnemonic`.
//   - an extended private key of any Handshake network: xprv (main), tprv
//     (testnet), rprv (regtest), sprv (simnet).
//   - a WIF private key of any Handshake network.
//   - a text file too large to scan (over 5 MB).
// A file is read as UTF-8, or as UTF-16 when it starts with a byte-order mark.
// Any other file with a NUL byte is binary: counted, not scanned.
//
// A secret split over several lines of hex or keys, and hex written as byte
// arrays, are not recognised; binary files are not read.
//
// Public test phrases are allowed: the BIP39 vectors of all-zero, all-0x7f,
// all-0x80 and all-0xff entropy (`abandon … about`, `abandon … art`, `zoo …
// wrong`, …). Any other phrase or key a test needs is generated at run time or
// listed by the SHA-256 of its words (of the key or lower-case hex, for those) in
// scripts/secret-scan-allow.txt, never by its words. The BIP39 wordlist
// (scripts/bip39-english.txt) is skipped at that exact path and only while it
// holds the published list: 24 of its consecutive lines pass the checksum.
//
// Usage:
//   node scripts/check-secrets.mjs               every file git tracks (CI)
//   node scripts/check-secrets.mjs --staged      the staged blobs a commit adds
//                                                (the pre-commit hook)
//   node scripts/check-secrets.mjs --push REMOTE every commit a push sends that
//                                                REMOTE does not have: the files
//                                                each one adds and its message.
//                                                Reads git's pre-push lines on
//                                                stdin (the pre-push hook). A
//                                                ref to a blob or a tree is
//                                                refused, not read
//   node scripts/check-secrets.mjs --range BEFORE HEAD MAIN
//                                                the commits BEFORE..HEAD (CI on
//                                                every branch). When BEFORE is not
//                                                a commit here (a new branch, a
//                                                force-push) it is the merge base
//                                                of HEAD and MAIN
//   node scripts/check-secrets.mjs FILE…         these files
// In --staged and --push a secret the previous version of the same file
// already held is not reported again: that version is in an earlier commit,
// which the push of it or CI checks. File names, commit authors and ref names
// are scanned too. It prints where it found a secret (file:line) and its kind,
// never the words or the key.
// scripts/install-git-hooks.mjs installs the hooks; `pnpm install` runs it.

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
/** The BIP39 English wordlist, as a path from the repository root. */
export const WORDLIST_PATH = "scripts/bip39-english.txt";
/** SHA-256 of the published BIP39 English wordlist (bitcoin/bips bip-0039/english.txt). */
export const WORDLIST_SHA256 = "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda";
export const MAX_TEXT_BYTES = 5_000_000;

const sha256 = (data) => createHash("sha256").update(data).digest();
const sha256hex = (data) => sha256(data).toString("hex");

/** Whether `buf` is the published wordlist (line endings aside: a Windows checkout may use CRLF). */
function isPublishedWordlist(buf) {
  return sha256hex(buf.toString("latin1").replaceAll("\r\n", "\n")) === WORDLIST_SHA256;
}

const WORDLIST = readFileSync(join(HERE, "bip39-english.txt"));
// A wordlist with a word changed or missing would let phrases through.
if (!isPublishedWordlist(WORDLIST))
  throw new Error(`${WORDLIST_PATH} is not the published BIP39 English wordlist: not scanning`);
const WORDS = WORDLIST.toString("utf8").split(/\r?\n/).filter(Boolean);
const INDEX = new Map(WORDS.map((w, i) => [w, i]));
const LENGTHS = [24, 21, 18, 15, 12];
const MAX_GAP = 160;
// What may stand between two words of a phrase as people write one down, up
// to 160 characters:
//   - whitespace and line breaks; quotes; `+` of string concatenation; the
//     `,` `;` `|` `[` `]` of a word array, a CSV row or a Markdown table;
//     backslashes with an escaped n/r/t (a Rust line continuation, a newline
//     inside a JSON string);
//   - list markers: a `-`, `*` or `•` with whitespace on both sides, and a
//     list number of one or two digits standing on its own (after whitespace,
//     `|` or `[`, before whitespace, `|` or `]`), optionally followed by `.`,
//     `)` or `:`.
// Other code punctuation (`:`, `.`, `&`, `(`, `=`, `{`) is not, so identifiers
// that happen to be wordlist words (`parent.account`, `change: &change`) do
// not form a phrase, and neither do kebab-case names (`border-image-repeat`,
// CSS lists are full of them) or a number glued to a word (`word1`). The other
// passes below look at the same text differently.
const GAP =
  /^(?:[\s"'`+,;|[\]\\]|(?<=\\)[nrt]|(?<=\s)[-*\u2022](?=\s)|(?<=[\s|[])\d{1,2}[.):]?(?=[\s|\]]))+$/;
// An extended private key of any Handshake network. hsd writes one as base58
// of 82 bytes that start with the network's keyPrefix.xprivkey
// (lib/protocol/networks.js), and those versions always read as xprv (main),
// tprv (testnet), rprv (regtest) and sprv (simnet). Matched by shape, not by
// checksum, so a key with a typo in it is still refused.
const EXTENDED_KEY =
  /(?<![1-9A-HJ-NP-Za-km-z])[xtrs]prv[1-9A-HJ-NP-Za-km-z]{100,112}(?![1-9A-HJ-NP-Za-km-z])/g;
// A WIF private key as hsd's KeyRing.toSecret writes it: base58 of 38 bytes,
// the network's keyPrefix.privkey byte (main 0x80, testnet 0xef, regtest 0x5a,
// simnet 0x64), the 32-byte key, a compression flag and a 4-byte
// double-SHA-256 checksum. Every base58 run of that length is decoded; the
// checksum keeps other base58 text out.
const WIF_VERSIONS = new Set([0x80, 0xef, 0x5a, 0x64]);
const BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BASE58_RUN = /(?<![1-9A-HJ-NP-Za-km-z])[1-9A-HJ-NP-Za-km-z]{50,53}(?![1-9A-HJ-NP-Za-km-z])/g;

// A field (whitespace-separated) that is decoration around at most a list
// number, and one that is decoration around a word. A field with anything else
// in it (`parent.account`, `word1`) ends a run of words.
const DECORATION = /^[^a-z0-9]*\d{0,2}[^a-z0-9]*$/;
const DECORATED_WORD = /^[^a-z0-9]*\d{0,2}[^a-z0-9]*([a-z]+)[^a-z0-9]*$/;
const HTML_TAG = /<\/?[a-z][^<>\n]*>/g;
// Twelve or more words joined by one separator, the same one throughout. Unlike
// a gap in GAP, this cannot turn `border-image-repeat border-image-slice` into
// a run: the words must form one unbroken token.
const JOINED = /(?<![a-z])[a-z]+ ?([-._/:]) ?[a-z]+(?: ?\1 ?[a-z]+){10,}/g;
const JSON_VALUE = /:\s*"([^"\n]*)"/g;

const BASE64_RUN = /[A-Za-z0-9+/]{64,}(?:\r?\n[A-Za-z0-9+/]+)*={0,2}/g;

// Hex is flagged only beside a word that says what it is: this repository is
// full of txids and hashes of the same lengths. Fewer than eight different
// digits is a placeholder (`1111…`), which real key material all but never is.
const HEX_RUN = /(?<![0-9a-f])[0-9a-f]{32,128}(?![0-9a-f])/g;
const HEX_LENGTHS = new Set([32, 64, 128]);
const HEX_KEYWORD = /seed|entropy|priv|secret|master|mnemonic/;
const HEX_KEYWORD_REACH = 40;

/** The words as GAP joins them; `null` where a run ends. */
function* gapJoined(lower) {
  let prev;
  for (const m of lower.matchAll(/[a-z]+/g)) {
    if (prev && !GAP.test(lower.slice(prev.index + prev[0].length, m.index))) yield null;
    prev = m;
    yield { w: m[0], at: m.index };
  }
}

/** The words of whitespace-separated fields, with their decoration taken off. */
function* decoratedFields(lower) {
  const text = lower.replace(HTML_TAG, (tag) => " ".repeat(tag.length));
  for (const m of text.matchAll(/\S+/g)) {
    if (DECORATION.test(m[0])) continue;
    const word = DECORATED_WORD.exec(m[0]);
    yield word ? { w: word[1], at: m.index + m[0].search(/[a-z]/) } : null;
  }
}

/** The words of each token joined throughout by one separator. */
function* joinedTokens(lower) {
  for (const m of lower.matchAll(JOINED)) {
    for (const t of m[0].matchAll(/[a-z]+/g)) yield { w: t[0], at: m.index + t.index };
  }
}

/** The string values of JSON objects, in order; the keys are not words of a phrase. */
function* jsonValues(lower) {
  for (const m of lower.matchAll(JSON_VALUE))
    yield { w: m[1], at: m.index + m[0].indexOf('"') + 1 };
}

/** The entropy behind `words`, or null when the checksum does not match. */
export function phraseEntropy(words) {
  if (!LENGTHS.includes(words.length) || words.some((w) => !INDEX.has(w))) return null;
  const bits = words.map((w) => INDEX.get(w).toString(2).padStart(11, "0")).join("");
  const csBits = words.length / 3;
  const entBits = bits.length - csBits;
  const entropy = Buffer.alloc(entBits / 8);
  for (let i = 0; i < entropy.length; i++) entropy[i] = parseInt(bits.slice(i * 8, i * 8 + 8), 2);
  const want = sha256(entropy)[0].toString(2).padStart(8, "0").slice(0, csBits);
  return want === bits.slice(entBits) ? entropy : null;
}

function base58Decode(s) {
  let n = 0n;
  for (const c of s) n = n * 58n + BigInt(BASE58.indexOf(c));
  const hex = n.toString(16);
  const body = n === 0n ? Buffer.alloc(0) : Buffer.from(hex.length % 2 ? `0${hex}` : hex, "hex");
  return Buffer.concat([Buffer.alloc(s.match(/^1*/)[0].length), body]);
}

/** Whether `s` is a WIF private key of a Handshake network. */
function isWif(s) {
  const b = base58Decode(s);
  if (b.length !== 38 || !WIF_VERSIONS.has(b[0])) return false;
  return sha256(sha256(b.subarray(0, 34)))
    .subarray(0, 4)
    .equals(b.subarray(34));
}

function allowedHashes() {
  const file = join(HERE, "secret-scan-allow.txt");
  if (!existsSync(file)) return new Set();
  return new Set(
    readFileSync(file, "utf8")
      .split("\n")
      .map((l) => l.replace(/#.*/, "").trim())
      .filter(Boolean),
  );
}

/** A published BIP39 test vector: the same byte repeated (0x00, 0x7f, 0x80, 0xff). */
function isPublicTestPhrase(entropy) {
  return [0x00, 0x7f, 0x80, 0xff].some((b) => entropy.every((x) => x === b));
}

const lineAt = (s, pos) => s.slice(0, pos).split("\n").length;

/**
 * Secrets in one text: `{ line, kind, words?, digest }`. `digest` is the
 * SHA-256 of the phrase's words or of the key, for telling one secret from
 * another; never print it with the finding.
 */
function findSecrets(text, allowed, decodeBase64 = true) {
  // Zero-width and other format characters are dropped, so a word with one
  // inside still reads as the word.
  const clean = text.replace(/\p{Cf}/gu, "");
  // Escaped line breaks (`\\n`, `\\\\n` in JSON transcripts) read as a gap,
  // not as a word "n": blanked to spaces of the same length.
  const lower = clean.toLowerCase().replace(/\\+[nrt]/g, (m) => " ".repeat(m.length));
  const phrases = [];
  const flush = (run) => {
    if (run.length < 12) return;
    // One finding per run of words: the longest valid phrase in it. Shorter
    // windows of a 24-word phrase may pass the checksum too; they are the
    // same secret. A window of fewer than n/2 different words is not a
    // phrase anyone generated (the odds of a random phrase repeating that
    // much are below 1 in 10^17) but a boolean array (`true, false, …`) or
    // a repeated word in prose, which pass the checksum 1 time in 16.
    for (const n of LENGTHS) {
      for (let i = 0; i + n <= run.length; i++) {
        const words = run.slice(i, i + n).map((t) => t.w);
        if (new Set(words).size < n / 2) continue;
        const entropy = phraseEntropy(words);
        if (!entropy || isPublicTestPhrase(entropy)) continue;
        const digest = sha256hex(words.join(" "));
        if (allowed.has(digest)) continue;
        const from = run[i].at;
        const to = run[i + n - 1].at;
        const phrase = {
          from,
          to,
          line: lineAt(lower, from),
          kind: "recovery phrase",
          words: n,
          digest,
        };
        // The passes read the same text: the same words are one finding, the longest.
        const same = phrases.find((p) => from <= p.to && p.from <= to);
        if (!same) phrases.push(phrase);
        else if (same.words < n) Object.assign(same, phrase);
        return;
      }
    }
  };
  for (const tokens of [gapJoined, decoratedFields, joinedTokens, jsonValues]) {
    let run = [];
    for (const t of tokens(lower)) {
      if (!t || !INDEX.has(t.w)) {
        flush(run);
        run = [];
        continue;
      }
      const prev = run[run.length - 1];
      if (prev && t.at - prev.at - prev.w.length > MAX_GAP) {
        flush(run);
        run = [];
      }
      run.push(t);
    }
    flush(run);
  }
  const findings = phrases.map(({ from, to, ...f }) => f);
  const key = (secret, at, kind) => {
    const digest = sha256hex(secret);
    if (!allowed.has(digest)) findings.push({ line: lineAt(clean, at), kind, digest });
  };
  for (const m of clean.matchAll(EXTENDED_KEY)) key(m[0], m.index, "extended private key");
  for (const m of clean.matchAll(BASE58_RUN))
    if (isWif(m[0])) key(m[0], m.index, "private key (WIF)");
  for (const m of lower.matchAll(HEX_RUN)) {
    if (!HEX_LENGTHS.has(m[0].length) || new Set(m[0]).size < 8) continue;
    const around = lower.slice(
      Math.max(0, m.index - HEX_KEYWORD_REACH),
      m.index + m[0].length + HEX_KEYWORD_REACH,
    );
    if (HEX_KEYWORD.test(around)) key(m[0], m.index, "seed or private key (hex)");
  }
  if (decodeBase64)
    for (const m of clean.matchAll(BASE64_RUN)) {
      const decoded = Buffer.from(m[0], "base64").toString("latin1");
      for (const f of findSecrets(decoded, allowed, false))
        findings.push({ ...f, line: lineAt(clean, m.index), kind: `${f.kind} in base64` });
    }
  return findings;
}

/** Findings in one text: `{ line, kind, words? }`, never the secret itself. */
export function scanText(text, allowed = new Set()) {
  return findSecrets(text, allowed).map(({ digest, ...f }) => f);
}

/** A file's text: UTF-8, or UTF-16 after a byte-order mark; null for a binary file. */
export function decodeText(buf) {
  if (buf[0] === 0xff && buf[1] === 0xfe) return buf.subarray(2).toString("utf16le");
  if (buf[0] === 0xfe && buf[1] === 0xff)
    return Buffer.from(buf.subarray(2, buf.length - (buf.length % 2)))
      .swap16()
      .toString("utf16le");
  if (buf.subarray(0, 8000).includes(0)) return null;
  return buf.toString("utf8");
}

/**
 * Secrets in the content `buf` of the file at repository path `path`:
 * `{ findings, binary }`, findings as `findSecrets` gives them.
 */
export function scanFile(path, buf, allowed = new Set()) {
  const named = findSecrets(path, allowed).map((f) => ({ ...f, kind: `file name: ${f.kind}` }));
  if (path === WORDLIST_PATH && isPublishedWordlist(buf)) return { findings: named, binary: false };
  const text = decodeText(buf);
  if (text === null) return { findings: named, binary: true };
  if (buf.length > MAX_TEXT_BYTES)
    return {
      findings: [
        ...named,
        { line: 1, kind: "text file too large to scan", digest: `large:${sha256hex(buf)}` },
      ],
      binary: false,
    };
  return { findings: [...named, ...findSecrets(text, allowed)], binary: false };
}

/** `path` for the report; a name that holds a secret must not be printed. */
const shownPath = (path, allowed) =>
  findSecrets(path, allowed).length ? "(a file whose name holds a secret)" : path;

const git = (cwd, args, input) =>
  execFileSync("git", args, { cwd, input, maxBuffer: 1 << 30, stdio: ["pipe", "pipe", "pipe"] });
const gitText = (cwd, args) => git(cwd, args).toString("utf8");
const ZERO = /^0+$/;

/** Objects by id, read with one `git cat-file --batch`. */
function readObjects(cwd, ids) {
  const out = git(cwd, ["cat-file", "--batch"], ids.join("\n") + "\n");
  const objects = new Map();
  let at = 0;
  for (const id of ids) {
    const nl = out.indexOf(10, at);
    const [, type, size] = out.subarray(at, nl).toString("utf8").split(" ");
    if (type === "missing" || size === undefined) throw new Error(`git object ${id} is missing`);
    objects.set(id, out.subarray(nl + 1, nl + 1 + Number(size)));
    at = nl + 1 + Number(size) + 1;
  }
  return objects;
}

/** The changes in git's raw diff (`--raw -z --no-abbrev`) that put content at a path. */
function changedBlobs(raw) {
  const parts = raw.split("\0");
  const changes = [];
  for (let i = 0; i < parts.length;) {
    const head = parts[i++];
    if (!head.startsWith(":")) continue;
    const [oldMode, newMode, oldId, newId, status] = head.slice(1).split(" ");
    const oldPath = parts[i++];
    const path = status[0] === "R" || status[0] === "C" ? parts[i++] : oldPath;
    // D removes content; a gitlink (160000) is another repository's commit.
    if (!"ACMRT".includes(status[0]) || newMode === "160000" || ZERO.test(newId)) continue;
    const hadBlob = !ZERO.test(oldId) && oldMode !== "160000";
    changes.push({ path, oldPath, oldId: hadBlob ? oldId : null, newId });
  }
  return changes;
}

/** Findings the changes add: those in each new blob that its old blob did not already hold. */
function scanChanges(cwd, changes, where, allowed, report) {
  if (!changes.length) return;
  const ids = [...new Set(changes.flatMap((c) => (c.oldId ? [c.oldId, c.newId] : [c.newId])))];
  const objects = readObjects(cwd, ids);
  for (const c of changes) {
    const now = scanFile(c.path, objects.get(c.newId), allowed);
    report.scanned++;
    if (now.binary) report.binary++;
    const before = c.oldId ? scanFile(c.oldPath, objects.get(c.oldId), allowed).findings : [];
    const had = new Set(before.map((f) => f.digest));
    for (const f of now.findings)
      if (!had.has(f.digest)) report.add(where(shownPath(c.path, allowed)), f);
  }
}

function hasCommit(cwd, id) {
  try {
    git(cwd, ["cat-file", "-e", `${id}^{commit}`]);
    return true;
  } catch {
    return false;
  }
}

/**
 * The commits a push sends, from git's pre-push lines: those REMOTE's branches
 * do not have. Tag objects on the way to a commit, and the ref names, are
 * scanned here; a ref that ends at a blob or a tree is refused, since no commit
 * walk reaches what it publishes.
 */
function pushedCommits(cwd, remote, updates, allowed, report) {
  const commits = new Set();
  for (const line of updates.split("\n")) {
    const [localRef, localId, remoteRef, remoteId] = line.trim().split(/\s+/);
    // A deleted ref sends no commits.
    if (!localId || ZERO.test(localId)) continue;
    for (const f of findSecrets(`${localRef}\n${remoteRef}`, allowed)) report.add("a ref name", f);
    let id = localId;
    let type = gitText(cwd, ["cat-file", "-t", id]).trim();
    while (type === "tag") {
      const tag = readObjects(cwd, [id]).get(id).toString("utf8");
      for (const f of findSecrets(tag, allowed)) report.add(`${id.slice(0, 10)} tag`, f);
      id = tag.match(/^object (\S+)/)[1];
      type = gitText(cwd, ["cat-file", "-t", id]).trim();
    }
    if (type !== "commit") {
      report.add(`${id.slice(0, 10)}`, {
        line: 1,
        kind: `a ref to a ${type}, which is not scanned`,
      });
      continue;
    }
    const known = remote ? [`--remotes=${remote}`] : [];
    if (remoteId && !ZERO.test(remoteId) && hasCommit(cwd, remoteId)) known.push(remoteId);
    for (const c of gitText(cwd, ["rev-list", id, "--not", ...known]).split("\n"))
      if (c) commits.add(c);
  }
  return [...commits];
}

function scanPush(cwd, remote, updates, allowed, report) {
  for (const id of pushedCommits(cwd, remote, updates, allowed, report)) {
    const commit = readObjects(cwd, [id]).get(id).toString("utf8");
    const split = commit.indexOf("\n\n");
    const header = split < 0 ? commit : commit.slice(0, split);
    const parents = header
      .split("\n")
      .filter((l) => l.startsWith("parent "))
      .map((l) => l.slice(7));
    const short = id.slice(0, 10);
    report.scanned++;
    for (const f of findSecrets(split < 0 ? "" : commit.slice(split + 2), allowed))
      report.add(`${short} commit message`, f);
    for (const f of findSecrets(header, allowed)) report.add(`${short} commit author`, f);
    // A merge is compared with its first parent: what it brings to the branch.
    const diff = parents.length
      ? ["diff-tree", "-r", "-z", "--no-abbrev", "-M", parents[0], id]
      : ["diff-tree", "--root", "--no-commit-id", "-r", "-z", "--no-abbrev", "-M", id];
    const changes = changedBlobs(gitText(cwd, diff));
    scanChanges(cwd, changes, (path) => `${short} ${path}`, allowed, report);
  }
}

function scanRange(cwd, [before, head, main], allowed, report) {
  const base = hasCommit(cwd, before) ? before : gitText(cwd, ["merge-base", head, main]).trim();
  scanPush(cwd, "", `range ${head} range ${base}\n`, allowed, report);
}

function scanStaged(cwd, allowed, report) {
  const raw = gitText(cwd, ["diff", "--cached", "--raw", "-z", "--no-abbrev", "-M"]);
  scanChanges(cwd, changedBlobs(raw), (path) => path, allowed, report);
}

/** Every file git tracks under `root`, read from the working tree. */
function scanTree(root, allowed, report) {
  const files = gitText(root, ["ls-files", "-z"]).split("\0").filter(Boolean);
  for (const path of files) {
    const file = join(root, path);
    if (!existsSync(file) || !statSync(file).isFile()) continue;
    const { findings, binary } = scanFile(path, readFileSync(file), allowed);
    report.scanned++;
    if (binary) report.binary++;
    for (const f of findings) report.add(shownPath(path, allowed), f);
  }
}

function scanPaths(paths, allowed, report) {
  let root = process.cwd();
  try {
    root = gitText(root, ["rev-parse", "--show-toplevel"]).trim();
  } catch {
    // Not in a git checkout: paths stay relative to the current directory.
  }
  for (const file of paths) {
    const path = relative(root, resolve(file)).split(sep).join("/");
    const { findings, binary } = scanFile(path, readFileSync(file), allowed);
    report.scanned++;
    if (binary) report.binary++;
    for (const f of findings) report.add(file, f);
  }
}

function newReport() {
  const report = { lines: [], scanned: 0, binary: 0 };
  report.add = (where, f) =>
    report.lines.push(`${where}:${f.line}: ${f.kind}${f.words ? ` (${f.words} words)` : ""}`);
  return report;
}

const REFUSALS = {
  staged: (n) =>
    `Commit refused: ${n} wallet secret(s) in the staged files. Take them out of those files and stage them again. ` +
    "A test uses the public BIP39 test phrase (abandon … art) or generates one at run time.",
  push: (n) =>
    `Push refused, nothing was sent: ${n} wallet secret(s) in the commits being pushed. ` +
    "Rewrite those commits without them first: a later commit that deletes a secret does not unpublish the one that added it.",
  tree: (n, counted) =>
    `${n} wallet secret(s) found in ${counted}. A real recovery phrase or key must never be committed: ` +
    "use the public BIP39 test phrase (abandon … art) or generate one at run time. " +
    "If a commit holding it was ever pushed, it is public: move its funds now. Deleting it or force-pushing does not unpublish it.",
};

function main(argv) {
  const allowed = allowedHashes();
  const report = newReport();
  const cwd = process.cwd();
  let mode = "tree";
  if (argv[0] === "--staged") {
    mode = "staged";
    scanStaged(cwd, allowed, report);
  } else if (argv[0] === "--push") {
    mode = "push";
    scanPush(cwd, argv[1], readFileSync(0, "utf8"), allowed, report);
  } else if (argv[0] === "--range") {
    mode = "range";
    scanRange(cwd, argv.slice(1), allowed, report);
  } else if (argv.length) {
    scanPaths(argv, allowed, report);
  } else {
    scanTree(gitText(cwd, ["rev-parse", "--show-toplevel"]).trim(), allowed, report);
  }
  const counted = `${report.scanned} ${mode === "push" || mode === "range" ? "commits and files" : "files"}`;
  if (report.lines.length) {
    for (const l of report.lines) console.error(l);
    console.error(`\n${(REFUSALS[mode] ?? REFUSALS.tree)(report.lines.length, counted)}`);
    return 1;
  }
  console.log(
    `OK: no recovery phrase or private key in ${counted}` +
      (report.binary ? ` (${report.binary} binary, not scanned).` : "."),
  );
  return 0;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (e) {
    console.error(`check-secrets: ${e.message}`);
    process.exitCode = 2;
  }
}
