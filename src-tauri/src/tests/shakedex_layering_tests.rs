//! docs/CODING_STANDARDS.md "Rust layering": new code does not import helpers
//! from a sibling command module, and does not add to the legacy shared
//! layers (`commands::sync::open_conn` and friends). The Shakedex code is new,
//! so it is held to that from the source text itself.

/// `(file, source)` for every Shakedex module outside `noncustodial`, and the
/// shared draft context they build on.
const SOURCES: [(&str, &str); 3] = [
    (
        "commands/shakedex.rs",
        include_str!("../commands/shakedex.rs"),
    ),
    ("shakedex_jobs.rs", include_str!("../shakedex_jobs.rs")),
    (
        "commands/draft_ctx.rs",
        include_str!("../commands/draft_ctx.rs"),
    ),
];

/// The only command modules these sources may name: the shared draft
/// context, the secure window's confirm helper (`secure_confirm`, the
/// one place that names `secure_prompt` for them), and the writer of a file
/// at a path the renderer chose (`user_file`). Everything else under
/// `commands` is a sibling, and the legacy shared layers (`sync`, `read`,
/// `namebase`, `secure_prompt`) are siblings too — refusing the whole class
/// keeps a new sibling from slipping past a list of known ones.
const ALLOWED: [&str; 3] = ["draft_ctx", "secure_confirm", "user_file"];

/// Code only: a mention in a comment is not an import, and the unit tests at
/// the end of a file (`mod tests`, where `super` is the file itself) are not
/// the module.
fn code_lines(src: &str) -> impl Iterator<Item = &str> {
    src.lines()
        .map(str::trim_start)
        .take_while(|l| !l.starts_with("mod tests"))
        .filter(|l| !l.starts_with("//"))
}

