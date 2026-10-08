"use strict";

// Independent known-answer transaction vectors generated from canonical hsd
// 8.0.0. Consumed by src-tauri/src/tests/hsd_parity_tests.rs to prove our Rust
// transaction construction / signing / serialization / covenant encoding match
// hsd byte-for-byte. Regenerate with: `npm install && node gen_hsd_vectors.js`.
//
// hsd and our Rust signer both produce RFC-6979 deterministic, low-S ECDSA
// signatures, so for identical inputs (same coins, output order, locktime,
// sighash type) the FULL signed-tx hex is identical — not merely valid.

const assert = require("assert");
const fs = require("fs");
const path = require("path");
const hsd = require("hsd");
const { Mnemonic, HDPrivateKey } = require("hsd").hd;
const { KeyRing, MTX, Coin, Output, Address, Script, Covenant, Network } = hsd;
const rules = require("hsd/lib/covenants/rules");
const sha3 = require("bcrypto/lib/sha3");
const Witness = require("hsd/lib/script/witness");

const NETWORK = Network.get("main");
const COIN_TYPE = NETWORK.keyPrefix.coinType; // 5353
const ACCOUNT = 0;
const HASH_ALL = Script.hashType.ALL; // 1

const MNEMONIC =
  "april coyote civil finger crane uncle situate moon choice wrong " +
  "goose client purse deer funny hobby shrug give anxiety truly rack " +
  "stand salad coach";

const master = HDPrivateKey.fromMnemonic(new Mnemonic(MNEMONIC));

// --- helpers -------------------------------------------------------------

function ring(branch, index) {
  const key = master.derivePath(`m/44'/${COIN_TYPE}'/${ACCOUNT}'/${branch}/${index}`);
  const r = KeyRing.fromPrivate(key.privateKey);
  r.witness = true;
  return r;
}

function addr(branch, index) {
  return ring(branch, index).getAddress().toString("main");
}

// Fee formula MUST mirror src-tauri/src/noncustodial/send.rs exactly:
//   base  = 10 (overhead) + nIn*40 + nOut*32 ; witness = nIn*101
//   vsize = ceil((base*4 + witness) / 4)     ; fee = vsize * max(rate,1)
// `assertVsize` checks this estimate against hsd's own getVirtualSize() of
// every signed P2WPKH vector, so the formula cannot drift from hsd silently.
function estSize(nIn, nOut) {
  const base = 10 + nIn * 40 + nOut * 32;
  return Math.ceil((base * 4 + nIn * 101) / 4);
}

function estFee(nIn, nOut, rate) {
  return estSize(nIn, nOut) * Math.max(rate, 1);
}

function assertVsize(mtx) {
  const want = mtx.getVirtualSize();
  const got = estSize(mtx.inputs.length, mtx.outputs.length);
  if (got !== want)
    throw new Error(
      `estSize(${mtx.inputs.length}, ${mtx.outputs.length}) = ${got}, hsd vsize = ${want}`,
    );
}

// Handshake does NOT byte-reverse hashes (unlike Bitcoin). The txid string the
// node reports for a coin is the exact byte order written into a spending
// input's prevout hash, so there is no transformation: the funding txid == the
// prevout hash bytes == hsd's Coin.hash.
function prevoutHash(txidHex) {
  return Buffer.from(txidHex, "hex");
}

function mkCoin(txid, vout, value, fundingRing) {
  return new Coin({
    version: 0,
    height: -1,
    value: value,
    hash: prevoutHash(txid),
    index: vout,
    address: fundingRing.getAddress(),
    covenant: new Covenant(),
  });
}

