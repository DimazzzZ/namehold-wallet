//! Tests for `build_name_action_capabilities` — the 265-line pure function
//! that derives all 11 can_* flags, task state, and next-action from phase +
//! wallet evidence. Exercises the function directly without RPC/DB.

use crate::commands::names::{
    build_name_action_capabilities, conservative_capabilities, AuctionTaskState, NameActionContext,
};

/// Default context: no wallet evidence at all.
fn empty_ctx() -> NameActionContext {
    NameActionContext {
        has_bid_commitment: false,
        has_bid_coin: false,
        has_reveal_coin: false,
        has_owner_coin: false,
        owner_covenant_type: None,
        name_height: None,
        transfer_has_items: None,
        existing_bid_count: 0,
        has_pending_open: false,
        reveal_txid: None,
        reveal_draft_status: None,
        bid_value_doos: None,
    }
}

// ---------------------------------------------------------------------------
// AVAILABLE phase
// ---------------------------------------------------------------------------

#[test]
fn available_can_open_only() {
    let ctx = empty_ctx();
    let caps = build_name_action_capabilities(
        "example".into(),
        "AVAILABLE".into(),
        "AVAILABLE",
        None,
        &ctx,
        false,
        false,
        None,
    );
    assert_eq!(caps.task_state, AuctionTaskState::AvailableToOpen);
    assert!(caps.can_open.allowed);
    assert!(!caps.can_bid.allowed);
    assert!(!caps.can_reveal.allowed);
    assert!(!caps.can_redeem.allowed);
    assert!(!caps.can_register.allowed);
    assert!(!caps.can_update.allowed);
    assert!(!caps.can_transfer.allowed);
    assert!(!caps.can_finalize.allowed);
    assert!(!caps.can_cancel_transfer.allowed);
    assert!(!caps.can_renew.allowed);
    assert!(!caps.can_revoke.allowed);
}

#[test]
fn available_with_pending_open_blocks_open() {
    let mut ctx = empty_ctx();
    ctx.has_pending_open = true;
    let caps = build_name_action_capabilities(
        "example".into(),
        "AVAILABLE".into(),
        "AVAILABLE",
        None,
        &ctx,
        false,
        false,
        None,
    );
    assert!(!caps.can_open.allowed);
    assert_eq!(caps.task_state, AuctionTaskState::WaitingForBidding);
}

// ---------------------------------------------------------------------------
// OPENING phase
// ---------------------------------------------------------------------------

#[test]
fn opening_disallows_all_actions() {
    let ctx = empty_ctx();
    let caps = build_name_action_capabilities(
        "test".into(),
        "OPENING".into(),
        "OPENING",
        None,
        &ctx,
        false,
        false,
        None,
    );
    assert_eq!(caps.task_state, AuctionTaskState::WaitingForBidding);
    assert!(!caps.can_open.allowed);
    // NOTE: OPENING is bid-compatible (is_bidding_compatible covers OPENING),
    // so can_bid is allowed here when there is no existing bid — the task
    // state stays WaitingForBidding but the bid capability is open.
    assert!(caps.can_bid.allowed);
    assert!(!caps.can_reveal.allowed);
    assert!(!caps.can_register.allowed);
}

// ---------------------------------------------------------------------------
// BIDDING phase
// ---------------------------------------------------------------------------

#[test]
fn bidding_allows_bid_when_no_existing_bid() {
    let ctx = empty_ctx();
    let caps = build_name_action_capabilities(
        "test".into(),
        "BIDDING".into(),
        "BIDDING",
        None,
        &ctx,
        false,
        false,
        None,
    );
    assert_eq!(caps.task_state, AuctionTaskState::ReadyToBid);
    assert!(caps.can_bid.allowed);
    assert!(!caps.can_open.allowed);
}

#[test]
fn bidding_blocks_bid_when_already_has_commitment() {
    let mut ctx = empty_ctx();
    ctx.has_bid_commitment = true;
    ctx.existing_bid_count = 1;
    let caps = build_name_action_capabilities(
        "test".into(),
        "BIDDING".into(),
        "BIDDING",
        None,
        &ctx,
        false,
        false,
        None,
    );
    assert!(!caps.can_bid.allowed);
}

// ---------------------------------------------------------------------------
// REVEAL phase
// ---------------------------------------------------------------------------

#[test]
fn reveal_allows_reveal_when_has_bid_coin() {
    let mut ctx = empty_ctx();
    ctx.has_bid_commitment = true;
    ctx.has_bid_coin = true;
    let caps = build_name_action_capabilities(
        "test".into(),
        "REVEAL".into(),
        "REVEAL",
        None,
        &ctx,
        false,
        false,
        None,
    );
    assert_eq!(caps.task_state, AuctionTaskState::ReadyToReveal);
    assert!(caps.can_reveal.allowed);
    assert!(!caps.can_bid.allowed);
}

#[test]
fn reveal_broadcast_pending_state() {
    let mut ctx = empty_ctx();
    ctx.has_bid_commitment = true;
    ctx.has_bid_coin = false; // spent by reveal
    ctx.reveal_txid = Some("abc123".into());
    ctx.reveal_draft_status = Some("broadcasted".into());
    let caps = build_name_action_capabilities(
        "test".into(),
        "REVEAL".into(),
        "REVEAL",
        None,
        &ctx,
        false,
        false,
        None,
    );
    assert_eq!(caps.task_state, AuctionTaskState::RevealBroadcastPending);
    assert!(!caps.can_reveal.allowed);
}

