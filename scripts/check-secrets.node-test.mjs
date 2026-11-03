// Tests for check-secrets.mjs and the git hooks that run it, with
// `node --test scripts/check-secrets.node-test.mjs` (part of `pnpm lint:secrets`).
// No real phrase or key appears here: every flagged one is generated from
// random entropy at run time.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  MAX_TEXT_BYTES,
  WORDLIST_PATH,
  decodeText,
  phraseEntropy,
  scanFile,
  scanText,
} from "./check-secrets.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const WORDLIST = readFileSync(join(ROOT, WORDLIST_PATH));
const WORDS = WORDLIST.toString("utf8").split("\n").filter(Boolean);
const sha256 = (data) => createHash("sha256").update(data).digest();

/** A valid BIP39 phrase from `entropy` (16–32 bytes). */
function phrase(entropy) {
  const cs = entropy.length / 4;
  const bits =
    [...entropy].map((b) => b.toString(2).padStart(8, "0")).join("") +
    sha256(entropy)[0].toString(2).padStart(8, "0").slice(0, cs);
  return Array.from(
    { length: bits.length / 11 },
    (_, i) => WORDS[parseInt(bits.slice(i * 11, i * 11 + 11), 2)],
  );
}

/** `p` with its checksum broken: the last word's lowest bit, a checksum bit, flipped. */
function badChecksum(p) {
  return [...p.slice(0, -1), WORDS[WORDS.indexOf(p[p.length - 1]) ^ 1]];
}

/** The gate's output holds neither two consecutive words of `p` nor the hash of its words. */
function assertNotPrinted(out, p) {
  for (let i = 0; i + 1 < p.length; i++)
    assert.doesNotMatch(out, new RegExp(`\\b${p[i]}\\W+${p[i + 1]}\\b`, "i"));
  assert.equal(out.includes(sha256(p.join(" ")).toString("hex")), false);
}

const BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/** Base58 of `payload` followed by its 4-byte double-SHA-256 checksum, as hsd writes keys. */
function base58check(payload) {
  const bytes = Buffer.concat([payload, sha256(sha256(payload)).subarray(0, 4)]);
  let n = BigInt(`0x${bytes.toString("hex")}`);
  let s = "";
  while (n > 0n) {
    s = BASE58[Number(n % 58n)] + s;
    n /= 58n;
  }
  for (const b of bytes) {
    if (b !== 0) break;
    s = `1${s}`;
  }
  return s;
}

/** A BIP32 extended key with `version` and the 74 bytes after it set to `body` (random by default). */
function extendedKey(version, body = randomBytes(74)) {
  const head = Buffer.alloc(4);
  head.writeUInt32BE(version);
  return base58check(Buffer.concat([head, body]));
}

// hsd 8.0.0 lib/protocol/networks.js keyPrefix of each network.
const NETWORKS = [
  {
    name: "main",
    xprivkey: 0x0488ade4,
    xprv: "xprv",
    xpubkey: 0x0488b21e,
    xpub: "xpub",
    privkey: 0x80,
  },
  {
    name: "testnet",
    xprivkey: 0x04358394,
    xprv: "tprv",
    xpubkey: 0x043587cf,
    xpub: "tpub",
    privkey: 0xef,
  },
  {
    name: "regtest",
    xprivkey: 0xeab404c7,
    xprv: "rprv",
    xpubkey: 0xeab4fa05,
    xpub: "rpub",
    privkey: 0x5a,
  },
  {
    name: "simnet",
    xprivkey: 0x0420b900,
    xprv: "sprv",
    xpubkey: 0x0420bd3a,
    xpub: "spub",
    privkey: 0x64,
  },
];

// --- Phrases as they are written down -------------------------------------

test("a random 24-word phrase in a JSON field is found, without printing it", () => {
  const p = phrase(randomBytes(32));
  const f = scanText(`{\n  "mnemonic": "${p.join(" ")}"\n}`);
  assert.deepEqual(f, [{ line: 2, kind: "recovery phrase", words: 24 }]);
  assertNotPrinted(JSON.stringify(f), p);
});

test("a 12-word phrase wrapped across Rust line continuations is found", () => {
  const p = phrase(randomBytes(16));
  const text = `let m = "${p.slice(0, 6).join(" ")} \\\n             ${p.slice(6).join(" ")}";`;
  assert.equal(scanText(text).length, 1);
});

test("a phrase split by JS string concatenation and JSON-escaped newlines is found", () => {
  const p = phrase(randomBytes(32));
  const js = `const M =\n  "${p.slice(0, 10).join(" ")} " +\n  "${p.slice(10).join(" ")}";`;
  assert.equal(scanText(js).length, 1);
  const escaped = `"${p.slice(0, 12).join(" ")} \\\\\\n    ${p.slice(12).join(" ")}"`;
  assert.equal(scanText(escaped).length, 1);
});

const capital = (w) => w[0].toUpperCase() + w.slice(1);

