# Coding Standards

How code is written in this repo. Tooling enforces formatting and lints; this
file covers what tooling cannot. A documented rule here overrides the generic
code-smell heuristics a reviewer might otherwise apply.

## Gates (must be clean before every commit)

| Layer    | Command (from)                                              |
|----------|-------------------------------------------------------------|
| Rust     | `cargo fmt --check` (`src-tauri/`)                          |
| Rust     | `cargo clippy --all-targets -- -D warnings` (`src-tauri/`)  |
| Rust     | `cargo test` (`src-tauri/`) — CI runs the same tests via `cargo nextest run --manifest-path src-tauri/Cargo.toml --locked` |
| Frontend | `npx tsc -b && npx vite build` (repo root)                  |
| Frontend | `npx vitest run` (repo root)                                |
| Frontend | `npm run lint:format` (prettier), `npm run lint:secure-imports` and `npm run lint:native-title` |

Locally `cargo test` is enough; CI uses nextest for its two-lane split (see
`src-tauri/.config/nextest.toml`).

## Rust layering

- `src-tauri/src/commands/*` are **IO shells**: they lock the DB, build clients,
  call Tauri. They import from `db::queries`, `noncustodial::*`, `providers::*`
  and shared helpers. **New** code must not import helpers from a sibling
  command module; if two commands need the same thing it belongs in
  `db::queries` (data), `noncustodial::*` (pure domain logic), or a small
  dedicated helper module under `commands/` (e.g. `commands/active_profile.rs`).
  Existing imports of `commands::read` (profile resolution, node readiness),
  `commands::sync::open_conn`, `commands::namebase` and
  `commands::secure_prompt` are legacy shared layers — do not add to them.
- Where the shared pieces live instead: a background job opens its connection with `db::connection::open_migrated` (the body behind the legacy `open_conn`); the context a draft-building command starts from (`Ctx`, `load_ctx`, `fee_rate`, `fetch_name_state`, `renewal_block`) is `commands::draft_ctx`; send policy that needs only a node client (`broadcast_network_guard_with_client`) is `noncustodial::rpc`. `tests/shakedex_layering_tests.rs` reads the Shakedex sources and fails on a sibling-command import; add a new module's source to its list.
- **Pure logic is split from IO** so it can be unit-tested without a live node
  or a Tauri `State`: the `*_with_client(&dyn NodeRpc, ...)` pattern for RPC
  code, `*_from_conn(&Connection)` for DB code, `*_pure.rs` modules for
  larger bodies.
- `#[cfg_attr(coverage_nightly, coverage(off))]` goes **only** on IO shells
  (a `#[tauri::command]`, a function that locks `state.db`, spawns a process,
  or hits the network). Never on a pure function — write a test instead.
- `Option` means "unknown", not "false". A tri-state check such as
  `network_check(expected, reported) -> Option<bool>` returns `None` when
  either side is unknown, and callers act only on `Some(false)`. Document the
  `None` meaning at the type or function.
- Errors are `AppError`. A DB failure in a **user-triggered** command is
  returned (`?`), not swallowed into a default. Silent fallbacks
  (`.ok().flatten()`, `unwrap_or_default()`) are allowed only in routing /
  best-effort paths, and the line above them must say why.
- **Fail closed on what the user is asked to trust.** The secure confirmation window's rows and a listing's verdict never default a missing field to `0`, `""` or a guessed reason. Parse a typed struct and refuse ("corrupted draft"), or report "could not check". A purchase summary that read back as zeros would have shown "Total 0 HNS" in the window; a missing renewal height once hid a listing as "expires before finalize". Every field of a node reply such a path reads gets its own `let Some(x) = … else { could not check }`: fixing one field does not fix its neighbours. The renewal height was fixed and the name height three lines above it kept `unwrap_or(0)`, so a node that left it out still showed "the name has changed hands".
- **Cases the UI tells apart are separate enum variants.** If a screen must render two situations differently, the backend sends two variants, never one variant whose `reason` string the screen compares. "No node can check this" (SPV, Explorer) is `Hidden::Unverified`; "the node failed to answer" is `Hidden::CouldNotCheck { reason }`. While they shared a variant, the Market told a user with a full node to get a full node and hid the real reason.
- **A payload written in one module and read in another is a typed struct both use**, e.g. `PurchaseSummary` for a draft's `summary_json`, not `json!{}` on one side and key lookups on the other. A domain state is an enum stored by its spelling through `ToSql`/`FromSql` (`PurchaseState`), not a string compared in several files.
- **A gate that guards a send has one implementation.** Every path that broadcasts calls `noncustodial::rpc::broadcast_gates`; a second copy that "applies the same gates as X" is the smell from §Copy below.
- **`run_sync_steps` also runs in the daemon.** A step added there runs inside `namehold-syncd` too, and `SECURITY.md` guarantees that the daemon never signs or broadcasts. A step that can send takes the `SyncCaller` and does nothing that sends when it is `Daemon` (`shakedex_jobs::Rebroadcast::Never`), with a test that the daemon path makes no send call. The R13 purchase refresh once rebroadcast from the daemon.

