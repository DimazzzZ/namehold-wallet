# Feature specs

One file per feature that spans more than one layer (backend + frontend, or
code + user docs). Named `YYYY-MM-DD-<feature>.md` by the date the spec was
first written. A spec is the source of truth a reviewer checks the code
against; when behaviour changes, the spec changes in the same PR.

Each spec has these sections, in this order:

1. **Summary** — one paragraph, what the user gets.
2. **Terms** — the words the code and UI use, defined once.
3. **Requirements** — numbered `R1`, `R2`, … Each is testable and names the
   code that enforces it and the test that pins it.
4. **Explicitly not enforced** — things a reader might assume the feature
   does but the code does not. This section exists so copy cannot overclaim.
5. **Known gaps** — accepted limitations, with the reason they are accepted.
6. **Pointers** — files, tests, docs, CHANGELOG entry.

| Spec | Status |
|------|--------|
| [2026-09-11 Remote-node connection & broadcast guard](./2026-09-11-remote-node-connection-and-broadcast-guard.md) | Implemented |
| [2026-09-14 Network-derived behaviour](./2026-09-14-network-derived-behaviour.md) | Implemented |
| [2026-09-15 Batch name operations](./2026-09-15-batch-name-operations.md) | Implemented; spec written after the fact |
| [2026-09-15 Per-profile node banner & preflight](./2026-09-15-per-profile-node-banner-and-preflight.md) | Steps 1-2 of 9 implemented — see the spec's own status table |
| [2026-09-20 Multiple bids per name](./2026-09-20-multiple-bids-per-name.md) | Implemented |
| [2026-09-21 Paid name swaps](./2026-09-21-paid-name-swaps.md) | Not implemented — UI withdrawn, see spec |