// Build, sign, and snapshot a plain (covenant-free) send.
function plainSend({ inputs, recipient, change, locktime = 0 }) {
  const mtx = new MTX();
  mtx.version = 0;
  mtx.locktime = locktime;

  const rings = [];
  for (const i of inputs) {
    const r = ring(i.branch, i.index);
    rings.push(r);
    mtx.addCoin(mkCoin(i.displayTxid, i.vout, i.value, r));
  }
  mtx.addOutput(Address.fromString(recipient.address, "main"), recipient.value);
  if (change) {
    mtx.addOutput(Address.fromString(change.address, "main"), change.value);
  }

  const signed = mtx.sign(rings);
  if (signed !== inputs.length) {
    throw new Error(`expected to sign ${inputs.length} inputs, signed ${signed}`);
  }
  assertVsize(mtx);

  // Per-input sighash (SIGHASH_ALL) using the P2WPKH script code.
  const sighashes = inputs.map((i, idx) => {
    const r = rings[idx];
    const prev = Script.fromPubkeyhash(r.getKeyHash());
    return mtx.signatureHash(idx, prev, i.value, HASH_ALL).toString("hex");
  });

  return {
    inputs: inputs.map((i, idx) => ({
      displayTxid: i.displayTxid,
      // Prevout hash literally present in the serialized tx bytes. Handshake
      // does not reverse, so this equals the funding txid byte-for-byte.
      prevoutHashInternal: i.displayTxid,
      vout: i.vout,
      value: i.value,
      branch: i.branch,
      index: i.index,
      keyHash160: ring(i.branch, i.index).getKeyHash().toString("hex"),
      sighashAll: sighashes[idx],
    })),
    recipient,
    change: change || null,
    locktime,
    txid: mtx.txid(),
    signedHex: mtx.toRaw().toString("hex"),
  };
}

// Build, sign, and snapshot a covenant-bearing tx (single covenant output +
// change). `covenantOutput` carries {value, address, covenant(hsd Covenant)}.
function covenantTx({ input, covenantOutput, change, locktime = 0 }) {
  const mtx = new MTX();
  mtx.version = 0;
  mtx.locktime = locktime;
  const r = ring(input.branch, input.index);
  mtx.addCoin(mkCoin(input.displayTxid, input.vout, input.value, r));

  const out = new Output();
  out.value = covenantOutput.value;
  out.address = Address.fromString(covenantOutput.address, "main");
  out.covenant = covenantOutput.covenant;
  mtx.outputs.push(out);

  mtx.addOutput(Address.fromString(change.address, "main"), change.value);

  const signed = mtx.sign([r]);
  if (signed !== 1) throw new Error("covenantTx: input not signed");

  const prev = Script.fromPubkeyhash(r.getKeyHash());
  const sighash = mtx.signatureHash(0, prev, input.value, HASH_ALL).toString("hex");

  return {
    input: {
      prevoutHashInternal: input.displayTxid,
      displayTxid: input.displayTxid,
      vout: input.vout,
      value: input.value,
      branch: input.branch,
      index: input.index,
      keyHash160: r.getKeyHash().toString("hex"),
      sighashAll: sighash,
    },
    covenantOutput: {
      value: covenantOutput.value,
      address: covenantOutput.address,
      covenantRaw: Buffer.from(covenantOutput.covenant.encode()).toString("hex"),
    },
    change,
    locktime,
    txid: mtx.txid(),
    signedHex: mtx.toRaw().toString("hex"),
  };
}

function cov(type, push) {
  const c = new Covenant();
  c.type = type;
  push(c);
  return c;
}

const T = rules.types;

// --- fixed test material -------------------------------------------------

const NAME = "proofofconcept";
const NAME_HASH = rules.hashName(NAME); // 32 bytes
const RAW_NAME = Buffer.from(NAME, "ascii");
const BLIND_NONCE = Buffer.alloc(32, 0x07);
const BLIND_VALUE = 1234567;
const BLIND = rules.blind(BLIND_VALUE, BLIND_NONCE);
const REVEAL_NONCE = Buffer.alloc(32, 0x09);
const RENEWAL_BLOCK = Buffer.alloc(32, 0x05);
const RESOURCE = Buffer.from([0x00]); // empty DNS resource
const ADDR_HASH20 = Buffer.alloc(20, 0x06);
const HEIGHT = 200;
const START = 100;

// Distinct, non-palindromic funding txids (display order).
const TXID_A = Buffer.from(Array.from({ length: 32 }, (_, i) => i + 1)).toString("hex");
const TXID_B = Buffer.from(Array.from({ length: 32 }, (_, i) => 0x40 + i)).toString("hex");
const TXID_C = Buffer.from(Array.from({ length: 32 }, (_, i) => 0x80 + i)).toString("hex");

// --- Shakedex (hsd-generated) --------------------------------------------
//
// Reference vectors for buying a name sold through a Shakedex lock: the lock
// script and address, one price step (sighash 0x84 = SINGLEREVERSE|ANYONECANPAY),
// the buyer's purchase transaction, and the buyer's FINALIZE out of the lock.
// And, under `sell`, the seller's side: the FINALIZE into the lock, price steps
// signed by the R17 key, the cancel and its FINALIZE.
// The purchase and the finalize are verified by hsd's own script interpreter.

