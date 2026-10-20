//! Every activation of a wallet profile goes through
//! `secure_wallet::activate_profile`, which also drops a signer session that
//! belongs to another profile. A direct `set_active_profile(` call elsewhere in
//! `secure_wallet.rs` would bring back the stale "Unlocked" signer.

const SOURCE: &str = include_str!("../commands/secure_wallet.rs");

/// Code lines of the module (comments and the unit tests at the end are not it).
fn code_lines(src: &str) -> Vec<&str> {
    src.lines()
        .take_while(|l| !l.trim_start().starts_with("mod tests"))
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect()
}

/// The `set_active_profile(` calls outside `activate_profile` and outside the
/// delete path's clear to the empty id (`set_active_profile(&conn, "")`).
fn stray_calls(src: &str) -> Vec<String> {
    let mut in_helper = false;
    let mut out = Vec::new();
    for l in code_lines(src) {
        if l.contains("fn activate_profile(") {
            in_helper = true;
        } else if in_helper && l.starts_with('}') {
            in_helper = false;
        }
        if l.contains("set_active_profile(") && !in_helper && !l.contains("\"\")") {
            out.push(l.trim().to_string());
        }
    }
    out
}

#[test]
fn only_activate_profile_sets_the_active_profile() {
    assert_eq!(stray_calls(SOURCE), Vec::<String>::new());
}

#[test]
fn the_scan_sees_a_direct_call() {
    let src = "fn f() {\n    db::queries::set_active_profile(&conn, &id)?;\n}\n\
               pub(crate) fn activate_profile(s: &S, id: &str) {\n    db::queries::set_active_profile(&conn, id)?;\n}\n\
               fn g() { db::queries::set_active_profile(&conn, \"\")?; }\n";
    assert_eq!(stray_calls(src).len(), 1);
}

#[test]
fn every_activation_site_calls_the_helper() {
    let calls = code_lines(SOURCE)
        .iter()
        .filter(|l| l.contains("activate_profile(") && !l.contains("fn activate_profile"))
        .count();
    // create, import mnemonic, import xpub, Ledger, set_active_wallet_profile.
    assert_eq!(calls, 5);
}