// Each format: the text it makes of a phrase, and the line its first word is on.
const FORMATS = {
  "a numbered list `1. word`": (p) => [p.map((w, i) => `${i + 1}. ${w}`).join("\n"), 1],
  "a numbered list `1) word`": (p) => [p.map((w, i) => `${i + 1}) ${w}`).join("\n"), 1],
  "a numbered list `1 word`": (p) => [p.map((w, i) => `${i + 1} ${w}`).join("\n"), 1],
  "a numbered list `1.word`": (p) => [p.map((w, i) => `${i + 1}.${w}`).join("\n"), 1],
  "a numbered list `1)word`": (p) => [p.map((w, i) => `${i + 1})${w}`).join("\n"), 1],
  "a numbered list `01: word`": (p) => [
    p.map((w, i) => `${String(i + 1).padStart(2, "0")}: ${w}`).join("\n"),
    1,
  ],
  "a numbered list `[1] word`": (p) => [p.map((w, i) => `[${i + 1}] ${w}`).join("\n"), 1],
  "a bullet list `- word`": (p) => [p.map((w) => `- ${w}`).join("\n"), 1],
  "a bullet list `* word`": (p) => [p.map((w) => `* ${w}`).join("\n"), 1],
  "a bullet list `• word`": (p) => [p.map((w) => `• ${w}`).join("\n"), 1],
  "an indented YAML list": (p) => [`mnemonic:\n${p.map((w) => `  - ${w}`).join("\n")}`, 2],
  "a Markdown table `| 1 | word |`": (p) => [
    `| No. | Mnemonic |\n| --- | -------- |\n${p.map((w, i) => `| ${i + 1} | ${w} |`).join("\n")}`,
    3,
  ],
  "a comma-separated line": (p) => [p.join(","), 1],
  "a semicolon-separated line": (p) => [p.join(";"), 1],
  "a tab-separated line": (p) => [p.join("\t"), 1],
  "a pipe-separated line": (p) => [p.join("|"), 1],
  "a ` | `-separated line": (p) => [p.join(" | "), 1],
  "a quoted JSON array": (p) => [JSON.stringify(p), 1],
  "a pretty-printed JSON array": (p) => [JSON.stringify({ mnemonic: p }, null, 2), 3],
  "a single-quoted list": (p) => [`MNEMONIC = [${p.map((w) => `'${w}'`).join(", ")}]`, 1],
  "Title Case": (p) => [p.map(capital).join(" "), 1],
  "upper case": (p) => [p.join(" ").toUpperCase(), 1],
  "mixed case": (p) => [p.map((w, i) => (i % 2 ? w.toUpperCase() : capital(w))).join(" "), 1],
  "a commented list `// word`": (p) => [p.map((w) => `// ${w}`).join("\n"), 1],
  "a commented list `# word`": (p) => [p.map((w) => `# ${w}`).join("\n"), 1],
  "a quoted block `> word`": (p) => [p.map((w) => `> ${w}`).join("\n"), 1],
  "a comment wrapped over lines": (p) => [
    `// ${p.slice(0, 6).join(" ")}\n// ${p.slice(6).join(" ")}`,
    1,
  ],
  "a numbered list `(1) word`": (p) => [p.map((w, i) => `(${i + 1}) ${w}`).join("\n"), 1],
  "a numbered list `#1 word`": (p) => [p.map((w, i) => `#${i + 1} ${w}`).join("\n"), 1],
  "a numbered list `1. **word**`": (p) => [p.map((w, i) => `${i + 1}. **${w}**`).join("\n"), 1],
  "a numbered list `1:word`": (p) => [p.map((w, i) => `${i + 1}:${w}`).join("\n"), 1],
  "a numbered list `1-word`": (p) => [p.map((w, i) => `${i + 1}-${w}`).join("\n"), 1],
  "a numbered list `1 => word`": (p) => [p.map((w, i) => `${i + 1} => ${w}`).join("\n"), 1],
  "numbers inline `1:word 2:word`": (p) => [p.map((w, i) => `${i + 1}:${w}`).join(" "), 1],
  "an HTML list": (p) => [`<ol>\n${p.map((w) => `  <li>${w}</li>`).join("\n")}\n</ol>`, 2],
  "words joined by `-`": (p) => [p.join("-"), 1],
  "words joined by `.`": (p) => [p.join("."), 1],
  "words joined by `_`": (p) => [p.join("_"), 1],
  "words joined by `/`": (p) => [p.join("/"), 1],
  "words joined by `:`": (p) => [p.join(":"), 1],
  "words joined by ` / `": (p) => [p.join(" / "), 1],
  "a JSON object per word": (p) => [JSON.stringify(p.map((w, i) => ({ i, w }))), 1],
  "a JSON map of words": (p) => [
    JSON.stringify(Object.fromEntries(p.map((w, i) => [i + 1, w]))),
    1,
  ],
  "a pretty-printed JSON map of words": (p) => [
    JSON.stringify(Object.fromEntries(p.map((w, i) => [i + 1, w])), null, 2),
    2,
  ],
  "words with zero-width characters inside": (p) => [
    p.map((w) => `${w[0]}​${w.slice(1, 3)}­${w.slice(3)}`).join(" "),
    1,
  ],
};

for (const [name, render] of Object.entries(FORMATS)) {
  test(`a 24-word phrase written as ${name} is found`, () => {
    const p = phrase(randomBytes(32));
    const [text, line] = render(p);
    assert.deepEqual(scanText(text), [{ line, kind: "recovery phrase", words: 24 }]);
  });

  test(`a 12-word list written as ${name} with a wrong checksum passes`, () => {
    const p = badChecksum(phrase(randomBytes(16)));
    assert.equal(phraseEntropy(p), null);
    assert.deepEqual(scanText(render(p)[0]), []);
  });
}

test("a numbered list of 12 is found as 12 words", () => {
  const p = phrase(randomBytes(16));
  const text = `Backup:\n${p.map((w, i) => `${i + 1}. ${w}`).join("\n")}\n`;
  assert.deepEqual(scanText(text), [{ line: 2, kind: "recovery phrase", words: 12 }]);
});

// --- What must not read as a phrase ----------------------------------------

test("the public BIP39 test phrases are allowed", () => {
  const zero24 = [...Array(23).fill("abandon"), "art"].join(" ");
  const zero12 = [...Array(11).fill("abandon"), "about"].join(" ");
  assert.notEqual(phraseEntropy(zero24.split(" ")), null);
  assert.deepEqual(scanText(`MN = "${zero24}"; M12 = "${zero12}"`), []);
  assert.deepEqual(scanText(phrase(Buffer.alloc(32, 0xff)).join(" ")), []);
  assert.deepEqual(scanText(phrase(Buffer.alloc(16, 0x7f)).join("\n")), []);
});

test("a phrase listed by its hash is allowed", () => {
  const p = phrase(randomBytes(16)).join(" ");
  const hash = createHash("sha256").update(p).digest("hex");
  assert.equal(scanText(p).length, 1);
  assert.deepEqual(scanText(p, new Set([hash])), []);
});

test("a run of wordlist words that is not a phrase passes", () => {
  // Thirty wordlist words with no valid phrase in them (all-zero entropy with a
  // wrong checksum), and code whose identifiers are wordlist words.
  assert.deepEqual(scanText(Array(30).fill("abandon").join(" ")), []);
  const code = [
    "    ChildRequest {",
    "        network: parent.network,",
    "        account: parent.account,",
    "        change: &parent.change,",
    "        change_address: &parent.change_address,",
    "        entry,",
    "        target_rate: target.0,",
    "    }",
  ].join("\n");
  assert.deepEqual(scanText(code), []);
});

test("a boolean array that passes the checksum is not a phrase", () => {
  // `true` and `false` are both wordlist words; 1 array in 16 passes the checksum.
  let words;
  for (let mask = 0; !words; mask++) {
    const w = Array.from({ length: 12 }, (_, i) => ((mask >> i) & 1 ? "true" : "false"));
    if (phraseEntropy(w)) words = w;
  }
  assert.deepEqual(scanText(`let bits = [${words.join(", ")}];`), []);
});

test("one word repeated is not a phrase, even when it passes the checksum", () => {
  const word = WORDS.find(
    (w) => phraseEntropy(Array(12).fill(w)) && w !== "abandon" && w !== "zoo",
  );
  assert.ok(word);
  assert.deepEqual(scanText(`this is ${Array(12).fill(word).join(" ")} long`), []);
});

test("words joined in separate tokens, as in CSS class names, are not a phrase", () => {
  const p = phrase(randomBytes(16));
  assert.deepEqual(scanText(`class="${p.slice(0, 6).join("-")} ${p.slice(6).join("-")}"`), []);
});

test("words joined by two different separators are not a phrase", () => {
  const p = phrase(randomBytes(16));
  assert.deepEqual(scanText(`${p.slice(0, 6).join("-")}.${p.slice(6).join("-")}`), []);
  assert.deepEqual(scanText(`${p.slice(0, 11).join("/")}-${p[11]}`), []);
});

