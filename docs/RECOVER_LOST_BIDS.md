# Recovering lost Handshake bids

If you placed a Handshake auction bid and lost the local wallet data before
revealing it, Namehold can recover the bid from your seed phrase alone — even
if the bid was originally placed in a different hsd-compatible wallet.

---

## The problem

Handshake auctions use **sealed bids**: the blockchain stores only a `blind`
(a hash of your bid value + a nonce), never the value itself. To **reveal**
your bid (mandatory — miss the reveal window and the locked-up HNS is
forfeit), your wallet must reproduce the exact value and nonce that produced
the on-chain blind.

Wallets store this value + nonce in a local database row. If that row is lost
(reinstall without backup, disk failure, seed-restore into a fresh install,
migration between wallets), the reveal becomes impossible — and the lockup is
gone when the reveal window closes.

---

## Why Namehold can recover it

The nonce derivation is the **hsd standard** — it's deterministically computed
from:

- Your account **xpub** (derived from your seed phrase)
- The **name hash** (public, on-chain)
- The bid **address hash** (public, on-chain in the BID output)

Given these three inputs, the nonce is fixed. The only remaining unknown is
the **bid value** — and that's bounded above by the coin's **lockup** (also
public on-chain). So Namehold can brute-force the value: try candidates,
recompute the blind, and compare against the on-chain blind until it matches.

Because this derivation is the hsd standard (not a Namehold-specific format),
**recovery works for bids placed in any hsd-compatible wallet** — not just
Namehold.

---

## Prerequisites

- Your **BIP39 seed phrase** (12 or 24 words).
- The bid must still be in the **REVEAL phase** on-chain — the reveal window
  hasn't closed yet. Once it closes, the lockup is forfeit at the protocol
  level and no wallet can recover it.
- A synced hsd node (Namehold needs to see the on-chain BID coin).

---

## Step-by-step

1. **Import your seed into Namehold.**
   Open the app → "Create or import wallet" → enter your BIP39 mnemonic. Set
   a passphrase (required for signing later). The wallet profile is created.

2. **Sync the wallet.**
   Click **Sync** (or wait for the background sync daemon to run). Namehold
   discovers the on-chain BID coin for the name you bid on.

3. **Open the name.**
   Find the name in the **Auctions** page or **Owned Names** list. Click it
   to open the **Name Actions** modal.

4. **The Recover bid panel appears automatically.**
   When Namehold detects a BID coin in the REVEAL phase with no local
   commitment row, it shows a gray "Recover bid" panel where the Reveal
   button would normally be. You don't need to enable anything — it just
   appears when the situation calls for it.

5. **Recover — two options:**

   **a. You remember the exact amount you bid:**
   Type the HNS value into the "Your bid amount (HNS)" field and click
   **Recover bid**. The app derives the nonce, checks the blind, and writes
   the commitment row. Instant.

   **b. You don't remember the amount:**
   Click **Auto-recover (brute-force)**. The button changes to "Searching…"
   while the app sweeps candidate values:

   - **Tier 1 — round values:** tries every whole HNS, then every 0.1 HNS,
     then every 0.01 HNS increment up to the coin's lockup. Almost every
     real-world bid is a round number, so this wins fast.
   - **Tier 2 — full sweep:** if Tier 1 misses, a parallel scan of every
     dollarydoo (10⁻⁶ HNS) value up to the lockup runs. Typical bids under
     100 HNS finish in seconds on a modern machine.

   On success, a toast shows the recovered amount (e.g. "Bid recovered
   (5.5 HNS) — you can reveal now.").

6. **Reveal.**
   The Reveal button is now unlocked. Click it, sign in the secure window,
   and broadcast the reveal transaction before the window closes.

---

## Limits

- **Lockups above 1000 HNS:** the full sweep is capped for safety (10⁹
  candidates). For very large lockups, enter the known value instead.
- **Recovery uses only the account xpub** (public) — it never needs your
  passphrase or unlocks the signer. Works on watch-only profiles.
- **Idempotent:** running recovery a second time on an already-recovered bid
  succeeds and does nothing (no duplicate rows, no re-broadcast).

---

## FAQ

**Do I need my original wallet installed?**
No. Just the seed phrase. Namehold reconstructs everything from the seed +
the on-chain data.

**Does this work for bids I placed years ago?**
Only if the bid is still in the REVEAL phase. If the reveal window already
closed, the HNS is lost at the protocol level — no wallet can recover it.

**Can I recover multiple bids?**
Yes, one name at a time. Each recovery is independent.

**What if Auto-recover says "bid lockup is too large"?**
The lockup exceeds the sweep cap (~1000 HNS). Use the "Enter the amount"
path instead — you'll need to recall (or narrow down) the value you bid.

**What if it says "no unspent bid coin found"?**
Either the wallet hasn't synced yet (click Sync and try again), or the bid
was already revealed/redeemed (nothing to recover).

**What if it says "bid value doesn't match" or "no bid value reproduces the
on-chain blind"?**
The seed you imported doesn't match the wallet that placed the bid. Double-
check you're using the correct mnemonic.
