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
/// context, and the secure window's confirm helper (`secure_confirm`, the
/// one place that names `secure_prompt` for them). Everything else under
/// `commands` is a sibling, and the legacy shared layers (`sync`, `read`,
/// `namebase`, `secure_prompt`) are siblings too — refusing the whole class
/// keeps a new sibling from slipping past a list of known ones.
const ALLOWED: [&str; 2] = ["draft_ctx", "secure_confirm"];

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