test("identifiers in code that are wordlist words are not a phrase, however they are decorated", () => {
  const p = phrase(randomBytes(16));
  assert.deepEqual(scanText(p.map((w) => `  parent.${w},`).join("\n")), []);
  assert.deepEqual(scanText(p.map((w) => `  ${w}1,`).join("\n")), []);
  assert.deepEqual(scanText(p.map((w, i) => `${i < 6 ? `# ${w}` : `# ${w} x`}`).join("\n")), []);
});

test("a field that is no word ends a run of words", () => {
  const p = phrase(randomBytes(16));
  const text = `# ${p.slice(0, 6).join(" ")} parent.account ${p.slice(6).join(" ")}`;
  assert.deepEqual(scanText(text), []);
});

test("JSON keys are not words of a phrase, and a value that is no word ends a run", () => {
  const p = phrase(randomBytes(16));
  assert.deepEqual(scanText(JSON.stringify(Object.fromEntries(p.map((w) => [w, 1])))), []);
  const broken = p.map((w, i) => ({ w: i === 6 ? "not a word" : w }));
  assert.deepEqual(scanText(JSON.stringify(broken)), []);
});

test("the same phrase seen by several readings is one finding", () => {
  const p = phrase(randomBytes(32));
  assert.equal(scanText(`"${p.join(" ")}"`).length, 1);
  assert.equal(scanText(`${p.join(" ")}\n\n${p.join("-")}`).length, 2);
});

test("a number glued to a word does not join words", () => {
  const p = phrase(randomBytes(16));
  assert.deepEqual(scanText(p.map((w, i) => `${w}${i}`).join(" ")), []);
});

test("words more than 160 characters apart are not one phrase", () => {
  const p = phrase(randomBytes(16));
  assert.equal(
    scanText(`${p.slice(0, 6).join(" ")}${" ".repeat(160)}${p.slice(6).join(" ")}`).length,
    1,
  );
  assert.deepEqual(
    scanText(`${p.slice(0, 6).join(" ")}${" ".repeat(161)}${p.slice(6).join(" ")}`),
    [],
  );
});

test("eleven words are never a phrase", () => {
  assert.deepEqual(scanText(phrase(randomBytes(16)).slice(0, 11).join(" ")), []);
});

// --- Keys -------------------------------------------------------------------

for (const n of NETWORKS) {
  test(`an extended private key of ${n.name} (${n.xprv}) is found, its public key is not`, () => {
    for (const body of [Buffer.alloc(74, 0), Buffer.alloc(74, 0xff)]) {
      assert.ok(extendedKey(n.xprivkey, body).startsWith(n.xprv));
      assert.ok(extendedKey(n.xpubkey, body).startsWith(n.xpub));
    }
    const xprv = extendedKey(n.xprivkey);
    assert.deepEqual(scanText(`key: ${xprv}`), [{ line: 1, kind: "extended private key" }]);
    assert.deepEqual(scanText(`key: ${extendedKey(n.xpubkey)}`), []);
  });

  test(`a WIF private key of ${n.name} is found, a mistyped one is not`, () => {
    const wif = base58check(
      Buffer.concat([Buffer.from([n.privkey]), randomBytes(32), Buffer.from([1])]),
    );
    assert.deepEqual(scanText(`\nsecret = "${wif}"`), [{ line: 2, kind: "private key (WIF)" }]);
    const at = 20;
    const other = BASE58[(BASE58.indexOf(wif[at]) + 1) % 58];
    assert.deepEqual(scanText(wif.slice(0, at) + other + wif.slice(at + 1)), []);
  });
}

test("an extended private key next to an underscore is found", () => {
  const key = extendedKey(NETWORKS[0].xprivkey);
  assert.equal(scanText(`_${key}_`).length, 1);
  assert.equal(scanText(`KEY_${key}`).length, 1);
});

test("an extended private key with a typo is still found", () => {
  const xprv = "xprv" + "9s21ZrQH143K".padEnd(107, "a");
  assert.deepEqual(scanText(`key: ${xprv}`), [{ line: 1, kind: "extended private key" }]);
  assert.deepEqual(
    scanText(
      "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8",
    ),
    [],
  );
});

test("a WIF-length base58 string with another version byte is not a key", () => {
  const notWif = base58check(
    Buffer.concat([Buffer.from([0x01]), randomBytes(32), Buffer.from([1])]),
  );
  assert.deepEqual(scanText(notWif), []);
});

// --- Seeds as hex, phrases as base64 -----------------------------------------

for (const bytes of [16, 32, 64]) {
  const hex = () => randomBytes(bytes).toString("hex");
  test(`${bytes} bytes of hex beside a key word are found`, () => {
    for (const word of ["seed", "entropy", "private_key", "secret", "master", "mnemonic"]) {
      const h = hex();
      assert.deepEqual(scanText(`let ${word} = hex!("${h}");`), [
        { line: 1, kind: "seed or private key (hex)" },
      ]);
      assert.equal(scanText(`"${h}" // the ${word}`).length, 1);
      assert.equal(scanText(`let ${word} = "${h.toUpperCase()}";`).length, 1);
    }
  });

  test(`${bytes} bytes of hex with no key word beside them are not a secret`, () => {
    assert.deepEqual(scanText(`txid = "${hex()}"`), []);
  });
}

test("hex of another length beside a key word is not a secret", () => {
  assert.deepEqual(scanText(`seed = "${randomBytes(20).toString("hex")}"`), []);
  assert.deepEqual(scanText(`seed = "${randomBytes(33).toString("hex")}"`), []);
});

test("a key word more than 40 characters from the hex does not count", () => {
  const h = randomBytes(32).toString("hex");
  assert.equal(scanText(`seed${" ".repeat(36)}${h}`).length, 1);
  assert.deepEqual(scanText(`seed${" ".repeat(37)}${h}`), []);
  assert.equal(scanText(`${h}${" ".repeat(36)}seed`).length, 1);
  assert.deepEqual(scanText(`${h}${" ".repeat(37)}seed`), []);
});

test("a placeholder of few different digits beside a key word is not a secret", () => {
  assert.deepEqual(scanText(`"privateKey": "${"1".repeat(64)}"`), []);
  assert.deepEqual(scanText(`seed = "${"deadbeef".repeat(8)}"`), []);
});

test("hex listed by its hash is allowed", () => {
  const h = randomBytes(32).toString("hex");
  const allowed = new Set([sha256(h).toString("hex")]);
  assert.deepEqual(scanText(`seed = "${h}"`, allowed), []);
});

test("a phrase in base64 is found, without printing it", () => {
  const p = phrase(randomBytes(16));
  const text = `\nblob: ${Buffer.from(p.join(" ")).toString("base64")}\n`;
  const found = scanText(text);
  assert.deepEqual(found, [{ line: 2, kind: "recovery phrase in base64", words: 12 }]);
  assertNotPrinted(JSON.stringify(found), p);
});

test("a phrase in base64 wrapped at 76 columns, or inside encoded JSON, is found", () => {
  const p = phrase(randomBytes(32));
  const b64 = Buffer.from(`{"m":"${p.join(" ")}"}`).toString("base64");
  assert.equal(scanText(b64).length, 1);
  assert.equal(scanText(b64.replace(/.{76}/g, "$&\n")).length, 1);
});

test("a phrase among binary bytes in base64 is found", () => {
  const p = phrase(randomBytes(16));
  const blob = Buffer.concat([randomBytes(30), Buffer.from(` ${p.join(" ")} `), randomBytes(30)]);
  assert.equal(scanText(blob.toString("base64")).length, 1);
});

test("base64 of binary or of a wrong checksum is not a secret", () => {
  assert.deepEqual(scanText(randomBytes(512).toString("base64")), []);
  const bad = badChecksum(phrase(randomBytes(16)));
  assert.deepEqual(scanText(Buffer.from(bad.join(" ")).toString("base64")), []);
});

test("the public BIP32 and BIP39 hex vectors this repository tests with are allowed", () => {
  const allow = readFileSync(join(ROOT, "scripts/secret-scan-allow.txt"), "utf8");
  const seed = "000102030405060708090a0b0c0d0e0f";
  assert.ok(allow.includes(sha256(seed).toString("hex")));
  for (const line of allow.split("\n"))
    assert.doesNotMatch(line.replace(/^[0-9a-f]{64} /, ""), /[0-9a-f]{32}/);
});

// --- Files ------------------------------------------------------------------

test("a UTF-16 file with a byte-order mark is read as text", () => {
  const p = phrase(randomBytes(32));
  const le = Buffer.concat([
    Buffer.from([0xff, 0xfe]),
    Buffer.from(`words: ${p.join(" ")}\n`, "utf16le"),
  ]);
  const be = Buffer.from(le).swap16();
  for (const buf of [le, be]) {
    const r = scanFile("notes.txt", buf);
    assert.equal(r.binary, false);
    assert.deepEqual(
      r.findings.map((f) => f.words),
      [24],
    );
  }
  assert.equal(decodeText(be).includes(p[0]), true);
});

test("a file with a NUL byte is binary: counted, not scanned", () => {
  const buf = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x00]),
    Buffer.from(phrase(randomBytes(16)).join(" ")),
  ]);
  assert.deepEqual(scanFile("image.png", buf), { findings: [], binary: true });
});

