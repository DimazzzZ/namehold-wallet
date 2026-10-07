# Implement Shakedex ourselves; no dependency on `hns-swap`

Status: accepted (2026-10-05). Number 0002 is skipped because "ADR-002" already refers to the network-derived behaviour spec (`docs/specs/2026-09-14-network-derived-behaviour.md`).

Buying and selling names through Shakedex listings needs a lock script, P2WSH spends, `ANYONECANPAY|SINGLEREVERSE` signatures, and the Shakedex listing file (v2 JSON) that LearnHNS Market and Bob exchange. A Rust crate, `hns-swap` 0.5.0, already implements the transaction primitives, and an audit (2026-10-05) found it correct: it reproduces the hsd golden vector byte for byte, verifies all 236 live LearnHNS listings that still have a coin, checks the sighash byte strictly, and does not panic under fuzzing. We still write our own primitives on top of `noncustodial/tx.rs`, and the wallet does not depend on `hns-swap` at all — not at runtime and not in tests. The code sits in the path that signs money, and the crate has one author and no reviewers: most of it appears to be AI-generated over ten weeks, its author keeps value movement switched off in his own wallet crate built on it, and every new release would need a fresh audit from us. It also lacks the parts we need most — the Shakedex v2 JSON format and `expiresAt` — so after our own JSON layer and the glue to `DraftPlan` the net saving is roughly 0–150 lines.

## Considered options

- **Depend on `hns-swap` at runtime, pinned.** Rejected for the reasons above.
- **Fork it and depend on our fork.** Rejected: we would maintain someone else's codebase plus the k256 stack (+20 crates) in our signing path, for little saving.
- **Keep it as a dev-dependency for differential tests.** Considered and rejected for now. The hsd golden vector and the live LearnHNS listings already give an independent reference.

## Consequences

- We contribute the missing pieces (v2 JSON with `expiresAt`, and accepting `feeAddr` when `fee == 0`) upstream as a pull request, as a contribution to the ecosystem, not as a step toward adopting the crate.
- A test fails if `Cargo.lock` ever contains `hns-swap`, so the decision cannot be undone by accident.
- Correctness rests on our tests: the hsd v8.0.0 golden vector, strict verification of live LearnHNS listings, and a regtest end-to-end purchase from a listing created by the real shakedex CLI.
- Revisit if the crate gains independent reviewers and its author enables value movement in production, or if keeping our implementation in step with the protocol becomes costly.

Audit: `.mimocode/research/hns-swap-crate-audit.md` (internal knowledge base, not in git). Protocol: `.mimocode/research/shakedex-protocol.md`.