## Chain rules

- **A transaction built now is judged at `tip + 1`.** hsd checks a mempool transaction at the next block's height, so a maturity or lockup is over once `tip + 1` reaches it, not `tip`. Use the helper (`NameParams::blocks_until_finalize`); a new rule of this kind becomes a `NameParams`/`Network` method with a test on both sides of the boundary (the last block refused, the first accepted). This has gone wrong twice: the coinbase maturity filter (2026-09) and the Shakedex Finalize (2026-10) both held coins or names one block longer than hsd does.
- **A rule the spec puts on an input file is checked when the file is read.** Signature form (65 bytes, low-S, sighash byte), sizes and address shapes are refused by the parser at import, not left to a later on-node verification that turns them into "failed verification". A high-S listing file once imported because only verification checked low-S.
- **A foreign file keeps what it says apart from what it means.** A field the parser interprets (a `feeAddr` beside zero fees means "no fee output") is stored twice: the interpreted value the code uses, and the value as written for the round-trip. Unknown fields are kept at every level, inside each price step as well as at the top. Clearing the interpreted field made `to_json` write `feeAddr: null`, and a step's unknown fields were dropped.
- **Network numbers come from `noncustodial/network.rs`, never from memory.** Those values are pinned against hsd `lib/protocol/networks.js` by `network_tests`. Code, test comments and docs take them from there. Mainnet and testnet share some values (the transfer lockup is 288 on both; regtest is 10, simnet 5), which is exactly what a guess gets wrong: the user manual and a test comment both once said "10 on testnet, 5 on regtest".

## Rust tests

- Unit tests live in `src-tauri/src/tests/<area>_tests.rs`, registered in
  `src-tauri/src/tests/mod.rs` in alphabetical order. Tests that need private
  access to a module's internals may live in a `#[cfg(test)] mod` inside it.
- Test names state the behaviour: `check_node_connection_flags_a_cross_network_node`,
  not `test_probe_2`.
- Mock the node with `tests/mock_node_rpc.rs::MockNodeRpc`; build a DB with
  `Connection::open_in_memory()` + `crate::db::migrations::run(&conn)`.
- Tests that depend on randomness must assert on the actual value they
  produced, never on a forced one (see `cookie_vault.rs::flip_hex_digit`).
- When a helper is moved, its tests move with it. Never delete a test to make
  a refactor compile.
- **Process-global state needs a serial key.** `std::env::set_var` /
  `remove_var` change the variable for the whole process, and the harness runs
  tests as threads of one process — so a test that swaps `HOME` races every
  test that reads it, including indirectly. Put `#[serial(<key>)]`
  (`serial_test`) on the writer **and on every reader**, sharing one key per
  variable. Rust 2024 marks these functions `unsafe` for exactly this reason.
  Cost of getting it wrong: `test_hsd_candidates_includes_home_paths` failed
  about one full run in twenty and passed alone every time.
- **Wait for content, never for the file.** `fs::read_to_string(p).ok()` is
  `Some("")` the instant a file exists, and a writer that redirects (`echo x >
  f`) creates and truncates before it writes a byte. A poll loop keyed on
  `Some(_)` therefore exits on the empty window and asserts against nothing.
  Treat an empty read as "not ready" (see `node_lifecycle_tests::recorded_argv`).
- A test that passes alone and fails in the suite is a defect in the test, not
  a reason to retry it. Find the shared state; the two rules above are the two
  ways it has bitten so far.

