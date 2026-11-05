// Run with: node site.test.cjs
const assert = require("node:assert");
const { detectSystem, pickFiles, formatSize } = require("./site.js");
const base = "https://github.com/DimazzzZ/namehold-wallet/releases/download/v0.7.0/";
const asset = (name, url = base + name) => ({ name, browser_download_url: url, size: 25338771 });

assert.equal(detectSystem("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"), "mac");
assert.equal(detectSystem("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"), "windows");
assert.equal(detectSystem("Mozilla/5.0 (X11; Linux x86_64)"), "appimage");
assert.equal(detectSystem("Mozilla/5.0 (Linux; Android 14)"), null);
assert.equal(detectSystem("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)"), null);

const files = pickFiles({ assets: [
  asset("Namehold_0.7.0_universal.dmg"), asset("Namehold_0.7.0_universal.dmg.sig"),
  asset("Namehold_0.7.0_universal.app.tar.gz"), asset("Namehold_0.7.0_x64-setup.exe"),
  asset("Namehold_0.7.0_amd64.AppImage"), asset("Namehold_0.7.0_amd64.deb"),
  asset("Namehold-0.7.0-1.x86_64.rpm"), asset("SHA256SUMS"), asset("SHA256SUMS.sig"),
] });
assert.equal(files.mac.name, "Namehold_0.7.0_universal.dmg");
assert.equal(files.rpm.name, "Namehold-0.7.0-1.x86_64.rpm");
assert.equal(Object.keys(files).length, 7);

// A link that leaves this repository's releases is never used.
assert.deepEqual(pickFiles({ assets: [asset("Namehold_0.7.0_universal.dmg", "https://evil.example/x.dmg")] }), {});
assert.deepEqual(pickFiles({ assets: [asset("Namehold_0.7.0_universal.dmg", "https://github.com/someone/else/releases/download/x.dmg")] }), {});
// A name goes into commands people paste, so nothing but plain file characters.
assert.deepEqual(pickFiles({ assets: [asset("x;rm -rf ~_universal.dmg")] }), {});
assert.deepEqual(pickFiles(null), {});
assert.equal(formatSize(25338771), "24 MB");
assert.equal(formatSize(undefined), "");
console.log("ok");