function lockScript(pub) {
  // type == TRANSFER ? <pub> checksig : type == FINALIZE
  return Buffer.concat([
    Buffer.from("d0598763", "hex"),
    Buffer.from([0x21]),
    pub,
    Buffer.from("ac67d05a8768", "hex"),
  ]);
}

const shakedex = (() => {
  const name = "dexreviews";
  const nameHash = rules.hashName(name);
  const idx = nameHash.readUInt32BE(0) & 0x7fffffff;
  const lockKey = master.derivePath(`m/44'/${COIN_TYPE}'/0'/2'/${idx}'`);
  const pub = lockKey.publicKey;
  const script = lockScript(pub);
  const lockAddress = Address.fromScripthash(sha3.digest(script));
  const lockCoinHash = Buffer.alloc(32, 0x44);
  const lockValue = 1_000_000;
  const height = 120;
  const price = 250_000_000;
  const lockTimeSecs = 1_783_696_480;
  const encodedLocktime = ((lockTimeSecs >>> 9) | 0x80000000) >>> 0;
  const paymentAddr = addr(0, 5);
  const SIGHASH = 0x84;

  // hsd's own encoding of the seconds-based lock time must agree.
  const probe = new MTX();
  probe.addCoin(mkCoin(TXID_A, 0, 1, ring(0, 0)));
  probe.setLocktime(lockTimeSecs, true);
  assert.strictEqual(probe.locktime, encodedLocktime, "locktime encoding");
  assert.strictEqual(probe.inputs[0].sequence, 0xfffffffe, "locktime sequence");

  const lockCoin = new Coin({
    version: 0,
    height,
    value: lockValue,
    address: lockAddress,
    hash: lockCoinHash,
    index: 0,
    covenant: cov(T.FINALIZE, (c) => {
      c.pushHash(nameHash);
      c.pushU32(height);
      c.push(Buffer.from(name));
      c.pushU8(0);
      c.pushU32(0);
      c.pushU32(0);
      c.pushHash(Buffer.alloc(32, 0x55));
    }),
  });

  // Price-step template: input 0 = lock coin (sequence fffffffe), outputs
  // [placeholder at the lock address, payment]. SINGLEREVERSE commits only to
  // the payment output.
  const tpl = new MTX();
  tpl.version = 0;
  tpl.locktime = encodedLocktime;
  tpl.addCoin(lockCoin);
  tpl.inputs[0].sequence = 0xfffffffe;
  tpl.addOutput(lockAddress, lockValue);
  tpl.addOutput(Address.fromString(paymentAddr, "main"), price);
  const stepHash = tpl.signatureHash(0, Script.decode(script), lockValue, SIGHASH);
  const signature = tpl.signature(0, Script.decode(script), lockValue, lockKey.privateKey, SIGHASH);

  // Purchase: input 0 = lock coin with witness [sig, script], input 1 = buyer
  // funding; outputs [TRANSFER at lock address, change, payment].
  const dest = ring(0, 6);
  const funding = { displayTxid: TXID_A, vout: 0, value: 400_000_000, branch: 0, index: 0 };
  const fee = 20_000;
  const p = new MTX();
  p.version = 0;
  p.locktime = encodedLocktime;
  p.addCoin(lockCoin);
  p.inputs[0].sequence = 0xfffffffe;
  p.inputs[0].witness = Witness.fromItems([signature, script]);
  const fr = ring(funding.branch, funding.index);
  p.addCoin(mkCoin(funding.displayTxid, funding.vout, funding.value, fr));
  p.addOutput({
    address: lockAddress,
    value: lockValue,
    covenant: cov(T.TRANSFER, (c) => {
      c.pushHash(nameHash);
      c.pushU32(height);
      c.pushU8(0);
      c.push(dest.getKeyHash());
    }),
  });
  p.addOutput(Address.fromString(addr(1, 0), "main"), funding.value - price - fee);
  p.addOutput(Address.fromString(paymentAddr, "main"), price);
  assert.strictEqual(p.sign([fr]), 1, "purchase: only the funding input is signed by the buyer");
  assert(p.verify(), "purchase verifies in hsd");

  // Buyer FINALIZE out of the lock once the lockup has passed.
  const transferCoin = Coin.fromTX(p.toTX(), 0, height + 1);
  const f = new MTX();
  f.version = 0;
  f.addCoin(transferCoin);
  f.inputs[0].witness = Witness.fromItems([script]);
  const funding2 = { displayTxid: TXID_B, vout: 0, value: 1_000_000, branch: 0, index: 1 };
  const fr2 = ring(funding2.branch, funding2.index);
  f.addCoin(mkCoin(funding2.displayTxid, funding2.vout, funding2.value, fr2));
  const renewalBlock = Buffer.alloc(32, 0x66);
  f.addOutput({
    address: dest.getAddress(),
    value: lockValue,
    covenant: cov(T.FINALIZE, (c) => {
      c.pushHash(nameHash);
      c.pushU32(height);
      c.push(Buffer.from(name));
      c.pushU8(0);
      c.pushU32(0);
      c.pushU32(0);
      c.pushHash(renewalBlock);
    }),
  });
  const fee2 = 10_000;
  f.addOutput(Address.fromString(addr(1, 0), "main"), funding2.value - fee2);
  assert.strictEqual(f.sign([fr2]), 1, "finalize: only the funding input is signed");
  assert(f.verify(), "finalize verifies in hsd");

  // "dexreviews" is R17's golden name; "namehold" has the top bit of its
  // name hash set, so only the 0x7fffffff mask keeps its index below 2^31.
  const lockPath = [];
  for (const n of ["dexreviews", "namehold"]) {
    const h = rules.hashName(n);
    const index = h.readUInt32BE(0) & 0x7fffffff;
    for (const net of ["main", "regtest"]) {
      const coin = Network.get(net).keyPrefix.coinType;
      const path = `m/44'/${coin}'/0'/2'/${index}'`;
      const k = master.derivePath(path);
      lockPath.push({
        network: net,
        name: n,
        nameHashPrefix: h.readUInt32BE(0),
        index,
        path,
        lockPub: k.publicKey.toString("hex"),
        lockAddress: Address.fromScripthash(sha3.digest(lockScript(k.publicKey))).toString(net),
      });
    }
  }
  assert(
    lockPath.some((e) => e.nameHashPrefix >= 0x80000000),
    "a name with the top bit set",
  );

  // Selling with the same R17 lock key, chained as on chain: our TRANSFER
  // coin (committing to the lock program) is finalized into the lock; price
  // steps are signed over that FINALIZE coin; the cancel spends it with 0x83
  // into a TRANSFER at the lock address committing to our cancel address; the
  // cancel's FINALIZE brings the name there. hsd's interpreter verifies every
  // transaction, and each step through a purchase that spends it.
  const sell = (() => {
    const finalizeCov = (renewalBlock) =>
      cov(T.FINALIZE, (c) => {
        c.pushHash(nameHash);
        c.pushU32(height);
        c.push(Buffer.from(name));
        c.pushU8(0);
        c.pushU32(0);
        c.pushU32(0);
        c.pushHash(renewalBlock);
      });
    const transferCov = (hash) =>
      cov(T.TRANSFER, (c) => {
        c.pushHash(nameHash);
        c.pushU32(height);
        c.pushU8(0);
        c.push(hash);
      });

    // FINALIZE into the lock: our TRANSFER coin at our own address, funded
    // by one of our coins.
    const transferInput = {
      displayTxid: Buffer.alloc(32, 0x31).toString("hex"),
      vout: 0,
      value: lockValue,
      branch: 0,
      index: 7,
    };
    const owner = ring(transferInput.branch, transferInput.index);
    const transferCoin = new Coin({
      version: 0,
      height: height + 10,
      value: lockValue,
      address: owner.getAddress(),
      hash: prevoutHash(transferInput.displayTxid),
      index: transferInput.vout,
      covenant: transferCov(sha3.digest(script)),
    });
    const lfFunding = { displayTxid: TXID_C, vout: 1, value: 2_000_000, branch: 0, index: 8 };
    const lfRing = ring(lfFunding.branch, lfFunding.index);
    const lfRenewal = Buffer.alloc(32, 0x77);
    const lfFee = 10_000;
    const lf = new MTX();
    lf.version = 0;
    lf.addCoin(transferCoin);
    lf.addCoin(mkCoin(lfFunding.displayTxid, lfFunding.vout, lfFunding.value, lfRing));
    lf.addOutput({ address: lockAddress, value: lockValue, covenant: finalizeCov(lfRenewal) });
    lf.addOutput(Address.fromString(addr(1, 1), "main"), lfFunding.value - lfFee);
    assert.strictEqual(lf.sign([owner, lfRing]), 2, "lock finalize: both inputs are ours");
    assert(lf.verify(), "lock finalize verifies in hsd");
    assert(
      transferCoin.covenant.items[3].equals(lf.outputs[0].address.hash),
      "the FINALIZE pays the program the TRANSFER commits to",
    );
    const lockCoin = Coin.fromTX(lf.toTX(), 0, height + 20);

    // Price steps over the lock coin, each as SD's template builds it (a
    // placeholder TRANSFER output 0, the payment last), each bought once.
    const paymentAddr = addr(0, 9);
    const buyer = ring(0, 6);
    const steps = [
      { price: 300_000_000, lockTimeSecs: 1_783_700_000 },
      { price: 200_000_000, lockTimeSecs: 1_783_786_400 },
      { price: 100_000_000, lockTimeSecs: 1_783_872_800 },
    ].map(({ price, lockTimeSecs }) => {
      const tpl = new MTX();
      tpl.version = 0;
      tpl.addCoin(lockCoin);
      const placeholder = new Output();
      placeholder.covenant.type = T.TRANSFER;
      tpl.outputs.push(placeholder);
      tpl.addOutput(Address.fromString(paymentAddr, "main"), price);
      tpl.setLocktime(lockTimeSecs, true);
      assert.strictEqual(tpl.inputs[0].sequence, 0xfffffffe, "step sequence");
      const prev = Script.decode(script);
      const sighash = tpl.signatureHash(0, prev, lockValue, SIGHASH);
      const signature = tpl.signature(0, prev, lockValue, lockKey.privateKey, SIGHASH);
      const fill = new MTX();
      fill.version = 0;
      fill.locktime = tpl.locktime;
      fill.addCoin(lockCoin);
      fill.inputs[0].sequence = 0xfffffffe;
      fill.inputs[0].witness = Witness.fromItems([signature, script]);
      fill.addOutput({
        address: lockAddress,
        value: lockValue,
        covenant: transferCov(buyer.getKeyHash()),
      });
      fill.addOutput(Address.fromString(paymentAddr, "main"), price);
      assert(fill.verify(), "a purchase of the step verifies in hsd");
      return {
        price,
        lockTimeSecs,
        encodedLocktime: tpl.locktime,
        sighash: sighash.toString("hex"),
        signature: signature.toString("hex"),
      };
    });

    // Cancel: the lock key signs input 0 with ANYONECANPAY|SINGLE, output 0
    // a TRANSFER at the lock address committing to our cancel address; our
    // funding input is signed ALL.
    const CANCEL_SIGHASH = Script.hashType.ANYONECANPAY | Script.hashType.SINGLE;
    assert.strictEqual(CANCEL_SIGHASH, 0x83, "cancel sighash");
    const cancelDest = ring(0, 11);
    const cFunding = { displayTxid: TXID_A, vout: 2, value: 1_000_000, branch: 0, index: 10 };
    const cRing = ring(cFunding.branch, cFunding.index);
    const cFee = 10_000;
    const c = new MTX();
    c.version = 0;
    c.addCoin(lockCoin);
    c.addCoin(mkCoin(cFunding.displayTxid, cFunding.vout, cFunding.value, cRing));
    c.addOutput({
      address: lockAddress,
      value: lockValue,
      covenant: transferCov(cancelDest.getKeyHash()),
    });
    c.addOutput(Address.fromString(addr(1, 2), "main"), cFunding.value - cFee);
    const cancelSig = c.signature(
      0,
      Script.decode(script),
      lockValue,
      lockKey.privateKey,
      CANCEL_SIGHASH,
    );
    c.inputs[0].witness = Witness.fromItems([cancelSig, script]);
    assert.strictEqual(c.sign([cRing]), 1, "cancel: only the funding input is signed ALL");
    assert(c.verify(), "cancel verifies in hsd");

    // The cancel's FINALIZE out of the lock to the cancel address.
    const cancelCoin = Coin.fromTX(c.toTX(), 0, height + 30);
    const cfFunding = { displayTxid: TXID_B, vout: 3, value: 1_000_000, branch: 0, index: 12 };
    const cfRing = ring(cfFunding.branch, cfFunding.index);
    const cfRenewal = Buffer.alloc(32, 0x88);
    const cfFee = 10_000;
    const cf = new MTX();
    cf.version = 0;
    cf.addCoin(cancelCoin);
    cf.inputs[0].witness = Witness.fromItems([script]);
    cf.addCoin(mkCoin(cfFunding.displayTxid, cfFunding.vout, cfFunding.value, cfRing));
    cf.addOutput({
      address: cancelDest.getAddress(),
      value: lockValue,
      covenant: finalizeCov(cfRenewal),
    });
    cf.addOutput(Address.fromString(addr(1, 3), "main"), cfFunding.value - cfFee);
    assert.strictEqual(cf.sign([cfRing]), 1, "cancel finalize: only the funding input is signed");
    assert(cf.verify(), "cancel finalize verifies in hsd");

    return {
      lockFinalize: {
        transferInput,
        fundingInput: lfFunding,
        renewalBlock: lfRenewal.toString("hex"),
        changeAddress: addr(1, 1),
        fee: lfFee,
        vsize: lf.getVirtualSize(),
        signedHex: lf.toRaw().toString("hex"),
        txid: lf.txid(),
      },
      steps: {
        lockCoin: { hash: lf.txid(), index: 0, value: lockValue },
        paymentAddr,
        data: steps,
      },
      cancel: {
        lockCoin: { hash: lf.txid(), index: 0, value: lockValue },
        sighashType: CANCEL_SIGHASH,
        cancelAddress: cancelDest.getAddress().toString("main"),
        fundingInput: cFunding,
        changeAddress: addr(1, 2),
        fee: cFee,
        vsize: c.getVirtualSize(),
        signedHex: c.toRaw().toString("hex"),
        txid: c.txid(),
      },
      cancelFinalize: {
        cancelCoin: { hash: c.txid(), index: 0, value: lockValue },
        fundingInput: cfFunding,
        renewalBlock: cfRenewal.toString("hex"),
        flags: 0,
        claimed: 0,
        renewals: 0,
        changeAddress: addr(1, 3),
        fee: cfFee,
        vsize: cf.getVirtualSize(),
        signedHex: cf.toRaw().toString("hex"),
        txid: cf.txid(),
      },
    };
  })();

  return {
    name,
    nameHash: nameHash.toString("hex"),
    height,
    lockPub: pub.toString("hex"),
    lockScript: script.toString("hex"),
    lockAddress: lockAddress.toString("main"),
    lockCoin: { hash: lockCoinHash.toString("hex"), index: 0, value: lockValue },
    step: {
      price,
      lockTimeSecs,
      encodedLocktime,
      paymentAddr,
      sighash: stepHash.toString("hex"),
      signature: signature.toString("hex"),
    },
    purchase: {
      fundingInput: funding,
      destAddress: dest.getAddress().toString("main"),
      changeAddress: addr(1, 0),
      fee,
      signedHex: p.toRaw().toString("hex"),
      txid: p.txid(),
    },
    finalize: {
      transferCoin: { hash: p.hash().toString("hex"), index: 0, value: lockValue },
      fundingInput: funding2,
      renewalBlock: renewalBlock.toString("hex"),
      flags: 0,
      claimed: 0,
      renewals: 0,
      fee: fee2,
      signedHex: f.toRaw().toString("hex"),
      txid: f.txid(),
    },
    lockPath,
    sell,
  };
})();