test("a text file too large to scan is a finding, not a skip", () => {
  const r = scanFile("big.log", Buffer.alloc(MAX_TEXT_BYTES + 1, "a"));
  assert.deepEqual(
    r.findings.map((f) => f.kind),
    ["text file too large to scan"],
  );
});

// --- The BIP39 wordlist -----------------------------------------------------

test("the BIP39 wordlist at its own path is not a finding", () => {
  assert.deepEqual(scanFile(WORDLIST_PATH, WORDLIST).findings, []);
});

test("the BIP39 wordlist anywhere else is a finding: it is skipped by exact path", () => {
  for (const path of [
    "docs/bip39-english.txt",
    "scripts/bip39-english.txt.orig",
    "vendor/scripts/bip39-english.txt",
  ])
    assert.equal(scanFile(path, WORDLIST).findings.length, 1, path);
});

test("the wordlist path is scanned when it holds anything but the published list", () => {
  const p = phrase(randomBytes(32));
  const grown = Buffer.concat([WORDLIST, Buffer.from(`${p.join(" ")}\n`)]);
  assert.ok(scanFile(WORDLIST_PATH, grown).findings.length >= 1);
});

test("a CRLF checkout of the wordlist is still the wordlist", () => {
  const crlf = Buffer.from(WORDLIST.toString("utf8").replaceAll("\n", "\r\n"));
  assert.deepEqual(scanFile(WORDLIST_PATH, crlf).findings, []);
});