## TypeScript ↔ Rust bridge

- A Rust struct and its TS mirror must agree field-for-field on spelling. New
  structs use `#[serde(rename_all = "camelCase")]` (e.g. `NodeConnectionCheck`);
  a few legacy models (`Asset`, `Settings`) are snake_case on both sides — do
  not "fix" them, the frontend depends on the spelling. Command **arguments**
  stay snake_case (`api_key`), matching Tauri's argument mapping.
- The TS mirror lives in `src/types/index.ts`. Adding a field means: Rust
  struct + TS interface + **every** test fixture that builds that object (grep
  the tests for a neighbouring field, e.g. `synced:`).
- Tri-state fields are `Option<bool>` ↔ `boolean | null`. No enums across the
  bridge for a three-valued flag.

## Frontend

- Behaviour shared by two screens lives in one hook (`useNodeConnectionCheck`)
  or one module (`lib/connectionMode.ts`), so the screens cannot drift on what
  a word like "connected" means.
- Untyped form state is cast **once** into a typed local at the top of the
  component, not at every use.
- **A number from a foreign file is untrusted at render time.** A listing's u64 fields (`expiresAt`, `lockTime`, prices) can exceed what `Date` or a safe JS integer holds, and a `RangeError` thrown while rendering takes down the whole page. Format them through a helper that returns `null` for a value it cannot represent (`listedUntilText`), with a test at the u64 maximum.
- **A backend refusal is mirrored at every entry point.** When the backend refuses a profile kind or a mode, every button that leads there is disabled with the same sentence — the list's Buy, not only the dialog's confirm. The sentence and the predicate are one constant and one function both screens import (`RECOVERY_PHRASE_ONLY`, `canUseShakedex`), and the backend error uses the same words. Work from the backend gate function, not from memory: every `return Err` in it (`commands/shakedex.rs::software_writer_ctx` checks the profile kind **and** whether the node can send) is one input of the predicate (`canUseShakedex(kind, writeCap)`) and one sentence (`shakedexRefusal`). Mirroring only the profile kind left Buy and Finalize enabled on a node that cannot send. Disable the button with the reason, never hide it: a hidden Finalize tells a Ledger user nothing.
- **A state the spec says a screen shows reaches that screen.** A column that no command returns is not "shown". When the spec says "Activity says nothing was paid", a test renders Activity and finds those words. The lost reason sat in `shakedex_purchases.lost_reason`, which no screen read, so a lost purchase silently disappeared. Activity shows a draft's `errorMessage` as its status hint, which makes a draft the usual way to tell the user why something ended.
- Hover hints use the `Tooltip` component, never the native `title` attribute
  — `npm run lint:native-title` enforces it. A `title` cannot be styled or
  positioned, has no delay we control, and is **never shown on an element with
  `pointer-events: none`**, which every disabled `Button` sets: the hint
  disappears exactly when it has something to say. `title` remains an ordinary
  prop on `Dialog`, `PageHeader`, `Card`, `Alert` and `EmptyState` (a heading,
  not a hint) and on `Badge`, which renders a Tooltip itself.
  - The Tooltip's trigger wrapper is a real element — floating-ui measures it,
    so it cannot be `display: contents`. It defaults to `inline-flex`; pass
    `className` (which **replaces** that default) whenever the parent's layout
    depends on the trigger, e.g. a block list item or a `flex-1` / `shrink-0`
    flex child.
  - A Tooltip renders a `span`, so it cannot wrap a `<td>` or `<tr>`. Wrap the
    cell's contents instead.
  - When the `title` was also the element's accessible name (an icon with no
    text), add an `aria-label` — the Tooltip does not name the trigger.
- Components use `data-testid` for anything a test clicks or asserts on.
  Tests use Vitest + Testing Library, mock `invoke` per command name, and
  build **complete** fixtures (every field of the type, no partials).
- A test that pins a contract the app cannot currently reach (e.g. a state
  the backend never produces on that screen) says so in a comment above the
  `it(...)`.

## Copy, comments and docs

- UI text, doc comments, `CHANGELOG.md` and `docs/*.md` may only claim what
  the code **enforces**. "X will be refused" requires a code path that
  refuses X. If only reads are gated, say reads.
- A comment that points at another file (`(read.rs)`) is a sign the rule
  should be extracted and shared instead.
