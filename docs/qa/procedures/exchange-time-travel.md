# exchange-time-travel

Bring one chain's median time past (MTP) beyond a trade's refund time, so the refund path of that chain's lock opens. Both locks are time-locked by MTP, never by height or by a computer's clock (spec §3.1, R11): a refund is accepted by the node, and sent by Namehold or the watchtower, only once the chain's `mediantime` is past the refund time.

- **Parameters:** the chain (`hns` or `btc`); the target `T`, a Unix time (exact: `terms_times $TERMS` from [exchange-setup](exchange-setup.md#variables-and-helpers); approximate: the trade card's `HNS can go back to the seller from` / `BTC can go back to the buyer from`, minute precision, UTC).
- **Blocks mined:** 11 on that chain (6 is the least that moves the median; 11 replaces all of it).

## The rule both chains follow

The MTP is the median of the last 11 block timestamps. A refund is final once:

- **HNS**: `512 × units < MTP`, where the lock time is `2147483648 + units` and `units = ceil(hnsRefundTime / 512)` (R2: up to 511 s later than the refund time itself);
- **Bitcoin**: `btcRefundTime < MTP` (BIP 113).

So aim for an MTP at least 600 s past `T` on Bitcoin and 1 200 s past `T` on HNS, and mine 11 blocks at or after that time. Check with `mtp` (or `H getblockchaininfo` / `B getblockchaininfo` → `mediantime`).

Some steps of the engine also read **this computer's clock** (spec R8: the reveal deadline by both clocks; the Buyer gives up waiting for the Seller only after `Seller locks HNS by` has passed in real time). Moving a chain's time never moves those: a case says when it needs real time to pass as well.

## Path R — real time (the default; always allowed)

1. Wait until the real clock (`date +%s`) is past `T + 600` (Bitcoin) or `T + 1200` (HNS).
2. Mine 11 blocks: `bmine 11` (Bitcoin) or `mine 11` (HNS). Each new block carries the real time, so the MTP lands at about now.
3. `mtp`: the chain's `mediantime` is past `T` (by the margin above).

An HNS refund comes about 60 minutes after the trade starts and a BTC refund about 120 minutes after (regtest policy, `noncustodial/exchange/policy.rs`). Run the refund cases side by side so one wait serves them all ([README](../README.md#exchange-run-order)).

## Path M — mocktime (minutes instead of hours)

- **Bitcoin**: the run's own bitcoind (`.regtest/bitcoin`, used only by these cases) may be given a mocktime.
- **HNS**: `setmocktime` on the shared hsd is forbidden by [rules.md](../rules.md#the-regtest-chain) ("a forward clock offset changes the chain for every later run"). Use path M on HNS **only** when the maintainer has agreed to it for this run, and write that in the report. Without that agreement, use path R for HNS.
- **HNS time locks count in 512-second units** (`hns_htlc::time_lock_secs`): an HNS refund opens at the next multiple of 512 s at or after `hnsRefundTime`, up to 8.5 minutes later than the card's "HNS can go back to the seller from". Take the HNS chain's median time at least 600 s past `hnsRefundTime` (path M's `T + 1200` does), or the refund rightly stays unsent.
- **The limit**: never set a mocktime more than 100 minutes ahead of the real clock. Both nodes refuse a block more than 2 hours ahead of their own clock (`time-too-new`); once the mocktime is reset, the next blocks are stamped just after the MTP, so a larger jump leaves a chain on which no block can be mined until real time catches up. When the target is further than that, first wait in real time (path R's step 1) until it is within 100 minutes.

Steps:

1. `now=$(date +%s)`; choose `M = T + 600` (Bitcoin) or `M = T + 1200` (HNS); check `M - now ≤ 6000`.
2. Set the clock:
   - Bitcoin: `bash scripts/btc-regtest.sh mocktime $M`. Bitcoin Core's mocktime is frozen: blocks are stamped `M`, then `M+1`… as the MTP catches up.
   - HNS (agreed only): `H setmocktime $M`. hsd keeps an offset: its clock reads `M` now and keeps running from there.
3. Mine 11 blocks on that chain (`bmine 11` / `mine 11`).
4. `mtp`: that chain's `mediantime` ≥ `M - 1`, so past `T`.
5. Reset the clock at once:
   - Bitcoin: `bash scripts/btc-regtest.sh mocktime 0` (0 means the system clock in Bitcoin Core).
   - HNS: `H setmocktime $(date +%s)`. **Never `setmocktime 0` on hsd**: hsd reads 0 as the year 1970 (it sets the offset to minus now), not as "the real clock".

   Restarting a node also drops its mocktime, but not the timestamps already mined.
6. Let the app and the watchtower see it: the daemon and the watchtower step every 60 s; on the active profile `Refresh` on the Exchange page runs a step at once.

## After a jump

The blocks mined under a mocktime stay in the future until real time catches up (up to 100 minutes). Until then, on that chain:

- **HNS**: a new trade may never reveal: the Buyer reveals only while `MTP + 15 min < HNS refund time` (R8), and a trade's refund time is its real start + 60 min. **Do not start a trade** while `mtp` shows the HNS `mediantime` ahead of now. Shakedex price steps (judged by MTP, [mine-and-sync](mine-and-sync.md#making-a-price-step-valid-median-time)) would also become valid early: no Shakedex case may run either.
- **Bitcoin**: a new trade's BTC refund would open early, shortening the Seller's claim window. Do not start a trade while the Bitcoin `mediantime` is ahead of now.

So run the cases that travel last ([README](../README.md#exchange-run-order)), and record `mtp` before and after each jump in the report.

## End state

That chain's MTP is past `T`; its mocktime is reset; the jump (path M) or the wait (path R) and the 11 blocks are recorded.