/// Every command module `line` names through `commands::` or `super::`
/// (in these files `super` is `commands`) that is not in [`ALLOWED`].
fn siblings_named(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    for prefix in ["commands::", "super::"] {
        for (at, _) in line.match_indices(prefix) {
            let module: String = line[at + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !ALLOWED.contains(&module.as_str()) {
                found.push(format!("{prefix}{module}"));
            }
        }
    }
    found
}

#[test]
fn shakedex_modules_import_no_sibling_command_module() {
    let mut scanned = 0;
    for (file, src) in SOURCES {
        for line in code_lines(src) {
            scanned += 1;
            let found = siblings_named(line);
            assert!(found.is_empty(), "{file} reaches into {found:?}: {line}");
        }
    }
    assert!(scanned > 500, "only {scanned} lines scanned");
}

/// The check itself: every legacy layer and sibling is caught, in a `use` or
/// a path, and the shared draft context is not.
#[test]
fn sibling_check_catches_every_sibling_form() {
    for line in [
        "use super::sync::open_conn;",
        "use crate::commands::read::resolve_profile;",
        "let c = super::namebase::client();",
        "use super::secure_prompt;",
        "use super::{draft_ctx, tx};",
        "crate::commands::names::fetch_name_state(c, n)",
        "use super::*;",
    ] {
        assert!(!siblings_named(line).is_empty(), "missed: {line}");
    }
    for line in [
        "use crate::commands::draft_ctx::{self, Ctx};",
        "use crate::commands::secure_confirm;",
        "crate::commands::secure_confirm::confirm_rows(app, t, m, d)",
        "draft_ctx::fee_rate(&p.ctx, fee_rate)",
        "use crate::noncustodial::shakedex::purchase;",
    ] {
        assert!(siblings_named(line).is_empty(), "false alarm: {line}");
    }
}

#[test]
fn transaction_commands_do_not_reach_into_the_shakedex_commands() {
    let tx = include_str!("../commands/tx.rs");
    for line in code_lines(tx) {
        assert!(
            !line.contains("commands::shakedex"),
            "commands/tx.rs reaches into commands::shakedex: {line}"
        );
    }
}

/// SECURITY.md, "The daemon never signs or broadcasts": a lexical scan of
/// the code lines (comments and a trailing `mod tests` left out) of the
/// market jobs' two files, `shakedex_jobs.rs` and the market client they
/// write through, `market/learnhns.rs`: neither names a signing call or key
/// material. The whole class is refused, case-insensitively: anything that
/// signs (`sign_…`, a signer), any private key, seed or mnemonic, a master
/// key, the unlocked session and the vault. A send is refused too, except
/// the purchase's one rebroadcast in `shakedex_jobs.rs`, which
/// `may_broadcast` closes for the daemon (`Rebroadcast::Never`), and the
/// resend of its Speed up child that only that rebroadcast's success reaches
/// (`rebroadcast_child`, spec 2026-10-21 R15): that exact line, exactly
/// twice.
#[test]
fn shakedex_jobs_hold_no_signing_call() {
    let sources = [
        ("shakedex_jobs.rs", include_str!("../shakedex_jobs.rs")),
        ("market/learnhns.rs", include_str!("../market/learnhns.rs")),
    ];
    let forbidden = [
        "sign_",
        "signer",
        "privkey",
        "secretkey",
        "seed",
        "mnemonic",
        ".master(",
        "session",
        "vault",
        "sendrawtransaction",
        "send_raw_transaction",
    ];
    const REBROADCAST: &str = "match self.client.send_raw_transaction(signed).await {";
    let mut rebroadcasts = 0;
    for (file, src) in sources {
        let mut scanned = 0;
        for line in code_lines(src) {
            scanned += 1;
            if file == "shakedex_jobs.rs" && line.trim_end() == REBROADCAST {
                rebroadcasts += 1;
                continue;
            }
            let lower = line.to_lowercase();
            for f in forbidden {
                assert!(!lower.contains(f), "{file} names `{f}`: {line}");
            }
        }
        assert!(scanned > 300, "{file}: only {scanned} lines scanned");
    }
    assert_eq!(
        rebroadcasts, 2,
        "the guarded rebroadcast and its child's resend, exactly once each"
    );
    // The child's resend is reached from the purchase's rebroadcast alone.
    let child_resends = code_lines(include_str!("../shakedex_jobs.rs"))
        .filter(|l| l.contains("rebroadcast_child("))
        .count();
    assert_eq!(child_resends, 2, "its definition and its one call");
}

/// Where the market jobs' two sends are (spec 2026-10-21 R15): one in
/// `refresh_missing`, after its `may_broadcast()` gate, and one in
/// `rebroadcast_child`, whose only call is in `refresh_missing` after that
/// same gate. Read lexically, by the function each code line falls in.
#[test]
fn shakedex_jobs_send_only_behind_may_broadcast() {
    const SEND: &str = "match self.client.send_raw_transaction(signed).await {";
    let mut current = String::new();
    let mut gate_seen_in = None::<String>;
    let mut sends = Vec::new();
    let mut calls = Vec::new();
    for line in code_lines(include_str!("../shakedex_jobs.rs")) {
        if let Some(rest) = line
            .strip_prefix("async fn ")
            .or_else(|| line.strip_prefix("fn "))
            .or_else(|| line.strip_prefix("pub(crate) async fn "))
            .or_else(|| line.strip_prefix("pub async fn "))
            .or_else(|| line.strip_prefix("pub fn "))
        {
            current = rest.split(['(', '<']).next().unwrap_or("").to_string();
        }
        if line.contains("self.may_broadcast()") {
            gate_seen_in = Some(current.clone());
        }
        let gated = gate_seen_in.as_deref() == Some(current.as_str());
        if line.trim_end() == SEND {
            sends.push((current.clone(), gated));
        }
        if line.contains("self.rebroadcast_child(") {
            calls.push((current.clone(), gated));
        }
    }
    assert_eq!(
        sends,
        vec![
            ("refresh_missing".to_string(), true),
            ("rebroadcast_child".to_string(), false)
        ],
        "the purchase's send after the gate, the child's in its own function"
    );
    assert_eq!(
        calls,
        vec![("refresh_missing".to_string(), true)],
        "the child's resend is called once, after the gate"
    );
}
