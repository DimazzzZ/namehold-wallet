# Testing the non-custodial wallet on regtest

The wallet signs locally and only talks to a node for **reads + broadcast**. To
exercise everything end-to-end you need a local **hsd regtest node with the
address index enabled** (so `getcoinsbyaddress` works).

## 1. Start a regtest node

Install hsd if needed (`npm i -g hsd`, or build from source), then:

```sh
hsd --network=regtest \
    --index-address --index-tx \
    --http-host=127.0.0.1 --api-key=test \
    --no-wallet
```

- Node RPC is now at `http://127.0.0.1:14037` (regtest), API key `test`.
- `--index-address` is **required** (the wallet scans coins by address).
- `--no-wallet` is fine — we are non-custodial; we never use hsd's wallet.

Helper CLI (separate terminal): `hsd-cli --network=regtest --api-key=test rpc <method> [args]`.

## 2. Launch the app

```sh
cd ~/git/namehold-wallet
pnpm install        # first time only
pnpm tauri dev
```

## 3. Configure + create a wallet

1. Onboarding → **Create a new wallet**, network **regtest**. A separate
   **secure window** asks for a passphrase, then shows your recovery phrase —
   confirm backup. (The main React UI never sees the phrase.)
2. Settings → **Node RPC**: URL `http://127.0.0.1:14037`, API key `test`,
   chain source **Local node**.

## 4. Fund it (mine regtest coins to your receive address)

Copy the **Receive Address** from the Wallet page, then mine to it (coinbase
needs 100 blocks to mature, so mine 100+):

```sh
hsd-cli --network=regtest --api-key=test rpc generatetoaddress 110 <receiveAddress>
```

Back in the app, click **Sync** → the spendable balance should appear.

## 5. Plain send

Wallet → **Send HNS** → address + amount → **Review** (fee/change preview) →
**Sign & Broadcast** (unlock in the secure window if locked). Then:

```sh
hsd-cli --network=regtest --api-key=test rpc generatetoaddress 1 <anyAddress>
```

Sync again; the tx shows under Recent transactions.

## 6. Name auction (acquire a fresh name)

In the Owned Names box, type a test name (e.g. `testname`) → **Name actions**:

1. **Open** → broadcast → mine `treeInterval`+1 blocks (regtest treeInterval=5):
   `generatetoaddress 6 <addr>`.
2. **Bid** (e.g. bid `1000000`, lockup `2000000`) → mine through the bidding
   period (regtest biddingPeriod=5): `generatetoaddress 6 <addr>`.
3. **Reveal** → mine through reveal period (regtest revealPeriod=10):
   `generatetoaddress 11 <addr>`.
4. **Sync**, then **Register** (optionally paste records JSON) → mine 1 block.

Verify on the node: `hsd-cli --network=regtest --api-key=test rpc getnameinfo testname`.

## 7. Migration lifecycle (a name you already own)

For a name the wallet owns (after Register, or after a transfer to you):
**Manage** → **Update** (records JSON), **Renew**, **Transfer** (to another
address) → mine `transferLockup` blocks (regtest=10) → **Finalize**. **Cancel**
reverts a pending transfer; **Revoke** burns the name.

Records JSON example for Update/Register:

```json
[{"type":"TXT","txt":["hello world"]},{"type":"NS","ns":"ns1.example."}]
```

## Notes / known caveats

- Covenant **serialization + signing** match hsd v6.1.1 byte-for-byte and are
  unit-tested, but on-chain acceptance of each action is exactly what this
  regtest pass validates — start here before testnet/mainnet.
- If `getcoinsbyaddress` errors, the node was started without `--index-address`.
- REVEAL/REDEEM need the wallet to have **synced** after the BID so it can find
  the bid coin (it matches by the bid address).
- Remote-node broadcast is gated behind `allow_remote_broadcast`; local node
  broadcasts freely.
