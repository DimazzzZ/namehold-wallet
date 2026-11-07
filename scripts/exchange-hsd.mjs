#!/usr/bin/env node
// exchange-hsd.mjs — the HNS side of the exchange checked by hsd itself
// (docs/specs/2026-11-07-hns-btc-exchange.md, R1–R4).
//
//   node scripts/exchange-hsd.mjs vectors        print reference vectors (JSON)
//   node scripts/exchange-hsd.mjs verify < in    verify transactions (JSON in,
//                                                JSON out)
//
// `vectors` builds the lock script, its address, the claim's NOINPUT
// sighash and signature and the refund, with hsd's own Script, MTX and
// bcrypto — independently of the Rust code — from keys derived from public
// labels (no key material in the repo). The Rust test
// `exchange_hsd_vectors_tests` compares against the committed output
// (src-tauri/src/tests/fixtures/exchange_hsd_vectors.json).
//
// `verify` takes `[{ "txHex": "...", "coins": [{ "hash", "index", "value",
// "scriptHex" }] }]`, runs hsd's script interpreter on every input
// (`tx.verify(view, STANDARD_VERIFY_FLAGS)`) and prints
// `[{ "ok": bool, "error": "..." }]`. The Rust test
// `exchange_hsd_verify_tests` feeds it the transactions the Rust code signs;
// it skips when node or hsd is missing.
//
// hsd is found through HSD_LIB (a path to hsd's package directory), else
// the global npm root.

import { createRequire } from 'node:module';
import { execSync } from 'node:child_process';
import path from 'node:path';
import fs from 'node:fs';

function hsdDir() {
  if (process.env.HSD_LIB) return process.env.HSD_LIB;
  const root = execSync('npm root -g', { encoding: 'utf8' }).trim();
  return path.join(root, 'hsd');
}

const dir = hsdDir();
const require = createRequire(path.join(dir, 'package.json'));
const hsd = require(dir);
const secp256k1 = require('bcrypto/lib/secp256k1');
const sha256 = require('bcrypto/lib/sha256');
const { Script, MTX, TX, Coin, CoinView, Address, Outpoint } = hsd;
const consensus = hsd.consensus;
const common = require(path.join(dir, 'lib/script/common.js'));

const ALL = common.hashType.ALL;
const NOINPUT = common.hashType.NOINPUT;
const ANYONECANPAY = common.hashType.ANYONECANPAY;

function keyFrom(label) {
  // sha256 of a public label; valid secp256k1 scalars for these labels.
  return sha256.digest(Buffer.from(label, 'utf8'));
}

function htlcScript(hash, claimPub, refundPub, lockTime) {
  const s = new Script();
  s.pushSym('OP_IF');
  s.pushSym('OP_SIZE');
  s.pushInt(32);
  s.pushSym('OP_EQUALVERIFY');
  s.pushSym('OP_SHA256');
  s.pushData(hash);
  s.pushSym('OP_EQUALVERIFY');
  s.pushData(claimPub);
  s.pushSym('OP_ELSE');
  s.pushInt(lockTime);
  s.pushSym('OP_CHECKLOCKTIMEVERIFY');
  s.pushSym('OP_DROP');
  s.pushData(refundPub);
  s.pushSym('OP_ENDIF');
  s.pushSym('OP_CHECKSIG');
  s.compile();
  return s;
}

function sigWithType(hash, priv, type) {
  return Buffer.concat([secp256k1.sign(hash, priv), Buffer.from([type])]);
}

