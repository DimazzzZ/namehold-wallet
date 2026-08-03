# Namehold ↔ hsrd: где должна проходить upstream-граница

## Область исследования

Отчёт сопоставляет:

- Namehold PR #21, commit [`9d168a6`](https://github.com/DimazzzZ/namehold-wallet/tree/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2);
- `handshake-rs/hns-node-rs` v0.3.4, commit [`40b456f`](https://github.com/handshake-rs/hns-node-rs/tree/40b456fa0772729542118a69f27edc37bf42a3d7) (wallet-related files не изменились на текущем `main` при проверке);
- `handshake-rs/hns-rs` на закреплённой PR-ом ревизии [`15f7155`](https://github.com/handshake-rs/hns-rs/tree/15f715576a2111fae2a8c65fccc7860ede64bd98).
- текущий `hns-rs/main` [`4b989aa`](https://github.com/handshake-rs/hns-rs/tree/4b989aabc132e7e79b8fd57a10f2465073faf588), чтобы не предлагать в Namehold то, что уже появилос upstream после pinned commit.

Источники — только официальные исходники и документация этих репозиториев.

## Краткий вывод

Совместный пилот имеет сильный смысл. `hsrd` прямо заявляет, что он не кошелёк, а его wallet index/RPC ждут независимый wallet adapter и cross-project qualification. Namehold — естественный первый consumer для этой проверки. [README hsrd](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/README.md#L7-L29), [wallet readiness](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/readiness.md#L137-L196)

Но пилот сейчас обнаружил неправильную границу модулей: Namehold взял на себя слишком много общего протокольного кода: private wire DTO, cursor pagination, snapshot binding, retry classification, proof/evidence validation и final-fee assessment. Это делает `hsrd.rs` мелким по интерфейсу, но неглубоким по реализации: сложность upstream-протокола протекает в application layer.

Целевая граница:

```text
Namehold
  keys, derivation, DB, user policy, confirmation, UI, node lifecycle
      |
      |  3 typed operations
      v
hns-wallet-client (upstream adapter; deep module)
  atomic restore, bound evidence, signed-tx facts
      |
      v
hsrd Wallet RPC v1
  canonical node/index/mempool authority

hns-rs
  canonical transaction/covenant/proof/swap semantics used by the adapter
```

Правило границы:

- **Upstream владеет инвариантами, которые одинаковы для любого wallet consumer**: wire schema, exact auth format, script identity, sorting/deduplication, cursor lifecycle, chain/mempool binding, canonical decode, txid/proof verification, typed errors и node-calculated fee facts.
- **Namehold владеет решениями и состоянием конкретного приложения**: seed/keys, derivation records, SQLite schema, sync schedule, coin reservations, max-fee/user approval, permission to use a remote broadcaster, action labels, presentation, Tauri security и lifecycle sidecar.

Это даёт лучшую **locality**: node facts остаются рядом с node/index authority, wallet policy — рядом с user workflow. Узкий **seam** даёт высокий **leverage**: один исправленный adapter защищает все будущие wallets.

## Что уже правильно находится upstream

### `hns-node-rs/crates/hns-wallet-index`

Wallet index уже владеет правильными низкоуровневыми инвариантами:

- `ScriptId` есть BLAKE2b-256 от canonical output-address encoding;
- history, UTXO и spender rows записываются в том же batch, что и canonical chain state;
- connect/disconnect/reorg имеют одну publication boundary;
- confirmed result связан с chain epoch и sorted script set, mempool result — ещё и с process nonce/generation.

Это нужно оставить здесь: только node имеет нужную locality к UTXO snapshot, block undo и atomic store batch. [`ScriptId` и history types](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-wallet-index/src/lib.rs#L149-L291), [`stage_connect` / `stage_disconnect`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-wallet-index/src/lib.rs#L408-L604), [atomicity documentation](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/HNS_NODE_WALLET_INDEX.md#L20-L51)

### `hns-node-rs/crates/hns-node/src/wallet_backend.rs`

Typed backend уже хорошо задаёт node-side interface. `get_confirmed_scripts_page` принимает **весь** sorted-unique set, привязывает cursor к его digest и epoch; `get_mempool_scripts_activity` делает один global scan по тому же набору. [`ConfirmedScriptsPage` types](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/wallet_backend.rs#L91-L182), [`get_confirmed_scripts_page`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/wallet_backend.rs#L806-L989), [`get_mempool_scripts_activity`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/wallet_backend.rs#L1149-L1270)

Node также правильно владеет `quote_transaction_fee`: он сам разрешает input coins из одного bound chain/mempool snapshot и считает actual/minimum fee, weight и sigops. Клиент не может подсунуть свои coins или derived sizing evidence. [`quote_transaction_fee`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/wallet_backend.rs#L1465-L1516), [fee quote contract](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/WALLET_RPC_V1.md#L139-L195)

### `hns-rs`

`hns-rs` уже является правильной protocol authority для runtime-independent semantics:

- canonical transaction encoding/decoding и txid: [`hns-transaction`](https://github.com/handshake-rs/hns-rs/blob/15f715576a2111fae2a8c65fccc7860ede64bd98/crates/hns-transaction/src/lib.rs#L214-L355);
- canonical NameState/resource codecs: [`name_state.rs`](https://github.com/handshake-rs/hns-rs/blob/15f715576a2111fae2a8c65fccc7860ede64bd98/crates/hns-covenants/src/name_state.rs#L23-L301);
- strict Urkel proof verification: [`hns-urkel-proof`](https://github.com/handshake-rs/hns-rs/blob/15f715576a2111fae2a8c65fccc7860ede64bd98/crates/hns-urkel-proof/src/lib.rs#L18-L93), [compatibility contract](https://github.com/handshake-rs/hns-rs/blob/15f715576a2111fae2a8c65fccc7860ede64bd98/docs/urkel-proof-compatibility.md);
- exact HSD fee-policy arithmetic: [`policy.rs`](https://github.com/handshake-rs/hns-rs/blob/15f715576a2111fae2a8c65fccc7860ede64bd98/crates/hns-script/src/policy.rs#L1-L190);
- canonical swap/HTLC/marketplace messages: [`hns-swap`](https://github.com/handshake-rs/hns-rs/blob/15f715576a2111fae2a8c65fccc7860ede64bd98/crates/hns-swap/src/lib.rs), [`hns-marketplace-protocol`](https://github.com/handshake-rs/hns-rs/blob/15f715576a2111fae2a8c65fccc7860ede64bd98/docs/marketplace-protocol.md).

Эти типы не нужно копировать в Namehold. PR правильно пинит все нужные crates на один commit. [`Cargo.toml`](https://github.com/DimazzzZ/namehold-wallet/blob/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2/src-tauri/Cargo.toml#L70-L79)

### Важное изменение после pinned `hns-rs` revision

PR пинит `15f7155`, но текущий `hns-rs/main` на `4b989aa` уже содержит strict typed TRANSFER/FINALIZE parsing, output/transaction builders и verifiers. `build_transfer_output` сохраняет locked value/owner address и canonical covenant linkage; `build_finalize_output` требует authenticated `NameState` и renewal block. [`hns-transaction/name.rs`](https://github.com/handshake-rs/hns-rs/blob/4b989aabc132e7e79b8fd57a10f2465073faf588/crates/hns-transaction/src/name.rs#L1-L206) Та же current revision добавляет canonical Shakedex fulfillment/recovery construction и spend classification поверх typed TRANSFER helpers. [`hns-swap/src/lib.rs`](https://github.com/handshake-rs/hns-rs/blob/4b989aabc132e7e79b8fd57a10f2465073faf588/crates/hns-swap/src/lib.rs#L547-L805)

Следовательно, любые Namehold-local TRANSFER/FINALIZE builders будут неправильной границей: их нужно брать из `hns-rs`. Но перепинивать PR прямо на current `main` пока рано: upstream changelog прямо говорит, что `0.2.0` ещё unreleased/untagged, а fee/name-transition/recovery additions остаются source/fixture-reviewed до consolidated qualification gate. [`CHANGELOG.md`](https://github.com/handshake-rs/hns-rs/blob/4b989aabc132e7e79b8fd57a10f2465073faf588/CHANGELOG.md#L1-L40) Правильная последовательность: дождаться qualified tag/commit, затем перепинить весь canonical crate set одним изменением.

## Главный недостающий deep module

`wallet_rpc.rs` сейчас делает wire projection, но его request/response types private. Поэтому Namehold вынужден вручную повторять более двух десятков `Wire*` structs и stringly-typed error/status values. [`wallet_rpc.rs` request enum](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/wallet_rpc.rs#L37-L148), [private wire projections](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/wallet_rpc.rs#L922-L1176), [Namehold duplicate DTOs](https://github.com/DimazzzZ/namehold-wallet/blob/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2/src-tauri/src/noncustodial/hsrd.rs#L1368-L1656)

Протокол при этом требует нетривиальной композиции: полный sorted script set, полный discard при смене epoch, проверка tip на каждой странице, mempool nonce/generation, opaque cursors и bound point reads. Сама документация говорит, что client должен отбрасывать partial results при любом изменении binding. [`WALLET_RPC_V1.md`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/WALLET_RPC_V1.md#L96-L137)

Это не должно быть каждый раз заново реализовано wallet-приложением. Нужен один upstream **adapter** как глубокий модуль: узкий typed interface скрывает pagination/binding/retry/validation.

### Где разместить

Предпочтительно не в `hns-node` binary crate и не в Namehold, а в малом библиотечном crate/repository, например:

- `hns-wallet-rpc-types`: public v1 envelopes, calls, results, bounds и typed error codes; этот crate должен быть единственным wire-schema owner и использоваться и server, и client;
- `hns-wallet-client`: transport-independent orchestration над `WalletTransport`, с optional `reqwest` implementation;
- canonical decode/proof/fee types берутся из одной pinned/tagged ревизии `hns-rs`.

Если не хочется создавать отдельный repo, оба crates могут жить в workspace `hns-node-rs`, но они должны быть publishable и не зависеть от RocksDB/P2P/node runtime. Это сохранит хорошую module depth.

## Минимальный upstream interface: три entry point

```rust
pub trait WalletTransport {
    async fn call<C: WalletRpcCall>(&self, call: C) -> Result<C::Result, RpcError>;
}

impl<T: WalletTransport> WalletClient<T> {
    pub async fn restore_scripts(
        &self,
        scripts: &[OutputScriptId],
    ) -> Result<WalletSnapshot, RestoreError>;

    pub async fn transaction_evidence(
        &self,
        snapshot: &WalletSnapshot,
        txids: &[TransactionHash],
    ) -> Result<Vec<VerifiedTransactionEvidence>, EvidenceError>;

    pub async fn assess_signed_transaction(
        &self,
        snapshot: &WalletSnapshot,
        transaction: &Transaction,
        target_blocks: u32,
    ) -> Result<SignedTransactionAssessment, AssessmentError>;
}
```

### 1. `restore_scripts`

Контракт:

- validates, sorts and deduplicates the **entire** wallet script set;
- retains the sorted-index ↔ caller-index map, but knows nothing about BIP44 or address ownership;
- exhausts confirmed pages and then one immutable mempool generation;
- validates chain epoch, full tip, nonce, generation, cursor progress, page/item bounds and every `script_index`;
- on a typed retryable stale error discards all partial data and may retry the whole read under a bounded caller-supplied retry budget;
- returns only a complete `WalletSnapshot`, never a partial vector.

`WalletSnapshot` should contain typed bindings, confirmed history/UTXOs, mempool activity and per-script positions. It must not contain a seed, derivation path, wallet labels or SQLite IDs. Upstream already defines the sorted-position semantics and limits, so this interface deepens the existing module instead of inventing a new authority. [Script-set contract](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/WALLET_RPC_V1.md#L96-L118)

### 2. `transaction_evidence`

Контракт:

- uses exactly the snapshot's epoch/tip/mempool binding;
- fetches all relevant txids, rejects the whole aggregate on any binding change;
- decodes canonical `hns_transaction::Transaction`, recomputes txid and validates status/payload/inclusion combinations;
- joins the raw transaction with already-known `script_index` relevance from the snapshot and returns typed `Received`/`Spent` evidence;
- preserves `PayloadPruned`/unknown data explicitly rather than fabricating zeroes or legacy hsd JSON.

Для пилота это можно реализовать поверх существующего `transaction_evidence`; каждый point read уже может быть привязан к prior mempool binding. Позже можно добавить batch RPC для efficiency, но это не нужно для правильности. [Combined transaction evidence](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/HNS_NODE_WALLET_INDEX.md#L123-L143)

Важная граница: upstream evidence говорит **что произошло в chain/mempool**. Namehold по-прежнему решает, как превратить это в `send`, `bid`, `reveal`, `transfer` и UI amount.

### 3. `assess_signed_transaction`

Контракт:

- accepts only an already signed canonical `Transaction`; no key callback and no signing method;
- locally encodes it and computes canonical txid;
- calls the existing bound `quote_transaction_fee` for those exact witness bytes;
- requires returned txid and all chain/mempool bindings to equal the local artifact/snapshot;
- returns factual `SignedTransactionAssessment`: actual fee, minimum fee, shortfall, weight, sigops, rate evidence and `meets_node_minimum`;
- never decides `approved`, never compares with the user's maximum fee, never auto-broadcasts.

Это точная граница между node policy facts и wallet policy. Upstream docs отдельно подчёркивают, что quote не является admission result, и что wallet должен повторно quote-ить final signed bytes. [Transaction-bound fee quotes](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/WALLET_RPC_V1.md#L139-L195)

Broadcast лучше оставить отдельным явным RPC action: именно Namehold должен проверить user approval, fee drift и permission для remote broadcaster перед мутацией.

## Необходимые upstream prerequisites

### Public typed wire contract

Server и client должны serde-рить одни и те же public v1 DTO и enums. Стабильные error codes (`stale_snapshot`, `payload_pruned`, `transaction_rejected` и другие) должны стать enum с `retryable`, а не строкой, как сейчас в Namehold. hsrd уже имеет стабильное error mapping, но оно private. [`wallet_backend_error`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/wallet_rpc.rs#L563-L697)

### Node identity in authenticated capabilities

`capabilities` сейчас возвращает bounds и semantic strings, но не возвращает actual `network`/`genesis_hash`. [`capabilities`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/wallet_rpc.rs#L289-L329) Для external endpoint кошелёк должен fail-closed сравнить ожидаемую network/genesis с node authority. Эти поля нужно добавить upstream, а client constructor должен выполнять handshake до любого restore/broadcast.

### Exact Authorization type and secure endpoint constructor

hsrd уже имеет строгий `RpcAuthorizationHeader`: 1..=4096 visible ASCII bytes, без leading/trailing whitespace; loader удаляет только один terminal LF/CRLF. [`RpcAuthorizationHeader`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/lib.rs#L470-L505), [file loader](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-node/src/main.rs#L362-L400)

Тот же parser/fixture должен использовать client. `AuthorizedEndpoint::new` должен возвращать `Result`, разрешать HTTP только для loopback и никогда не понижаться до unauthenticated client. При этом app-specific endpoint↔credential consent всё равно остаётся в Namehold: библиотека не может знать, кто изменил Tauri setting.

### Shared `OutputScriptId`

Namehold сейчас вручную повторяет version + length + program → BLAKE2b-256. [`Namehold script_id`](https://github.com/DimazzzZ/namehold-wallet/blob/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2/src-tauri/src/noncustodial/hsrd.rs#L1106-L1121) Алгоритм уже живёт в `hns-wallet-index::ScriptId::from_address`. [`upstream ScriptId`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-wallet-index/src/lib.rs#L149-L177)

Для долгосрочной locality тип `OutputScriptId` и его vectors лучше вынести в малый shared crate (логично рядом с canonical address encoding в `hns-rs`). До сближения двух internal primitive models достаточно одних cross-repo frozen vectors; не нужно тащить node/store crates в desktop wallet.

## Файл за файлом: что оставить в PR, что вынести upstream

| Namehold PR file | Оставить в Namehold | Вынести upstream / удалить из app layer | Почему |
|---|---|---|---|
| [`src-tauri/src/noncustodial/hsrd.rs`](https://github.com/DimazzzZ/namehold-wallet/blob/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2/src-tauri/src/noncustodial/hsrd.rs) | Тонкий adapter из Namehold settings/network/address labels в upstream typed results; UI-specific blockchain/name/resource JSON. | `RpcRequest/Response/Error`, все `Wire*`, auth validation, transport guard, typed error mapping, cursor loops, binding checks, `restore_address`, `transaction_evidence`, canonical tx/proof validation, fee-assessment validation, generic tracked-contract paging. | Сейчас файл смешивает wire, protocol, orchestration и presentation. Upstream-модуль даст depth; Namehold-файл станет маленьким application adapter. |
| [`src-tauri/src/commands/sync.rs`](https://github.com/DimazzzZ/namehold-wallet/blob/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2/src-tauri/src/commands/sync.rs#L517-L575) | Profile/address lookup, scheduler, SQLite transaction, upsert/mark-used/cursor, application telemetry. | Весь multi-script read должен быть одним `restore_scripts`; Namehold не должен loop-ить по адресам. | Wallet-wide atomicity — invariant RPC, а не SQLite/UI. Сейчас per-address errors silently skipped, после чего app globally marks missing coins spent. |
| [`src-tauri/src/commands/history.rs`](https://github.com/DimazzzZ/namehold-wallet/blob/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2/src-tauri/src/commands/history.rs#L96-L205) | Namehold action taxonomy, direction/counterparty UX, joins с local address/coin/name DB. | Typed decoded transaction + `script_index` relevance + status/inclusion/payload должны приходить из `transaction_evidence`; удалить hsd-shaped `serde_json::Value` seam. | Факты и relevance — upstream; labels `bid/reveal/send` — product policy. Текущий consumer ждёт `inputs[].coin.address`, которого producer не создаёт. [`transaction_json`](https://github.com/DimazzzZ/namehold-wallet/blob/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2/src-tauri/src/noncustodial/hsrd.rs#L1210-L1245) |
| [`src-tauri/src/commands/tx.rs`](https://github.com/DimazzzZ/namehold-wallet/blob/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2/src-tauri/src/commands/tx.rs#L707-L755) | Draft/reservation lifecycle, coin selection, local signing, user-confirmed fee, maximum-fee/drift decision, permission для remote broadcast, persistence после relay. | Canonical final artifact + local txid + bound quote validation в `assess_signed_transaction`; raw `send_raw_transaction` может быть typed transport method, но его вызов остаётся в Namehold. | Node владеет fee/admission facts; wallet владеет consent и expenditure policy. |
| [`src-tauri/src/noncustodial/marketplace.rs`](https://github.com/DimazzzZ/namehold-wallet/blob/9d168a6a3af2617b24ad29aa6fd343c59b6ff4c2/src-tauri/src/noncustodial/marketplace.rs) | Только реальные application workflows: persistence, user approval, matching/session UI. Thin error mapping допустим. | Canonical listing/swap/HTLC types уже в `hns-rs`; node tracker descriptor/classification должен там же иметь protocol authority. До upstream registration seam dead `tracked_contract_*` methods и claims лучше отложить. | Текущий файл — почти только re-export и два decoder; это не integrated feature. |
| `providers/signer.rs`, HD/key/draft modules | Всё: signer capability, unlock/session, derivation, transaction construction/signing. | Ничего в hsrd. | Upstream node явно обещает отсутствие key/seed/signing API. [Resource boundary](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/WALLET_RPC_V1.md#L252-L281) |
| `commands/node.rs`, Docker, release workflows | Managed/external lifecycle, paths, process ownership, UI status, packaging, platform assets. | hsrd должен upstream публиковать readiness/network/genesis/capabilities; Namehold не должен угадывать их по process flags. | Operational ownership — app-specific; node identity/readiness — node fact. |
| `security.rs`, settings commands, SQL migrations | Credential consent, renderer permissions, endpoint ownership, migrations и user-intent preservation. | Только reusable exact auth/transport types. | Библиотека не может защитить Tauri command или решить, какой endpoint получил user consent. |
| Namehold tests | DB migration fixtures, wallet policy, UI, signer, real end-to-end hsrd regtest qualification. | Wire round-trips, cursor/reorg/restart/auth adversarial cases, proof/txid/quote validation и shared frozen vectors. | Implementation contract должен тестироваться upstream; consumer workflow и independent black-box qualification — в Namehold. |

## Swap/marketplace: отдельный upstream tranche

Эту часть не стоит доводить в Namehold в обход upstream.

`hns-rs` уже имеет canonical `SwapProof`, `FixedPriceListing`, `HnsHtlc` и chain-verification helpers. [`SwapProof`](https://github.com/handshake-rs/hns-rs/blob/15f715576a2111fae2a8c65fccc7860ede64bd98/crates/hns-swap/src/lib.rs#L49-L223), [`HnsHtlc`](https://github.com/handshake-rs/hns-rs/blob/15f715576a2111fae2a8c65fccc7860ede64bd98/crates/hns-swap/src/htlc.rs#L28-L220) Но `hns-wallet-index/src/swap.rs` пока дублирует descriptor structs, script constants и spend classification локально. [`hns-wallet-index swap tracker`](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/crates/hns-wallet-index/src/swap.rs#L93-L327) Документация hsrd сама называет эту duplication не protocol authority и запрещает registration/preimage в wallet RPC до published canonical revision и threat review. [Tracked contracts](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/WALLET_RPC_V1.md#L229-L250), [security-model boundary](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/security-model.md#L337-L361)

Рекомендованная последовательность:

1. В `hns-rs` зафиксировать/tag-нуть canonical public descriptor revision и deterministic identity/conversion из `SwapProof`/`HnsHtlc`.
2. В `hns-node-rs` заменить local protocol duplication на pinned canonical semantics или на малый shared descriptor crate.
3. Спроектировать authenticated, quota-bound registration/unregistration lifecycle; сейчас append-only registry имеет lifetime capacity blocker. [Registry lifecycle](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/HNS_NODE_WALLET_INDEX.md#L70-L91)
4. Только после этого добавить typed registration/evidence в wallet RPC и реальный Namehold session workflow.

До этого момента в PR можно оставить pin `hns-rs` и local decode helpers как groundwork, но не заявлять tracked swap feature и не держать dead RPC methods в production path.

## Что не стоит выносить upstream

- BIP39/BIP44 derivation, encrypted seed, unlock timeout и signing session.
- Coin selection, local reservations, draft lifecycle и rebuild strategy.
- Решение, согласен ли пользователь на actual fee, и лимит maximum/absurd fee.
- `allow_remote_broadcast`, endpoint approval и binding secret к approved host.
- SQLite schema, profile switching, address gap policy и `mark_missing_as_spent` semantics.
- Namehold action classification, labels, counters, notifications и UI JSON.
- Managed-sidecar paths, process ownership, data-dir backup/resync, Docker/Tauri packaging.
- App migration и release-updater policy.

Это не protocol facts, а product policy. Если вынести их upstream, модуль перестанет быть reusable и начнёт скрыто задавать UX/security policy за все wallets.

## Практический план пилота

### Tranche A: безопасный pilot в Namehold

Оставить PR draft/experimental и использовать его как executable specification, но до любого пилота исправить app-owned P0/P1:

- renderer не может менять credential target и arbitrary resync path;
- transport construction fail-closed;
- никаких partial address restores перед `mark_missing_as_spent`;
- network/genesis проверяются перед sync;
- quote/broadcast txid сравниваются с local final transaction;
- remote broadcast gate enforced в backend.

### Tranche B: малый upstream client

1. Public v1 wire types + typed errors + auth/endpoint type.
2. `restore_scripts`, `transaction_evidence`, `assess_signed_transaction`.
3. Authenticated `network` + `genesis_hash` + readiness in capabilities.
4. Server/client round-trip fixtures и adversarial state-machine tests.
5. Тег hsrd/client/hns-rs revisions; Namehold пинит один qualified set.

### Tranche C: independent cross-project qualification

Namehold запускает real hsrd regtest binary, а не mock JSON, и проверяет:

- exact Authorization, wrong credential, remote plaintext rejection;
- wrong network/genesis;
- multi-page complete restore;
- reorg между confirmed pages;
- hsrd restart между confirmed/mempool/evidence calls;
- mempool generation change и parent/child ordering;
- pruned/retained evidence;
- final signed quote, txid binding, contextual rejection, broadcast and relay accounting;
- application crash/restart между snapshot и SQLite commit.

Upstream сам требует именно restart/reorg/adversarial transport qualification и не разрешает называть source inspection production readiness. [`WALLET_RPC_V1.md` conclusion](https://github.com/handshake-rs/hns-node-rs/blob/v0.3.4/docs/WALLET_RPC_V1.md#L268-L281)

### Tranche D: swap/marketplace после wallet core

Выполнить canonical descriptor convergence, registry lifecycle/threat review и typed registration RPC upstream; затем добавить Namehold session/UI. Не смешивать это с первым production wallet restore/broadcast gate.

## Рекомендация по merge strategy

Не нужно отказываться от PR или от синергии. Лучше изменить роль PR:

- **сейчас:** integration laboratory / executable specification на experimental branch или за feature flag;
- **параллельно:** вынести generic adapter и protocol fixes в upstream PR-ы;
- **перед main/release merge:** заменить локальную generic реализацию на pinned upstream adapter и приложить cross-project evidence.

Так Namehold не станет случайным владельцем hsrd wallet protocol, а hsrd получит реальный consumer-driven interface. Самый ценный результат пилота — не просто «Namehold работает с hsrd», а появление глубокого повторно используемого wallet adapter с настоящей cross-repo qualification.