// --- P2WSH spend (hsd-generated) -----------------------------------------
//
// One input paying a P2WSH program (SHA3-256 of `<pub> OP_CHECKSIG`) spent
// with SIGHASH_ALL to a plain address, witness [signature, script]. Pins
// `Transaction::sign_p2wsh_input` in src-tauri/src/noncustodial/tx.rs; hsd's
// own interpreter verifies the spend.

const p2wshSpend = (() => {
  const privateKey = Buffer.alloc(32, 0x11);
  const pub = KeyRing.fromPrivate(privateKey).publicKey;
  const script = Buffer.concat([Buffer.from([0x21]), pub, Buffer.from([0xac])]);
  const value = 500_000;
  const fee = 10_000;
  const coin = new Coin({
    version: 0,
    height: 100,
    value,
    address: Address.fromScripthash(sha3.digest(script)),
    hash: prevoutHash(TXID_A),
    index: 1,
  });
  const recipient = addr(0, 0);
  const mtx = new MTX();
  mtx.version = 0;
  mtx.addCoin(coin);
  mtx.addOutput(Address.fromString(recipient, "main"), value - fee);
  const signature = mtx.signature(0, Script.decode(script), value, privateKey, HASH_ALL);
  mtx.inputs[0].witness = Witness.fromItems([signature, script]);
  assert(mtx.verify(), "p2wsh spend verifies in hsd");
  return {
    privateKey: privateKey.toString("hex"),
    script: script.toString("hex"),
    coin: { hash: TXID_A, index: 1, value },
    recipient,
    fee,
    hashType: HASH_ALL,
    sighash: mtx.signatureHash(0, Script.decode(script), value, HASH_ALL).toString("hex"),
    signature: signature.toString("hex"),
    signedHex: mtx.toRaw().toString("hex"),
    txid: mtx.txid(),
  };
})();

