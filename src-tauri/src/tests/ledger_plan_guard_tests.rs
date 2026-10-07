use crate::commands::tx::refuse_unsupported_ledger_plan;
use crate::error::AppError;

const PLAIN: &str =
    r#"{"version":0,"locktime":0,"account":0,"network":"main","inputs":[],"outputs":[]}"#;
const FOREIGN: &str = r#"{"version":0,"locktime":2147483700,"account":0,"network":"main",
  "inputs":[{"txid":"0909090909090909090909090909090909090909090909090909090909090909","vout":0,
  "value":0,"branch":0,"child_index":0,"sighash_type":132,"sequence":4294967294,
  "foreign_witness_hex":["aa"]}],"outputs":[]}"#;
/// A `send_hns` draft stores its build parameters, not a plan; the signer
/// builds the plan from the wallet's own coins.
const SEND_PARAMS: &str = r#"{"network":"main","account":0,"to_address":"hs1q","amount_doos":1}"#;

#[test]
fn ledger_refuses_plan_with_foreign_input() {
    let err = refuse_unsupported_ledger_plan("ledger_hardware", "register", FOREIGN).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(m) if m.contains("recovery-phrase wallet")));
}

#[test]
fn ledger_accepts_ordinary_plan_and_hot_accepts_anything() {
    refuse_unsupported_ledger_plan("ledger_hardware", "register", PLAIN).unwrap();
    refuse_unsupported_ledger_plan("mnemonic_hot", "shakedex_purchase", FOREIGN).unwrap();
}

#[test]
fn ledger_refuses_a_covenant_plan_it_cannot_read() {
    let err =
        refuse_unsupported_ledger_plan("ledger_hardware", "register", "not a plan").unwrap_err();
    assert!(matches!(err, AppError::Other(m) if m.contains("corrupted draft")));
}

#[test]
fn ledger_send_hns_params_are_not_a_plan_and_pass() {
    refuse_unsupported_ledger_plan("ledger_hardware", "send_hns", SEND_PARAMS).unwrap();
}