test("the scanner refuses to run with a wordlist that is not the published one", () => {
  const dir = mkdtempSync(join(tmpdir(), "secret-gate-"));
  try {
    copyFileSync(join(ROOT, "scripts/check-secrets.mjs"), join(dir, "check-secrets.mjs"));
    const swapped = [...WORDS];
    [swapped[0], swapped[1]] = [swapped[1], swapped[0]];
    writeFileSync(join(dir, "bip39-english.txt"), `${swapped.join("\n")}\n`);
    writeFileSync(join(dir, "clean.txt"), "nothing here\n");
    const r = spawnSync(
      process.execPath,
      [join(dir, "check-secrets.mjs"), join(dir, "clean.txt")],
      {
        encoding: "utf8",
      },
    );
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /is not the published BIP39 English wordlist/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// --- This repository --------------------------------------------------------

test("every file git tracks scans clean, the BIP39 wordlist included", () => {
  const tracked = spawnSync("git", ["ls-files", "-z"], { cwd: ROOT, encoding: "utf8" });
  assert.ok(tracked.stdout.split("\0").includes(WORDLIST_PATH));
  const r = spawnSync(process.execPath, [join(ROOT, "scripts/check-secrets.mjs")], {
    cwd: ROOT,
    encoding: "utf8",
  });
  assert.equal(r.status, 0, r.stderr);
  const scanned = Number(r.stdout.match(/in (\d+) files/)?.[1]);
  assert.ok(scanned > 500, r.stdout);
});

test("the command line prints file:line and the kind, never the words", () => {
  const dir = mkdtempSync(join(tmpdir(), "secret-gate-"));
  try {
    const p = phrase(randomBytes(32));
    const file = join(dir, "vector.json");
    writeFileSync(file, `{\n  "mnemonic": "${p.join(" ")}"\n}\n`);
    const r = spawnSync(process.execPath, [join(ROOT, "scripts/check-secrets.mjs"), file], {
      encoding: "utf8",
    });
    assert.equal(r.status, 1);
    assert.ok(r.stderr.includes(`${file}:2: recovery phrase (24 words)`), r.stderr);
    assertNotPrinted(r.stdout + r.stderr, p);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// --- Git hooks --------------------------------------------------------------

const GATE_FILES = [
  "scripts/check-secrets.mjs",
  WORDLIST_PATH,
  "scripts/install-git-hooks.mjs",
  ".githooks/pre-commit",
  ".githooks/pre-merge-commit",
  ".githooks/pre-push",
];

/** The environment without CI and with git reading no user or system configuration. */
function isolatedEnv(home) {
  const env = { ...process.env };
  for (const k of Object.keys(env)) if (k.startsWith("GIT_")) delete env[k];
  delete env.CI;
  return Object.assign(env, {
    HOME: home,
    GIT_CONFIG_GLOBAL: join(home, "gitconfig"),
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_AUTHOR_NAME: "Test",
    GIT_AUTHOR_EMAIL: "test@example.invalid",
    GIT_COMMITTER_NAME: "Test",
    GIT_COMMITTER_EMAIL: "test@example.invalid",
  });
}

/**
 * A fresh repository at `<base>/repo` holding the gate's files as committed
 * here (same modes), with git isolated from the user's and the system's
 * configuration.
 */
function sandbox(sub = "") {
  const base = mkdtempSync(join(tmpdir(), "secret-gate-"));
  const repo = join(base, "repo");
  const pkg = join(repo, sub);
  mkdirSync(pkg, { recursive: true });
  const env = isolatedEnv(base);
  const run = (cmd, args, opts = {}) =>
    spawnSync(cmd, args, { cwd: opts.cwd ?? repo, env: { ...env, ...opts.env }, encoding: "utf8" });
  const git = (...args) => run("git", args);
  const ok = (...args) => {
    const r = git(...args);
    assert.equal(r.status, 0, `git ${args.join(" ")}: ${r.stderr}`);
    return r.stdout.trim();
  };
  ok("init", "-q", "-b", "main");
  for (const f of GATE_FILES) {
    mkdirSync(dirname(join(pkg, f)), { recursive: true });
    copyFileSync(join(ROOT, f), join(pkg, f));
    chmodSync(join(pkg, f), statSync(join(ROOT, f)).mode);
  }
  const write = (f, text) => writeFileSync(join(repo, f), text);
  const install = (opts) =>
    run(
      process.execPath,
      [join(pkg, "scripts/install-git-hooks.mjs"), ...(opts?.args ?? [])],
      opts,
    );
  const hooksPath = () => git("config", "--get", "core.hooksPath").stdout.trim();
  const cleanup = () => rmSync(base, { recursive: true, force: true });
  return { base, repo, git, ok, write, install, hooksPath, cleanup };
}

/** A sandbox with the hooks installed, the gate files committed and a bare `origin`. */
function hooked() {
  const s = sandbox();
  assert.equal(s.install().status, 0);
  s.ok("add", ".");
  s.ok("commit", "-q", "-m", "gate");
  s.ok("init", "-q", "--bare", "-b", "main", join(s.base, "remote.git"));
  s.ok("remote", "add", "origin", join(s.base, "remote.git"));
  return s;
}

const remoteHas = (s, ref) =>
  s.git("--git-dir", join(s.base, "remote.git"), "rev-parse", "--verify", "-q", ref).status === 0;

test("pnpm install installs the hooks: the prepare script runs the installer", () => {
  const pkg = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8"));
  assert.equal(pkg.scripts.prepare, "node scripts/install-git-hooks.mjs");
});

test("the hooks are committed executable", () => {
  const r = spawnSync(
    "git",
    [
      "ls-files",
      "-s",
      "--",
      ".githooks/pre-commit",
      ".githooks/pre-merge-commit",
      ".githooks/pre-push",
    ],
    {
      cwd: ROOT,
      encoding: "utf8",
    },
  );
  const modes = r.stdout
    .trim()
    .split("\n")
    .map((l) => l.split(" ")[0]);
  assert.deepEqual(modes, ["100755", "100755", "100755"]);
});

test("the installer points git at .githooks, and running it again changes nothing", () => {
  const s = sandbox();
  try {
    assert.equal(s.install().status, 0);
    assert.equal(s.hooksPath(), ".githooks");
    const again = s.install();
    assert.equal(again.status, 0);
    assert.match(again.stdout, /already installed/);
  } finally {
    s.cleanup();
  }
});

test("the installer does nothing in CI", () => {
  const s = sandbox();
  try {
    const r = s.install({ env: { CI: "true" } });
    assert.equal(r.status, 0);
    assert.equal(s.hooksPath(), "");
  } finally {
    s.cleanup();
  }
});

test("the installer does nothing outside a git checkout", () => {
  const base = mkdtempSync(join(tmpdir(), "secret-gate-"));
  try {
    const installer = join(base, "plain/scripts/install-git-hooks.mjs");
    mkdirSync(dirname(installer), { recursive: true });
    copyFileSync(join(ROOT, "scripts/install-git-hooks.mjs"), installer);
    const r = spawnSync(process.execPath, [installer], {
      cwd: join(base, "plain"),
      env: isolatedEnv(base),
      encoding: "utf8",
    });
    assert.equal(r.status, 0);
    assert.match(r.stdout, /not a git checkout/);
    assert.equal(existsSync(join(base, "plain/.git")), false);
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
});

test("the installer does nothing when the package is not the top of the checkout", () => {
  const s = sandbox("app");
  try {
    const r = s.install();
    assert.equal(r.status, 0);
    assert.match(r.stdout, /is not the top of the git checkout/);
    assert.equal(s.hooksPath(), "");
  } finally {
    s.cleanup();
  }
});

test("the check fails in a clone without the hooks, names the fix and changes nothing", () => {
  const s = sandbox();
  try {
    const r = s.install({ args: ["--check"] });
    assert.equal(r.status, 1);
    assert.match(r.stderr, /Run node scripts\/install-git-hooks\.mjs/);
    assert.equal(s.hooksPath(), "");
  } finally {
    s.cleanup();
  }
});

test("the check passes with the hooks installed, and in CI without them", () => {
  const s = sandbox();
  try {
    assert.equal(s.install({ args: ["--check"], env: { CI: "true" } }).status, 0);
    assert.equal(s.hooksPath(), "");
    assert.equal(s.install().status, 0);
    assert.equal(s.install({ args: ["--check"] }).status, 0);
  } finally {
    s.cleanup();
  }
});

test("pnpm lint:secrets fails on a clone without the hooks", () => {
  const pkg = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8"));
  assert.match(pkg.scripts["lint:secrets"], /^node scripts\/install-git-hooks\.mjs --check && /);
});

test("the installer refuses to replace another core.hooksPath", () => {
  const s = sandbox();
  try {
    s.ok("config", "core.hooksPath", "elsewhere");
    const r = s.install();
    assert.equal(r.status, 1);
    assert.match(r.stderr, /already runs hooks from elsewhere/);
    assert.equal(s.hooksPath(), "elsewhere");
  } finally {
    s.cleanup();
  }
});

test("the installer refuses to silence hooks already in .git/hooks", () => {
  const s = sandbox();
  try {
    writeFileSync(join(s.repo, ".git/hooks/pre-commit"), "#!/bin/sh\nexit 0\n");
    const r = s.install();
    assert.equal(r.status, 1);
    assert.match(r.stderr, /pre-commit/);
    assert.equal(s.hooksPath(), "");
  } finally {
    s.cleanup();
  }
});

test("a dotfile in .git/hooks is not a hook: the installer goes on", () => {
  const s = sandbox();
  try {
    writeFileSync(join(s.repo, ".git/hooks/.DS_Store"), "x");
    assert.equal(s.install().status, 0);
    assert.equal(s.hooksPath(), ".githooks");
  } finally {
    s.cleanup();
  }
});

test("a commit that stages a phrase is refused, and the hook prints where, not what", () => {
  const s = hooked();
  try {
    const p = phrase(randomBytes(32));
    s.write("vector.json", `{\n  "mnemonic": "${p.join(" ")}"\n}\n`);
    s.ok("add", "vector.json");
    const r = s.git("commit", "-q", "-m", "add a vector");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /^vector\.json:2: recovery phrase \(24 words\)$/m);
    assert.match(r.stderr, /Commit refused/);
    assertNotPrinted(r.stdout + r.stderr, p);
    assert.equal(s.ok("rev-list", "--count", "HEAD"), "1");
  } finally {
    s.cleanup();
  }
});

test("the first commit of a repository is checked too: there is no HEAD to compare with", () => {
  const s = sandbox();
  try {
    assert.equal(s.install().status, 0);
    s.write("seed.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "seed.txt");
    const r = s.git("commit", "-q", "-m", "first");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /^seed\.txt:1: recovery phrase \(12 words\)$/m);
  } finally {
    s.cleanup();
  }
});

test("a staged phrase is refused even after the working copy is cleaned", () => {
  const s = hooked();
  try {
    s.write("notes.md", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "notes.md");
    s.write("notes.md", "nothing to see\n");
    const r = s.git("commit", "-q", "-m", "notes");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /^notes\.md:1: recovery phrase \(12 words\)$/m);
  } finally {
    s.cleanup();
  }
});

test("a staged phrase whose file was deleted from disk is refused", () => {
  const s = hooked();
  try {
    s.write("seed.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "seed.txt");
    rmSync(join(s.repo, "seed.txt"));
    const r = s.git("commit", "-q", "-m", "seed");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /^seed\.txt:1: recovery phrase \(12 words\)$/m);
  } finally {
    s.cleanup();
  }
});

test("a commit with nothing secret in it goes through", () => {
  const s = hooked();
  try {
    s.write("README.md", "# Test\n\nNothing secret here, only prose and `code`.\n");
    s.ok("add", "README.md");
    assert.equal(s.git("commit", "-q", "-m", "readme").status, 0);
  } finally {
    s.cleanup();
  }
});

test("a push of commits that added and then removed a phrase is refused, and nothing reaches the remote", () => {
  const s = hooked();
  try {
    s.write("secret.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "secret.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "oops");
    const added = s.ok("rev-parse", "HEAD");
    s.ok("rm", "-q", "secret.txt");
    s.ok("commit", "-q", "-m", "remove it");
    const r = s.git("push", "-q", "origin", "main");
    assert.notEqual(r.status, 0);
    assert.match(
      r.stderr,
      new RegExp(`^${added.slice(0, 10)} secret\\.txt:1: recovery phrase \\(12 words\\)$`, "m"),
    );
    assert.match(r.stderr, /Push refused, nothing was sent/);
    assert.equal(remoteHas(s, "refs/heads/main"), false);
  } finally {
    s.cleanup();
  }
});

test("a phrase in a commit message is refused at push", () => {
  const s = hooked();
  try {
    const p = phrase(randomBytes(16));
    s.write("a.txt", "a\n");
    s.ok("add", "a.txt");
    s.ok("commit", "-q", "-m", "subject", "-m", `body: ${p.join(" ")}`);
    const r = s.git("push", "-q", "origin", "main");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /^[0-9a-f]{10} commit message:3: recovery phrase \(12 words\)$/m);
    assertNotPrinted(r.stdout + r.stderr, p);
    assert.equal(remoteHas(s, "refs/heads/main"), false);
  } finally {
    s.cleanup();
  }
});

test("a phrase is reported once, by the commit that added it: at commit time not again, at push by that commit", () => {
  const s = hooked();
  try {
    s.write("vectors.txt", `${phrase(randomBytes(16)).join(" ")}\nfirst\n`);
    s.ok("add", "vectors.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "bypassed");
    const added = s.ok("rev-parse", "HEAD");
    s.write("vectors.txt", `${readFileSync(join(s.repo, "vectors.txt"), "utf8")}second\n`);
    s.ok("add", "vectors.txt");
    assert.equal(s.git("commit", "-q", "-m", "edit the same file").status, 0);
    const r = s.git("push", "-q", "origin", "main");
    assert.notEqual(r.status, 0);
    const found = r.stderr.split("\n").filter((l) => l.includes("recovery phrase"));
    assert.deepEqual(found, [`${added.slice(0, 10)} vectors.txt:1: recovery phrase (12 words)`]);
  } finally {
    s.cleanup();
  }
});

test("a push scans only what the remote lacks: a commit already on it is not scanned again", () => {
  const s = hooked();
  try {
    s.ok("push", "-q", "origin", "main");
    s.write("old.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "old.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "already public");
    s.ok("push", "-q", "--no-verify", "origin", "main");
    s.write("new.txt", "clean\n");
    s.ok("add", "new.txt");
    s.ok("commit", "-q", "-m", "clean");
    // To the named remote (its remote-tracking branches), to its URL (the
    // remote's old tip), and as a new branch.
    assert.equal(s.git("push", "-q", "origin", "main").status, 0);
    s.write("newer.txt", "clean\n");
    s.ok("add", "newer.txt");
    s.ok("commit", "-q", "-m", "clean again");
    assert.equal(s.git("push", "-q", join(s.base, "remote.git"), "main").status, 0);
    assert.equal(s.git("push", "-q", "origin", "main:topic").status, 0);
  } finally {
    s.cleanup();
  }
});

test("deleting a remote branch is not refused", () => {
  const s = hooked();
  try {
    s.ok("push", "-q", "origin", "main", "main:topic");
    assert.equal(s.git("push", "-q", "origin", "--delete", "topic").status, 0);
    assert.equal(remoteHas(s, "refs/heads/topic"), false);
  } finally {
    s.cleanup();
  }
});

test("a commit -a that picks up a phrase is refused: the hook reads the index git built for it", () => {
  const s = hooked();
  try {
    s.write("notes.md", "nothing yet\n");
    s.ok("add", "notes.md");
    s.ok("commit", "-q", "-m", "notes");
    s.write("notes.md", `${phrase(randomBytes(16)).join(" ")}\n`);
    const r = s.git("commit", "-q", "-a", "-m", "more notes");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /^notes\.md:1: recovery phrase \(12 words\)$/m);
  } finally {
    s.cleanup();
  }
});

test("a phrase in a root commit is refused at push", () => {
  const s = hooked();
  try {
    s.ok("checkout", "-q", "--orphan", "fresh");
    s.write("seed.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "seed.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "start over");
    const r = s.git("push", "-q", "origin", "fresh");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /^[0-9a-f]{10} seed\.txt:1: recovery phrase \(12 words\)$/m);
    assert.equal(remoteHas(s, "refs/heads/fresh"), false);
  } finally {
    s.cleanup();
  }
});

test("a phrase a merge commit adds by itself is refused at push", () => {
  const s = hooked();
  try {
    s.ok("checkout", "-q", "-b", "topic");
    s.write("topic.txt", "clean\n");
    s.ok("add", "topic.txt");
    s.ok("commit", "-q", "-m", "topic");
    s.ok("checkout", "-q", "main");
    s.ok("merge", "-q", "--no-ff", "--no-commit", "topic");
    s.write("seed.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "seed.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "merge topic");
    const merge = s.ok("rev-parse", "HEAD");
    const r = s.git("push", "-q", "origin", "main");
    assert.notEqual(r.status, 0);
    assert.ok(r.stderr.includes(`${merge.slice(0, 10)} seed.txt:1: recovery phrase`), r.stderr);
  } finally {
    s.cleanup();
  }
});

test("a merge that brings a phrase from a branch already on the remote into another branch is refused", () => {
  const s = hooked();
  try {
    s.ok("push", "-q", "origin", "main");
    s.ok("checkout", "-q", "-b", "topic");
    s.write("seed.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "seed.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "already public");
    s.ok("push", "-q", "--no-verify", "origin", "topic");
    s.ok("checkout", "-q", "main");
    s.write("other.txt", "clean\n");
    s.ok("add", "other.txt");
    s.ok("commit", "-q", "-m", "other");
    s.ok("merge", "-q", "--no-ff", "--no-edit", "--no-verify", "topic");
    const merge = s.ok("rev-parse", "HEAD");
    const r = s.git("push", "-q", "origin", "main");
    assert.notEqual(r.status, 0);
    const found = r.stderr.split("\n").filter((l) => l.includes("recovery phrase"));
    assert.deepEqual(found, [`${merge.slice(0, 10)} seed.txt:1: recovery phrase (12 words)`]);
  } finally {
    s.cleanup();
  }
});

test("a phrase added while renaming a file is reported at the new path; a plain rename adds nothing", () => {
  const s = hooked();
  try {
    const lines = Array.from({ length: 40 }, (_, i) => `line ${i}`).join("\n");
    s.write("old.txt", `${lines}\n`);
    s.ok("add", "old.txt");
    s.ok("commit", "-q", "-m", "old");
    s.ok("mv", "old.txt", "mid.txt");
    s.write("mid.txt", `${lines}\n${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "mid.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "rename and add");
    const added = s.ok("rev-parse", "HEAD");
    s.ok("mv", "mid.txt", "new.txt");
    assert.equal(s.git("commit", "-q", "-m", "plain rename").status, 0);
    const r = s.git("push", "-q", "origin", "main");
    assert.notEqual(r.status, 0);
    const found = r.stderr.split("\n").filter((l) => l.includes("recovery phrase"));
    assert.deepEqual(found, [`${added.slice(0, 10)} mid.txt:41: recovery phrase (12 words)`]);
  } finally {
    s.cleanup();
  }
});

test("a submodule's commit is another repository's: adding or replacing one reads no object", () => {
  const s = hooked();
  try {
    s.ok("update-index", "--add", "--cacheinfo", `160000,${"1".repeat(40)},vendor`);
    assert.equal(s.git("commit", "-q", "-m", "add a submodule").status, 0);
    s.ok("rm", "-q", "--cached", "vendor");
    s.write("vendor", "a file now\n");
    s.ok("add", "vendor");
    assert.equal(s.git("commit", "-q", "-m", "replace it").status, 0);
    const r = s.git("push", "-q", "origin", "main");
    assert.equal(r.status, 0, r.stderr);
  } finally {
    s.cleanup();
  }
});

test("a scan that cannot read what is being pushed refuses the push", () => {
  const s = hooked();
  try {
    const r = spawnSync(process.execPath, ["scripts/check-secrets.mjs", "--push", "origin"], {
      cwd: s.repo,
      env: isolatedEnv(s.base),
      encoding: "utf8",
      input: `refs/heads/main ${"1".repeat(40)} refs/heads/main ${"0".repeat(40)}\n`,
    });
    assert.equal(r.status, 2, r.stdout);
  } finally {
    s.cleanup();
  }
});

test("a staged rename keeps the secret the file already held out of this commit's report", () => {
  const s = hooked();
  try {
    s.write("a.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "a.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "old");
    s.ok("mv", "a.txt", "b.txt");
    const r = s.git("commit", "-q", "-m", "rename");
    assert.equal(r.status, 0, r.stderr);
  } finally {
    s.cleanup();
  }
});

test("a merge whose result adds a phrase is refused before the merge commit", () => {
  const s = hooked();
  try {
    s.ok("switch", "-q", "-c", "feature");
    s.write("a.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "a.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "feature");
    s.ok("switch", "-q", "main");
    s.write("b.txt", "other\n");
    s.ok("add", "b.txt");
    s.ok("commit", "-q", "-m", "main moves on");
    const r = s.git("merge", "-q", "--no-ff", "-m", "merge", "feature");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /a\.txt:1: recovery phrase \(12 words\)/);
  } finally {
    s.cleanup();
  }
});

test("the refusals say what to do: a push to rewrite, a found secret to move its funds", () => {
  const s = hooked();
  try {
    s.write("a.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    const tree = spawnSync(process.execPath, ["scripts/check-secrets.mjs", "a.txt"], {
      cwd: s.repo,
      env: isolatedEnv(s.base),
      encoding: "utf8",
    });
    assert.match(tree.stderr, /it is public: move its funds now/);
    s.ok("add", "a.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "add");
    const push = s.git("push", "-q", "origin", "main");
    assert.match(push.stderr, /Rewrite those commits without them first/);
  } finally {
    s.cleanup();
  }
});

test("a pushed commit whose blob cannot be read refuses the push", () => {
  const s = hooked();
  try {
    s.write("a.txt", "fine\n");
    s.ok("add", "a.txt");
    s.ok("commit", "-q", "-m", "add");
    const blob = s.ok("rev-parse", "HEAD:a.txt");
    rmSync(join(s.repo, ".git/objects", blob.slice(0, 2), blob.slice(2)), { force: true });
    const r = spawnSync(process.execPath, ["scripts/check-secrets.mjs", "--push", "origin"], {
      cwd: s.repo,
      env: isolatedEnv(s.base),
      encoding: "utf8",
      input: `refs/heads/main ${s.ok("rev-parse", "HEAD")} refs/heads/main ${"0".repeat(40)}\n`,
    });
    assert.equal(r.status, 2, r.stdout);
    assert.match(r.stderr, /is missing/);
  } finally {
    s.cleanup();
  }
});

// --- What else a push publishes ---------------------------------------------

test("a tag of a blob or of a tree is refused at push and does not reach the remote", () => {
  const s = hooked();
  try {
    s.write("a.txt", "fine\n");
    const blob = s.ok("hash-object", "-w", "a.txt");
    s.ok("tag", "blobtag", blob);
    s.ok("tag", "treetag", s.ok("rev-parse", "HEAD^{tree}"));
    for (const [tag, type] of [
      ["blobtag", "blob"],
      ["treetag", "tree"],
    ]) {
      const r = s.git("push", "-q", "origin", tag);
      assert.notEqual(r.status, 0, tag);
      assert.ok(r.stderr.includes(`a ref to a ${type}`), r.stderr);
      assert.equal(remoteHas(s, `refs/tags/${tag}`), false);
    }
  } finally {
    s.cleanup();
  }
});

test("an annotated tag of a blob is refused too: the tag is followed to what it holds", () => {
  const s = hooked();
  try {
    s.write("a.txt", "fine\n");
    s.ok("tag", "-a", "-m", "fine", "annotated", s.ok("hash-object", "-w", "a.txt"));
    s.ok("tag", "-a", "-m", "fine", "nested", "annotated");
    assert.notEqual(s.git("push", "-q", "origin", "annotated").status, 0);
    assert.notEqual(s.git("push", "-q", "origin", "nested").status, 0);
  } finally {
    s.cleanup();
  }
});

test("a phrase in an annotated tag's message is refused at push, without printing it", () => {
  const s = hooked();
  try {
    const p = phrase(randomBytes(16));
    s.ok("tag", "-a", "-m", p.join(" "), "v1");
    const r = s.git("push", "-q", "origin", "v1");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /^[0-9a-f]{10} tag:\d+: recovery phrase \(12 words\)$/m);
    assertNotPrinted(r.stderr, p);
    assert.equal(remoteHas(s, "refs/tags/v1"), false);
  } finally {
    s.cleanup();
  }
});

test("an annotated tag brings its commits into the scan, and a clean one goes through", () => {
  const s = hooked();
  try {
    s.ok("tag", "-a", "-m", "clean", "v1");
    assert.equal(s.git("push", "-q", "origin", "v1").status, 0);
    s.write("a.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
    s.ok("add", "a.txt");
    s.ok("commit", "-q", "--no-verify", "-m", "add");
    s.ok("tag", "-a", "-m", "clean", "v2");
    const r = s.git("push", "-q", "origin", "v2");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /a\.txt:1: recovery phrase/);
  } finally {
    s.cleanup();
  }
});

test("a phrase in the author's name is refused at push, without printing it", () => {
  const s = hooked();
  try {
    const p = phrase(randomBytes(16));
    s.ok("commit", "-q", "--allow-empty", `--author=${p.join(" ")} <a@example.invalid>`, "-m", "x");
    const r = s.git("push", "-q", "origin", "main");
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /commit author:\d+: recovery phrase \(12 words\)/);
    assertNotPrinted(r.stderr, p);
  } finally {
    s.cleanup();
  }
});

test("a phrase in a file name is refused at commit and at push, without printing it", () => {
  const s = hooked();
  try {
    const p = phrase(randomBytes(16));
    s.write(`${p.join(" ")}.txt`, "fine\n");
    s.ok("add", ".");
    const commit = s.git("commit", "-q", "-m", "add");
    assert.notEqual(commit.status, 0);
    assert.match(
      commit.stderr,
      /\(a file whose name holds a secret\):1: file name: recovery phrase/,
    );
    assertNotPrinted(commit.stderr, p);
    s.ok("commit", "-q", "--no-verify", "-m", "add");
    const push = s.git("push", "-q", "origin", "main");
    assert.notEqual(push.status, 0);
    assertNotPrinted(push.stderr, p);
  } finally {
    s.cleanup();
  }
});

test("a phrase in a ref name is refused at push, without printing it", () => {
  const s = hooked();
  try {
    const p = phrase(randomBytes(16));
    const r = s.git("push", "-q", "origin", `main:refs/heads/${p.join("-")}`);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /a ref name:\d+: recovery phrase/);
    assertNotPrinted(r.stderr, p);
  } finally {
    s.cleanup();
  }
});

// --- CI ---------------------------------------------------------------------

/** The scanner on a commit range, as the Secrets workflow runs it. */
function scanRange(s, before, head) {
  return spawnSync(
    process.execPath,
    ["scripts/check-secrets.mjs", "--range", before, head, "main"],
    { cwd: s.repo, env: isolatedEnv(s.base), encoding: "utf8" },
  );
}

/** A branch off main that adds a phrase and removes it again, with an older phrase on main. */
function branchWithRemovedPhrase(s) {
  s.write("old.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
  s.ok("add", "old.txt");
  s.ok("commit", "-q", "--no-verify", "-m", "burned, long ago");
  s.ok("rm", "-q", "old.txt");
  s.ok("commit", "-q", "-m", "removed");
  const main = s.ok("rev-parse", "HEAD");
  s.ok("switch", "-q", "-c", "feature");
  s.write("a.txt", `${phrase(randomBytes(16)).join(" ")}\n`);
  s.ok("add", "a.txt");
  s.ok("commit", "-q", "--no-verify", "-m", "add");
  s.ok("rm", "-q", "a.txt");
  s.ok("commit", "-q", "-m", "remove");
  return { main, head: s.ok("rev-parse", "HEAD") };
}

test("a range scan finds a phrase added and removed again, naming the commit, and says to move funds", () => {
  const s = hooked();
  try {
    const { main, head } = branchWithRemovedPhrase(s);
    const r = scanRange(s, main, head);
    assert.equal(r.status, 1, r.stdout);
    assert.deepEqual(
      r.stderr.split("\n").filter((l) => l.includes(": recovery phrase")),
      [`${s.ok("rev-parse", "HEAD~1").slice(0, 10)} a.txt:1: recovery phrase (12 words)`],
    );
    assert.match(r.stderr, /move its funds now/);
  } finally {
    s.cleanup();
  }
});

test("a range scan of a new branch starts at its merge base: old history is not reported", () => {
  const s = hooked();
  try {
    const { head } = branchWithRemovedPhrase(s);
    const onlyNew = (before) => {
      const r = scanRange(s, before, head);
      assert.equal(r.status, 1, r.stdout);
      assert.equal(r.stderr.split("\n").filter((l) => l.includes(": recovery phrase")).length, 1);
      assert.match(r.stderr, / a\.txt:1: /);
    };
    onlyNew("0".repeat(40));
    onlyNew("1".repeat(40)); // force-pushed away: not in the clone
    onlyNew("");
  } finally {
    s.cleanup();
  }
});

test("a range scan reads only what comes after BEFORE; with nothing new it passes", () => {
  const s = hooked();
  try {
    const head = s.ok("rev-parse", "HEAD");
    const r = scanRange(s, head, head);
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stdout, /in 0 commits and files/);
    const { head: tip } = branchWithRemovedPhrase(s);
    assert.equal(scanRange(s, s.ok("rev-parse", `${tip}~1`), tip).status, 0, "BEFORE is honoured");
  } finally {
    s.cleanup();
  }
});

test("a range scan that cannot work out what to read fails", () => {
  const s = hooked();
  try {
    s.ok("checkout", "-q", "--orphan", "other");
    s.ok("commit", "-q", "--allow-empty", "-m", "unrelated");
    const r = scanRange(s, "0".repeat(40), "other");
    assert.equal(r.status, 2, r.stdout + r.stderr);
    const noArgs = spawnSync(process.execPath, ["scripts/check-secrets.mjs", "--range"], {
      cwd: s.repo,
      env: isolatedEnv(s.base),
      encoding: "utf8",
    });
    assert.equal(noArgs.status, 2);
  } finally {
    s.cleanup();
  }
});

test("CI scans on every branch and pull request, on its own, with nothing installed", () => {
  const yml = readFileSync(join(ROOT, ".github/workflows/secret-scan.yml"), "utf8");
  assert.match(yml, /^ {2}push:\n {4}branches: \["\*\*"\]$/m);
  assert.match(yml, /^ {2}pull_request:$/m);
  assert.doesNotMatch(yml, /^ {4}branches: \[main\]/m);
  assert.doesNotMatch(yml, /^\s+needs:/m);
  assert.doesNotMatch(yml, /pnpm|npm /);
  assert.match(yml, /fetch-depth: 0/);
  assert.match(yml, /run: node scripts\/check-secrets\.mjs$/m);
  assert.match(yml, /--range "\$BEFORE" "\$GITHUB_SHA" origin\/main/);
  assert.match(yml, /run: node --test scripts\/check-secrets\.node-test\.mjs$/m);
  // No expression inside a shell line: they reach the shell through env.
  for (const line of yml.split("\n").filter((l) => /^\s+(- )?run:/.test(l)))
    assert.doesNotMatch(line, /\$\{\{/);
});