// --- assemble vectors ----------------------------------------------------

const vectors = {
  meta: {
    generator: "gen_hsd_vectors.js",
    hsd: require("hsd/package.json").version,
    network: "main",
    coinType: COIN_TYPE,
    account: ACCOUNT,
    mnemonic: MNEMONIC,
    note:
      "Independent known-answer vectors. Rust must match signedHex/txid/sighash/" +
      "covenantRaw byte-for-byte.",
  },

  addresses: [
    { branch: 0, index: 0 },
    { branch: 0, index: 1 },
    { branch: 0, index: 2 },
    { branch: 1, index: 0 },
  ].map(({ branch, index }) => {
    const r = ring(branch, index);
    return {
      path: `m/44'/${COIN_TYPE}'/${ACCOUNT}'/${branch}/${index}`,
      branch,
      index,
      address: r.getAddress().toString("main"),
      keyHash160: r.getKeyHash().toString("hex"),
      pubkey: r.publicKey.toString("hex"),
    };
  }),

  // Reconstructed in Rust by building tx::Transaction directly (no coin
  // selection) — pins serialize/sighash/sign for the crypto core.
  plainSendDirect: plainSend({
    inputs: [{ displayTxid: TXID_A, vout: 0, value: 1_000_000, branch: 0, index: 0 }],
    recipient: { address: addr(0, 2), value: 500_000 },
    change: { address: addr(1, 0), value: 1_000_000 - 500_000 - estFee(1, 2, 1) },
  }),

  // Reproduced in Rust via build_send() with a single coin.
  buildSend1: (() => {
    const value = 1_000_000;
    const amount = 400_000;
    const rate = 1;
    const fee = estFee(1, 2, rate);
    const v = plainSend({
      inputs: [{ displayTxid: TXID_B, vout: 0, value, branch: 0, index: 0 }],
      recipient: { address: addr(0, 2), value: amount },
      change: { address: addr(1, 0), value: value - amount - fee },
    });
    v.params = { amount, rate, fee, change: value - amount - fee, coinValue: value };
    return v;
  })(),

  // Reproduced in Rust via build_send() forcing a 2-input selection.
  buildSend2: (() => {
    const v0 = 600_000,
      v1 = 500_000;
    const amount = 900_000;
    const rate = 1;
    const fee = estFee(2, 2, rate);
    const change = v0 + v1 - amount - fee;
    const v = plainSend({
      inputs: [
        { displayTxid: TXID_A, vout: 0, value: v0, branch: 0, index: 0 },
        { displayTxid: TXID_B, vout: 1, value: v1, branch: 0, index: 1 },
      ],
      recipient: { address: addr(0, 2), value: amount },
      change: { address: addr(1, 0), value: change },
    });
    v.params = { amount, rate, fee, change, coin0: v0, coin1: v1 };
    return v;
  })(),

  // Covenant-bearing full tx (OPEN). Pins covenant output serialization +
  // sighash commitment + signing end-to-end.
  openTx: (() => {
    const value = 1_000_000;
    const fee = estFee(1, 2, 1);
    return covenantTx({
      input: { displayTxid: TXID_C, vout: 0, value, branch: 0, index: 0 },
      covenantOutput: {
        value: 0,
        address: addr(0, 0),
        covenant: cov(T.OPEN, (c) => {
          c.pushHash(NAME_HASH);
          c.pushU32(0);
          c.push(RAW_NAME);
        }),
      },
      change: { address: addr(1, 0), value: value - fee },
      meta: { name: NAME, nameHash: NAME_HASH.toString("hex"), rawName: RAW_NAME.toString("hex") },
    });
  })(),

  openTxMeta: {
    name: NAME,
    nameHash: NAME_HASH.toString("hex"),
    rawName: RAW_NAME.toString("hex"),
  },

  // Raw covenant serializations (type || varint(count) || varbytes items).
  covenants: [
    {
      kind: "open",
      args: { nameHash: NAME_HASH.toString("hex"), rawName: RAW_NAME.toString("hex") },
      raw: cov(T.OPEN, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(0);
        c.push(RAW_NAME);
      }).encode(),
    },
    {
      kind: "bid",
      args: {
        nameHash: NAME_HASH.toString("hex"),
        start: START,
        rawName: RAW_NAME.toString("hex"),
        blind: BLIND.toString("hex"),
      },
      raw: cov(T.BID, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(START);
        c.push(RAW_NAME);
        c.pushHash(BLIND);
      }).encode(),
    },
    {
      kind: "reveal",
      args: {
        nameHash: NAME_HASH.toString("hex"),
        height: HEIGHT,
        nonce: REVEAL_NONCE.toString("hex"),
      },
      raw: cov(T.REVEAL, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(HEIGHT);
        c.pushHash(REVEAL_NONCE);
      }).encode(),
    },
    {
      kind: "redeem",
      args: { nameHash: NAME_HASH.toString("hex"), height: HEIGHT },
      raw: cov(T.REDEEM, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(HEIGHT);
      }).encode(),
    },
    {
      kind: "register",
      args: {
        nameHash: NAME_HASH.toString("hex"),
        height: HEIGHT,
        resource: RESOURCE.toString("hex"),
        renewalBlock: RENEWAL_BLOCK.toString("hex"),
      },
      raw: cov(T.REGISTER, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(HEIGHT);
        c.push(RESOURCE);
        c.pushHash(RENEWAL_BLOCK);
      }).encode(),
    },
    {
      kind: "update",
      args: {
        nameHash: NAME_HASH.toString("hex"),
        height: HEIGHT,
        resource: RESOURCE.toString("hex"),
      },
      raw: cov(T.UPDATE, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(HEIGHT);
        c.push(RESOURCE);
      }).encode(),
    },
    {
      kind: "renew",
      args: {
        nameHash: NAME_HASH.toString("hex"),
        height: HEIGHT,
        renewalBlock: RENEWAL_BLOCK.toString("hex"),
      },
      raw: cov(T.RENEW, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(HEIGHT);
        c.pushHash(RENEWAL_BLOCK);
      }).encode(),
    },
    {
      kind: "transfer",
      args: {
        nameHash: NAME_HASH.toString("hex"),
        height: HEIGHT,
        addrVersion: 0,
        addrHash: ADDR_HASH20.toString("hex"),
      },
      raw: cov(T.TRANSFER, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(HEIGHT);
        c.pushU8(0);
        c.push(ADDR_HASH20);
      }).encode(),
    },
    {
      kind: "finalize",
      args: {
        nameHash: NAME_HASH.toString("hex"),
        height: HEIGHT,
        rawName: RAW_NAME.toString("hex"),
        flags: 0,
        claimed: 0,
        renewals: 3,
        renewalBlock: RENEWAL_BLOCK.toString("hex"),
      },
      raw: cov(T.FINALIZE, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(HEIGHT);
        c.push(RAW_NAME);
        c.pushU8(0);
        c.pushU32(0);
        c.pushU32(3);
        c.pushHash(RENEWAL_BLOCK);
      }).encode(),
    },
    {
      kind: "cancel",
      args: { nameHash: NAME_HASH.toString("hex"), height: HEIGHT },
      // hsd encodes CANCEL as an UPDATE covenant with an empty resource item.
      raw: cov(T.UPDATE, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(HEIGHT);
        c.push(Buffer.alloc(0));
      }).encode(),
    },
    {
      kind: "revoke",
      args: { nameHash: NAME_HASH.toString("hex"), height: HEIGHT },
      raw: cov(T.REVOKE, (c) => {
        c.pushHash(NAME_HASH);
        c.pushU32(HEIGHT);
      }).encode(),
    },
  ].map((c) => ({ ...c, raw: Buffer.from(c.raw).toString("hex") })),

  nameHash: { name: NAME, hash: NAME_HASH.toString("hex") },

  blind: {
    value: BLIND_VALUE,
    nonce: BLIND_NONCE.toString("hex"),
    blind: BLIND.toString("hex"),
  },

  shakedex,

  p2wshSpend,
};

const outPath = path.join(__dirname, "vectors.json");
fs.writeFileSync(outPath, JSON.stringify(vectors, null, 2) + "\n");
console.log(`wrote ${outPath}`);
console.log(
  `  hsd ${vectors.meta.hsd}, ${vectors.addresses.length} addresses, ` +
    `${vectors.covenants.length} covenants`,
);
console.log(`  addr(0,0) = ${vectors.addresses[0].address}`);
console.log(`  buildSend1 txid = ${vectors.buildSend1.txid}`);
console.log(`  openTx txid = ${vectors.openTx.txid}`);
