# Shakedex lock keys are derived from the seed, on a hardened branch of their own

Status: accepted (2026-10-05).

To sell a name through a listing, the seller transfers it to a lock address built from a lock key, and that key alone can cancel the listing or sign new price steps. Shakedex and Bob Wallet generate the lock key at random and keep it in a file or an encrypted blob, so losing that backup strands the name: nobody can cancel, and a lock with no price steps stays stuck until the name expires. We derive each lock key from the profile's recovery phrase instead, at `m/44'/<coin type>'/<account>'/2'/<index>'`, where `index` is the first four bytes of the name hash, big-endian, masked to 31 bits. A name therefore always has the same lock key in a given account. Restoring the phrase restores every lock key without a gap-limit scan of the lock branch: the TRANSFER into a lock spends the name's coin from one of the wallet's own addresses, so the name appears in that address history, and its lock address follows from the name. Finding it there is new work — today's sync does not read address history on the node path — and is part of the selling stage (deferred: see the amendment of 2026-10-08 below). Two devices sharing a seed agree on the key without coordinating.

The branch is hardened end to end so that lock keys can never be computed from the account xpub, and so that they stay apart from the receive (`/0`) and change (`/1`) chains: a `0x84` signature, which anyone can complete, is only ever made by a key the wallet uses for nothing but its locked name. Hardening does not hide listings from someone holding the xpub — the seller's own TRANSFER address and the payment address are ordinary derived addresses — it only keeps the lock keys out of reach.

## Consequences

- Ledger and watch-only profiles cannot sell through Shakedex: the app has no seed to derive from, and hardened children cannot be derived from an xpub.
- The wallet never sends coins to a lock address and signs only price steps and cancels with a lock key. Anyone can still send coins there; sync must not count them as spendable.
- Lock keys are not compatible with Bob's or the CLI's key files. That does not matter to buyers, who only see the lock public key in the listing file. Moving a listing's management into Bob would need a key export, which is not planned.
- Re-listing a name after a cancel reuses its lock key and lock address. That is safe: every earlier price step commits to a lock coin that is already spent. It reveals nothing new either, since the name links the two listings anyway.
- Two names whose hashes agree in bits 1–31 of their first four bytes share a lock key and lock address. They still get separate lock coins, and each price step commits to its own coin, so funds are safe; listing state must be keyed by name and outpoint, never by address. The shared address links the two names publicly.
- Profiles or devices built from the same phrase derive the same lock keys but do not share listing state.
- Recovery derives lock addresses from the names in the wallet's own address history; there is no separate lock-branch scan. Until that scan exists, recovery is by name (amendment of 2026-10-08 below). It recovers the key, not the listing: price steps and the payment address are off-chain, so the saved listing file is the backup for those.
- Golden vector: `docs/specs/2026-10-05-shakedex-name-sales.md` R17.
- The path is permanent once a release has locked a name with it. Changing it later means scanning both the old and the new branch forever.

## Amendment 2026-10-08

The selling stage does not add the address-history scan. A lock is recovered by hand after a restore: the seller types the name, the wallet derives its lock key and adopts the lock only if the name's current owner coin sits at that key's lock address and carries FINALIZE at the name's height (spec R32), or the seller imports the saved listing file. Finding locks on its own, by deriving lock addresses from the names in the wallet's address history, is deferred to a roadmap item. The decision itself is unchanged: the path, the index and the hardening stay as above, and so does every consequence except the timing of the automatic recovery.