function vectors() {
  const claimPriv = keyFrom('namehold exchange vector: buyer hns claim');
  const refundPriv = keyFrom('namehold exchange vector: seller hns refund');
  const claimPub = secp256k1.publicKeyCreate(claimPriv, true);
  const refundPub = secp256k1.publicKeyCreate(refundPriv, true);
  const secret = sha256.digest(Buffer.from('namehold exchange vector: preimage', 'utf8'));
  const hash = sha256.digest(secret);
  const refundUnix = 1793000000; // a fixed Unix time
  const units = Math.ceil(refundUnix / consensus.LOCKTIME_MULT);
  const lockTime = (consensus.LOCKTIME_FLAG | units) >>> 0;
  const script = htlcScript(hash, claimPub, refundPub, lockTime);
  const lockAddr = Address.fromScript(script);
  const lockValue = 12345678000;
  const lockPrev = Buffer.alloc(32, 0x42);
  const toPub = secp256k1.publicKeyCreate(keyFrom('namehold exchange vector: buyer payout'), true);
  const toAddr = Address.fromPubkey(toPub);
  const fee = 50000;

  // Claim: one input (the lock), one output, lock time 0, sighash 0xc1.
  const claim = new MTX();
  claim.addOutpoint(new Outpoint(lockPrev, 1));
  claim.addOutput(toAddr, lockValue - fee);
  const claimType = (ALL | NOINPUT | ANYONECANPAY) >>> 0;
  const claimHash = claim.signatureHash(0, script, lockValue, claimType);
  const claimSig = sigWithType(claimHash, claimPriv, claimType);
  // The same signature over a claim of another outpoint: NOINPUT.
  const other = new MTX();
  other.addOutpoint(new Outpoint(Buffer.alloc(32, 0x77), 9));
  other.addOutput(toAddr, lockValue - fee);
  const otherHash = other.signatureHash(0, script, lockValue, claimType);
  claim.inputs[0].witness.items = [claimSig, secret, Buffer.from([1]), script.encode()];
  claim.inputs[0].witness.compile();

  const view = new CoinView();
  const coin = new Coin();
  coin.hash = lockPrev;
  coin.index = 1;
  coin.value = lockValue;
  coin.address = lockAddr;
  view.addCoin(coin);
  const claimTx = claim.toTX();
  const claimOk = claimTx.verify(view);

  // Refund: lock time = the script's, sequence below final, sighash ALL.
  const refundTo = Address.fromPubkey(
    secp256k1.publicKeyCreate(keyFrom('namehold exchange vector: seller payout'), true));
  const refund = new MTX();
  refund.locktime = lockTime;
  refund.addOutpoint(new Outpoint(lockPrev, 1));
  refund.inputs[0].sequence = 0xfffffffe;
  refund.addOutput(refundTo, lockValue - fee);
  const refundHash = refund.signatureHash(0, script, lockValue, ALL);
  const refundSig = sigWithType(refundHash, refundPriv, ALL);
  refund.inputs[0].witness.items = [refundSig, Buffer.alloc(0), script.encode()];
  refund.inputs[0].witness.compile();
  const refundTx = refund.toTX();
  const refundOk = refundTx.verify(view);

  return {
    hsdVersion: require(path.join(dir, 'package.json')).version,
    labels: {
      claimKey: 'namehold exchange vector: buyer hns claim',
      refundKey: 'namehold exchange vector: seller hns refund',
      secret: 'namehold exchange vector: preimage',
      buyerPayout: 'namehold exchange vector: buyer payout',
      sellerPayout: 'namehold exchange vector: seller payout',
    },
    hashHex: hash.toString('hex'),
    claimPubHex: claimPub.toString('hex'),
    refundPubHex: refundPub.toString('hex'),
    refundUnix,
    lockTime,
    scriptHex: script.encode().toString('hex'),
    lockAddressMain: lockAddr.toString('main'),
    lockAddressRegtest: lockAddr.toString('regtest'),
    lockValue,
    fee,
    lockPrevHex: lockPrev.toString('hex'),
    lockPrevIndex: 1,
    buyerPayoutMain: toAddr.toString('main'),
    sellerPayoutMain: refundTo.toString('main'),
    claimSighashHex: claimHash.toString('hex'),
    claimSighashOtherOutpointHex: otherHash.toString('hex'),
    claimSignatureHex: claimSig.toString('hex'),
    claimTxHex: claimTx.toHex(),
    claimTxid: claimTx.txid(),
    claimVerifies: claimOk,
    refundSighashHex: refundHash.toString('hex'),
    refundSignatureHex: refundSig.toString('hex'),
    refundTxHex: refundTx.toHex(),
    refundTxid: refundTx.txid(),
    refundVerifies: refundOk,
  };
}

function verify(items) {
  return items.map((item) => {
    try {
      const tx = TX.fromHex(item.txHex);
      const view = new CoinView();
      for (const c of item.coins) {
        const coin = new Coin();
        coin.hash = Buffer.from(c.hash, 'hex');
        coin.index = c.index;
        coin.value = c.value;
        coin.address = Address.fromScript(Script.decode(Buffer.from(c.scriptHex, 'hex')));
        view.addCoin(coin);
      }
      try {
        tx.check(view);
      } catch (e) {
        return { ok: false, error: `check: ${e.message}` };
      }
      for (let i = 0; i < tx.inputs.length; i++) {
        try {
          tx.verifyInput(i, view.getOutputFor(tx.inputs[i]), Script.flags.STANDARD_VERIFY_FLAGS);
        } catch (e) {
          return { ok: false, error: `input ${i}: ${e.code || e.message}` };
        }
      }
      return { ok: true, error: '' };
    } catch (e) {
      return { ok: false, error: e.message };
    }
  });
}

const mode = process.argv[2];
if (mode === 'vectors') {
  process.stdout.write(JSON.stringify(vectors(), null, 2) + '\n');
} else if (mode === 'verify') {
  const input = fs.readFileSync(0, 'utf8');
  process.stdout.write(JSON.stringify(verify(JSON.parse(input))) + '\n');
} else {
  process.stderr.write('usage: exchange-hsd.mjs vectors | verify < input.json\n');
  process.exit(2);
}