- Every user-visible change gets a bullet under `## [Unreleased]` in
  `CHANGELOG.md`. Node / connection behaviour is documented in
  `docs/NODE_SETUP.md`; security-relevant behaviour in `SECURITY.md`.
- Features that span backend + frontend + docs get a spec in `docs/specs/`
  (see `docs/specs/README.md`), written before or with the code, and kept
  current when behaviour changes.
- A spec's *Pinned:* lines name tests that exist. When the pinned behaviour lands in a test with another name, or a test is renamed, the *Pinned:* line changes in the same PR.
- A spec's *Enforced:* line that names two places asks for defence in depth: each place holds its own check and its own test. R16 named both `commands/tx.rs` and `providers/ledger/signing.rs`; only the first refused a Shakedex plan, so any other path to the Ledger signer would have signed it wrongly.
- A script that builds against another repository pins the full 40-character commit hash and checks `git rev-parse HEAD` after the checkout. An abbreviated hash is not a pin, because a later commit can share its prefix.

## Commits

- Subject: `type: summary` with `feat` / `fix` / `refactor` / `test` / `docs`
  / `build` / `chore` / `format`. Body explains **why** and what a reviewer
  should look at; list the gates you ran when the change is non-trivial.
- The type list is closed: no `perf`, `style` or `ci`. A speed-up is `refactor` (or `fix` when the slowness was a bug), a CI change is `build`. With squash merges the PR title is what lands on `main`, so it takes a type from the same list.
- One logical change per commit. Review follow-ups may be bundled when the
  message enumerates them, but a test-only refactor unrelated to the subject
  is its own commit.
- No tool-attribution trailers.

## Review

Changes are reviewed on two axes: **Standards** (this file plus the Fowler
smell baseline as judgement calls) and **Spec** (the feature's
`docs/specs/*.md`). A finding that says "either fix the code or soften the
claim" is resolved toward whichever the product intent actually is — and the
spec is updated to record the choice.

A pull request carries one feature. Tooling, CI and process changes that the feature's spec does not ask for (a commit-message linter, a new CI job) go in their own PR from a fresh `origin/main`. Rules learned while reviewing the feature, which cite its code, stay with it. Splitting a branch afterwards means rewriting its history, which is the same trap as renaming a commit.

Before asking for review, check the diff for the findings that keep coming back:

- Every button a test clicks or asserts on has a `data-testid`, and the test finds it by that (§Frontend).
- Every `unwrap_or*` / `.ok()` fallback has a "why" line above it, and none sits on a path the user is asked to trust (§Rust layering).
- Height comparisons for a transaction built now use `tip + 1` (§Chain rules).
- Every network-specific number in code, tests and docs is checked against `network.rs` (§Chain rules) — including numbers quoted from a review finding, which are as fallible as memory.
- Every number read from a foreign file and rendered goes through a helper that tolerates any u64 (§Frontend).
- Every `return Err` in a backend gate function has its disabled twin at each UI entry point (list, dialog, row action), with the same sentence, and the button is disabled rather than hidden (§Frontend).
- Every field of a node reply on a trusted path (verdict, confirmation rows) is matched one by one; none has `unwrap_or` (§Rust layering).
- Nothing in the UI compares a backend `reason` string; two cases it renders differently are two variants (§Rust layering).
- A new step in `run_sync_steps` sends nothing when the caller is the daemon (§Rust layering).
- Every terminal state a user must learn about (lost, refused, expired) has a test on the screen that tells them (§Frontend).
- Every place an *Enforced:* line names holds the check, with a test there (§Copy).
- A foreign file round-trips unchanged: interpreted fields are kept as written as well, and unknown fields are kept at every level (§Chain rules).
- External repositories are pinned by their full commit hash, and the script checks it after the checkout (§Copy).
- The PR holds no tooling or CI change that the spec does not ask for (§Review).
- Every check the spec puts on an input file runs in the parser (§Chain rules).
- No new import of a sibling command module or a legacy shared layer (§Rust layering).
- Every *Pinned:* line in the spec names a test that exists (§Copy).
- Every doc comment and doc line claims only what the code enforces (§Copy).
- Every commit subject uses a type from the closed list (§Commits).
- The spec the code cites (R-numbers, CONTEXT.md terms) is on `main` or merges first.