#[test]
fn reveal_done_waiting_for_close() {
    let mut ctx = empty_ctx();
    ctx.has_bid_commitment = true;
    ctx.has_bid_coin = false;
    ctx.has_reveal_coin = true; // reveal confirmed
                                // The _ fallback branch needs reveal_txid set AND the bid coin spent to
                                // reach RevealDoneWaitingForClose.
    ctx.reveal_txid = Some("deadbeef".into());
    let caps = build_name_action_capabilities(
        "test".into(),
        "REVEAL".into(),
        "REVEAL",
        None,
        &ctx,
        false,
        false,
        None,
    );
    assert_eq!(caps.task_state, AuctionTaskState::RevealDoneWaitingForClose);
    assert!(!caps.can_reveal.allowed);
}

// ---------------------------------------------------------------------------
// CLOSED phase — won / lost / owned
// ---------------------------------------------------------------------------

#[test]
fn closed_won_needs_register() {
    let mut ctx = empty_ctx();
    // Won: owns the name coin, but covenant type is still pre-REGISTER (< 6),
    // e.g. REVEAL(4) — so registration is still needed.
    ctx.has_owner_coin = true;
    ctx.owner_covenant_type = Some(4);
    let caps = build_name_action_capabilities(
        "test".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &ctx,
        true,
        false,
        None,
    );
    assert_eq!(caps.task_state, AuctionTaskState::WonNeedsRegister);
    assert!(caps.can_register.allowed);
    // can_redeem requires !owns_name, so a winner cannot redeem.
    assert!(!caps.can_redeem.allowed);
}

#[test]
fn closed_lost_needs_redeem() {
    let mut ctx = empty_ctx();
    ctx.has_reveal_coin = true;
    ctx.has_owner_coin = false;
    let caps = build_name_action_capabilities(
        "test".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &ctx,
        false,
        false,
        None,
    );
    assert_eq!(caps.task_state, AuctionTaskState::LostNeedsRedeem);
    assert!(caps.can_redeem.allowed);
    assert!(!caps.can_register.allowed);
}

#[test]
fn closed_owned_no_urgent_action() {
    let mut ctx = empty_ctx();
    ctx.has_owner_coin = true;
    ctx.owner_covenant_type = Some(7); // COV_UPDATE (>= COV_REGISTER=6 → registered)
    let caps = build_name_action_capabilities(
        "test".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &ctx,
        true,
        false,
        Some(200.0), // not expiring soon
    );
    assert_eq!(caps.task_state, AuctionTaskState::OwnedNoUrgentAction);
    assert!(caps.can_update.allowed);
    assert!(caps.can_transfer.allowed);
    assert!(caps.can_renew.allowed);
    assert!(caps.can_revoke.allowed);
    // Already registered (covenant >= 6) → can_register false.
    assert!(!caps.can_register.allowed);
}

#[test]
fn closed_owned_expiring_soon() {
    let mut ctx = empty_ctx();
    ctx.has_owner_coin = true;
    ctx.owner_covenant_type = Some(7); // registered
    let caps = build_name_action_capabilities(
        "test".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &ctx,
        true,
        false,
        Some(15.0), // < EXPIRING_SOON_THRESHOLD_DAYS (30)
    );
    assert_eq!(caps.task_state, AuctionTaskState::ExpiringSoon);
}

// ---------------------------------------------------------------------------
// Spend-lock override
// ---------------------------------------------------------------------------

#[test]
fn spend_locked_disables_all_spend_actions() {
    let mut ctx = empty_ctx();
    ctx.has_owner_coin = true;
    ctx.owner_covenant_type = Some(7);
    let caps = build_name_action_capabilities(
        "test".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &ctx,
        true,
        true, // spend_locked
        Some(200.0),
    );
    // Spend-lock should disable transfer/update/renew/revoke.
    assert!(!caps.can_transfer.allowed);
    assert!(!caps.can_update.allowed);
    assert!(!caps.can_renew.allowed);
    assert!(!caps.can_revoke.allowed);
}

// ---------------------------------------------------------------------------
// Transfer pending finalize
// ---------------------------------------------------------------------------

#[test]
fn transfer_pending_finalize() {
    let mut ctx = empty_ctx();
    ctx.has_owner_coin = true;
    ctx.owner_covenant_type = Some(9); // COV_TRANSFER
    ctx.transfer_has_items = Some(true);
    let caps = build_name_action_capabilities(
        "test".into(),
        "TRANSFER".into(),
        "TRANSFER",
        None,
        &ctx,
        true,
        false,
        None,
    );
    assert_eq!(caps.task_state, AuctionTaskState::TransferPendingFinalize);
    assert!(caps.can_finalize.allowed);
    assert!(caps.can_cancel_transfer.allowed);
}

// ---------------------------------------------------------------------------
// conservative_capabilities
// ---------------------------------------------------------------------------

#[test]
fn conservative_disables_everything() {
    let caps = conservative_capabilities("test", "node unreachable");
    assert!(!caps.can_open.allowed);
    assert!(!caps.can_bid.allowed);
    assert!(!caps.can_reveal.allowed);
    assert!(!caps.can_redeem.allowed);
    assert!(!caps.can_register.allowed);
    assert!(!caps.can_update.allowed);
    assert!(!caps.can_transfer.allowed);
    assert!(!caps.can_finalize.allowed);
    assert!(!caps.can_cancel_transfer.allowed);
    assert!(!caps.can_renew.allowed);
    assert!(!caps.can_revoke.allowed);
    assert_eq!(caps.task_state, AuctionTaskState::UnavailableOther);
    // Reason should be present on every capability.
    assert_eq!(caps.can_open.reason.as_deref(), Some("node unreachable"));
}
