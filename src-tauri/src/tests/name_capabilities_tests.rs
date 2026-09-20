//! Pure-logic tests for name action capability derivation.
//!
//! These tests cover the capability-building pipeline:
//!
//! 1. `NameActionContext` (DB-gathered evidence)
//! 2. `AuctionTaskState` (phase + evidence)
//! 3. `NameActionCapabilities` (full capability struct with reasons).
//!
//! All tests are pure — no DB, no RPC, no async. They construct fixtures and
//! assert the capability rules match the product spec.

use crate::commands::names::{
    build_name_action_capabilities, conservative_capabilities, derive_auction_task_state,
    next_action_for_task, AuctionTaskState, NameActionContext,
};
use crate::noncustodial::network::Network;
use crate::noncustodial::sync::{COV_REGISTER, COV_TRANSFER};

/// Helper to construct a minimal `NameActionContext` with all fields set.
#[allow(clippy::too_many_arguments)]
fn ctx(
    has_bid_commitment: bool,
    has_bid_coin: bool,
    has_reveal_coin: bool,
    has_owner_coin: bool,
    owner_covenant_type: Option<i64>,
    name_height: Option<i64>,
    transfer_has_items: Option<bool>,
    existing_bid_count: i64,
    has_pending_open: bool,
    reveal_txid: Option<String>,
    reveal_draft_status: Option<String>,
    bid_value_doos: Option<i64>,
) -> NameActionContext {
    NameActionContext {
        has_bid_commitment,
        has_bid_coin,
        has_reveal_coin,
        has_owner_coin,
        owner_covenant_type,
        name_height,
        transfer_has_items,
        existing_bid_count,
        has_pending_open,
        pending_broadcast_action: None,
        stranded_bid_count: 0,
        stranded_lockup_doos: 0,
        redeemable_reveal_count: 0,
        redeemable_value_doos: 0,
        reveal_txid,
        reveal_draft_status,
        bid_value_doos,
        lockup_value_doos: None,
    }
}

// ============================================================================
// derive_auction_task_state tests
// ============================================================================

#[test]
fn task_state_available_no_pending_open() {
    let state = derive_auction_task_state(
        "AVAILABLE",
        false,
        false,
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::AvailableToOpen);
}

#[test]
fn task_state_available_with_pending_open() {
    let state = derive_auction_task_state(
        "AVAILABLE",
        false,
        false,
        false,
        false,
        false,
        None,
        None,
        true, // has_pending_open
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::WaitingForBidding);
}

#[test]
fn task_state_empty_phase_treated_as_available() {
    let state = derive_auction_task_state(
        "",
        false,
        false,
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::AvailableToOpen);
}

#[test]
fn task_state_opening_phase() {
    let state = derive_auction_task_state(
        "OPENING",
        false,
        false,
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::WaitingForBidding);
}

#[test]
fn task_state_bidding_with_commitment() {
    let state = derive_auction_task_state(
        "BIDDING",
        false,
        true, // has_bid_commitment
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    // Multi-bid: an existing commitment during BIDDING keeps the wallet in
    // ReadyToBid (another independent bid is allowed).
    assert_eq!(state, AuctionTaskState::ReadyToBid);
}

#[test]
fn task_state_bidding_without_commitment() {
    let state = derive_auction_task_state(
        "BIDDING",
        false,
        false,
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::ReadyToBid);
}

#[test]
fn task_state_reveal_no_commitment_returns_unavailable() {
    let state = derive_auction_task_state(
        "REVEAL",
        false,
        false, // no commitment
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::UnavailableOther);
}

#[test]
fn task_state_reveal_with_broadcasted_draft() {
    let state = derive_auction_task_state(
        "REVEAL",
        false,
        true,
        true,
        false,
        false,
        None,
        None,
        false,
        None,
        Some("broadcasted"),
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::RevealBroadcastPending);
}

#[test]
fn task_state_reveal_with_broadcast_pending_draft() {
    let state = derive_auction_task_state(
        "REVEAL",
        false,
        true,
        true,
        false,
        false,
        None,
        None,
        false,
        None,
        Some("broadcast_pending"),
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::RevealBroadcastPending);
}

#[test]
fn task_state_reveal_with_confirmed_draft() {
    let state = derive_auction_task_state(
        "REVEAL",
        false,
        true,
        true,
        false,
        false,
        None,
        None,
        false,
        None,
        Some("confirmed"),
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::RevealDoneWaitingForClose);
}

#[test]
fn task_state_reveal_with_dropped_draft_and_unspent_bid_coin() {
    // Dropped draft but bid coin still unspent → ReadyToReveal (can retry).
    let state = derive_auction_task_state(
        "REVEAL",
        false,
        true,
        true, // has_bid_coin
        false,
        false,
        None,
        None,
        false,
        None,
        Some("dropped"),
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::ReadyToReveal);
}

#[test]
fn task_state_reveal_with_txid_and_spent_bid_coin() {
    // reveal_txid set but bid coin spent (cross-device reveal) → done.
    let state = derive_auction_task_state(
        "REVEAL",
        false,
        true,
        false, // bid coin spent
        false,
        false,
        None,
        None,
        false,
        Some("abc123"),
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::RevealDoneWaitingForClose);
}

#[test]
fn task_state_closed_owns_name_unregistered() {
    let state = derive_auction_task_state(
        "CLOSED",
        true, // owns_name
        false,
        false,
        false,
        true,    // has_owner_coin
        Some(2), // COV_OPEN < COV_REGISTER
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::WonNeedsRegister);
}

#[test]
fn task_state_closed_owns_name_already_registered() {
    let state = derive_auction_task_state(
        "CLOSED",
        true,
        false,
        false,
        false,
        true,
        Some(6), // COV_REGISTER
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::OwnedNoUrgentAction);
}

#[test]
fn task_state_closed_owns_name_registered_expiring_soon() {
    let state = derive_auction_task_state(
        "CLOSED",
        true,
        false,
        false,
        false,
        true,
        Some(6),
        Some(15.0), // days_until_expire = 15 (below 30-day threshold)
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::ExpiringSoon);
}

#[test]
fn task_state_closed_owns_name_no_coin_synced() {
    // Owned per explorer but coin not synced locally.
    let state = derive_auction_task_state(
        "CLOSED",
        true,
        false,
        false,
        false,
        false, // no owner coin
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::OwnedNoUrgentAction);
}

#[test]
fn task_state_closed_lost_has_reveal_coin() {
    let state = derive_auction_task_state(
        "CLOSED",
        false, // doesn't own
        false,
        false,
        true, // has_reveal_coin (losing bid)
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::LostNeedsRedeem);
}

#[test]
fn task_state_transfer_phase() {
    let state = derive_auction_task_state(
        "TRANSFER",
        false,
        false,
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::TransferPendingFinalize);
}

#[test]
fn task_state_revoked_phase() {
    let state = derive_auction_task_state(
        "REVOKED",
        false,
        false,
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::UnavailableOther);
}

#[test]
fn task_state_unknown_phase_owned() {
    let state = derive_auction_task_state(
        "UNKNOWN_PHASE",
        true, // owns_name
        false,
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::OwnedNoUrgentAction);
}

#[test]
fn task_state_unknown_phase_not_owned() {
    let state = derive_auction_task_state(
        "UNKNOWN_PHASE",
        false,
        false,
        false,
        false,
        false,
        None,
        None,
        false,
        None,
        None,
        Network::Main,
    );
    assert_eq!(state, AuctionTaskState::UnavailableOther);
}

// ============================================================================
// next_action_for_task tests
// ============================================================================

#[test]
fn next_action_available_to_open() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::AvailableToOpen);
    assert_eq!(key, Some("OPEN".into()));
    assert_eq!(label, Some("Open Auction".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_waiting_for_bidding() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::WaitingForBidding);
    assert_eq!(key, Some("WAIT".into()));
    assert_eq!(label, Some("Wait for Bidding".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_ready_to_bid() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::ReadyToBid);
    assert_eq!(key, Some("BID".into()));
    assert_eq!(label, Some("Place Bid".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_ready_to_reveal() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::ReadyToReveal);
    assert_eq!(key, Some("REVEAL".into()));
    assert_eq!(label, Some("Reveal Bid".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_reveal_broadcast_pending() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::RevealBroadcastPending);
    assert_eq!(key, None); // No inline action
    assert_eq!(label, Some("Reveal pending confirmation".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_reveal_done_waiting_for_close() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::RevealDoneWaitingForClose);
    assert_eq!(key, None);
    assert_eq!(label, Some("Revealed — waiting for close".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_won_needs_register() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::WonNeedsRegister);
    assert_eq!(key, Some("REGISTER".into()));
    assert_eq!(label, Some("Register Name".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_lost_needs_redeem() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::LostNeedsRedeem);
    assert_eq!(key, Some("REDEEM".into()));
    assert_eq!(label, Some("Redeem Lockup".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_transfer_pending_finalize() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::TransferPendingFinalize);
    assert_eq!(key, Some("FINALIZE".into()));
    assert_eq!(label, Some("Finalize Transfer".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_owned_no_urgent_action() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::OwnedNoUrgentAction);
    assert_eq!(key, Some("MANAGE".into()));
    assert_eq!(label, Some("Manage Name".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_expiring_soon() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::ExpiringSoon);
    assert_eq!(key, Some("RENEW".into()));
    assert_eq!(label, Some("Renew Name".into()));
    assert!(reason.is_some());
}

#[test]
fn next_action_unavailable_other() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::UnavailableOther);
    assert_eq!(key, None);
    assert_eq!(label, None);
    assert_eq!(reason, None);
}

// ============================================================================
// build_name_action_capabilities tests
// ============================================================================

#[test]
fn cap_available_phase_can_open() {
    let action_ctx = ctx(
        false, false, false, false, None, None, None, 0, false, None, None, None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "AVAILABLE".into(),
        "AVAILABLE",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert!(caps.can_open.allowed);
    assert_eq!(caps.can_open.reason, None);
}

#[test]
fn cap_available_phase_with_pending_open_cannot_open() {
    let action_ctx = ctx(
        false, false, false, false, None, None, None, 0, true, None, None, None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "AVAILABLE".into(),
        "AVAILABLE",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert!(!caps.can_open.allowed);
    assert!(caps
        .can_open
        .reason
        .as_ref()
        .unwrap()
        .contains("already opening"));
}

#[test]
fn cap_bidding_phase_can_bid_without_commitment() {
    let action_ctx = ctx(
        false, false, false, false, None, None, None, 0, false, None, None, None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "BIDDING".into(),
        "BIDDING",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert!(caps.can_bid.allowed);
    assert_eq!(caps.can_bid.reason, None);
}

#[test]
fn cap_bidding_phase_can_bid_again_with_commitment() {
    // Multi-bid (Namebase-style): an existing commitment no longer blocks a
    // new bid during BIDDING. `can_bid` stays allowed and `my_bid_count`
    // reports how many bids this wallet already holds.
    let action_ctx = ctx(
        true, false, false, false, None, None, None, 1, false, None, None, None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "BIDDING".into(),
        "BIDDING",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert!(caps.can_bid.allowed);
    assert_eq!(caps.can_bid.reason, None);
    assert_eq!(caps.my_bid_count, 1);
}

#[test]
fn cap_bidding_phase_already_bid_stays_ready_to_bid() {
    // Multi-bid: a name in BIDDING that THIS wallet already bid on stays
    // ReadyToBid (another independent bid is allowed), so the guided next
    // action keeps inviting a bid rather than parking on "wait for reveal".
    let action_ctx = ctx(
        true, false, false, false, None, None, None, 1, false, None, None, None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "BIDDING".into(),
        "BIDDING",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert_eq!(caps.task_state, AuctionTaskState::ReadyToBid);
    assert_eq!(caps.next_action_label.as_deref(), Some("Place Bid"));
}

#[test]
fn cap_reveal_phase_can_reveal_with_commitment_and_coin() {
    let action_ctx = ctx(
        true, true, false, false, None, None, None, 0, false, None, None, None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "REVEAL".into(),
        "REVEAL",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert!(caps.can_reveal.allowed);
    assert_eq!(caps.can_reveal.reason, None);
}

#[test]
fn cap_reveal_phase_cannot_reveal_without_commitment() {
    let action_ctx = ctx(
        false, true, false, false, None, None, None, 0, false, None, None, None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "REVEAL".into(),
        "REVEAL",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert!(!caps.can_reveal.allowed);
    assert!(caps
        .can_reveal
        .reason
        .as_ref()
        .unwrap()
        .contains("no bid commitment"));
}

#[test]
fn cap_reveal_phase_cannot_reveal_without_bid_coin() {
    let action_ctx = ctx(
        true, false, false, false, None, None, None, 0, false, None, None, None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "REVEAL".into(),
        "REVEAL",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert!(!caps.can_reveal.allowed);
    assert!(caps
        .can_reveal
        .reason
        .as_ref()
        .unwrap()
        .contains("no unspent bid coin"));
}

#[test]
fn cap_closed_phase_can_redeem_lost_bid() {
    let action_ctx = NameActionContext {
        // A reveal coin that is not the name's owner — a bid that lost.
        redeemable_reveal_count: 1,
        redeemable_value_doos: 0,
        ..ctx(
            false, false, true, false, None, None, None, 0, false, None, None, None,
        )
    };
    let caps = build_name_action_capabilities(
        "example".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &action_ctx,
        false, // doesn't own
        false,
        None,
        Network::Main,
    );
    assert!(caps.can_redeem.allowed);
}

/// Owning the name is no longer what blocks a redeem — holding no reveal coin
/// besides the winning one is. A wallet that outbids itself owns the name AND
/// has losing bids to reclaim; the old rule refused those, stranding them.
#[test]
fn cap_closed_phase_cannot_redeem_when_the_only_reveal_won() {
    let action_ctx = NameActionContext {
        redeemable_reveal_count: 0,
        redeemable_value_doos: 0,
        ..ctx(
            false, false, true, false, None, None, None, 0, false, None, None, None,
        )
    };
    let caps = build_name_action_capabilities(
        "example".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &action_ctx,
        true, // owns
        false,
        None,
        Network::Main,
    );
    assert!(!caps.can_redeem.allowed);
    assert!(caps
        .can_redeem
        .reason
        .as_ref()
        .unwrap()
        .contains("won the auction"));
}

/// The case the old rule got wrong: owns the name, and still holds losing
/// reveals from its own other bids.
#[test]
fn cap_closed_phase_can_redeem_own_losing_bids_while_owning_the_name() {
    let action_ctx = NameActionContext {
        redeemable_reveal_count: 2,
        redeemable_value_doos: 0,
        ..ctx(
            false, false, true, false, None, None, None, 0, false, None, None, None,
        )
    };
    let caps = build_name_action_capabilities(
        "example".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &action_ctx,
        true, // owns the name
        false,
        None,
        Network::Main,
    );
    assert!(
        caps.can_redeem.allowed,
        "winning one bid does not forfeit the others: {:?}",
        caps.can_redeem.reason
    );
}

#[test]
fn cap_closed_phase_can_register_unregistered_win() {
    let action_ctx = ctx(
        false,
        false,
        false,
        true,
        Some(2),
        None,
        None,
        0,
        false,
        None,
        None,
        None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &action_ctx,
        true,
        false,
        None,
        Network::Main,
    );
    assert!(caps.can_register.allowed);
}

#[test]
fn cap_closed_phase_cannot_register_already_registered() {
    let action_ctx = ctx(
        false,
        false,
        false,
        true,
        Some(6),
        None,
        None,
        0,
        false,
        None,
        None,
        None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &action_ctx,
        true,
        false,
        None,
        Network::Main,
    );
    assert!(!caps.can_register.allowed);
    assert!(caps
        .can_register
        .reason
        .as_ref()
        .unwrap()
        .contains("already registered"));
}

#[test]
fn cap_owned_can_update_transfer_renew_revoke() {
    // Owned AND registered. A wallet that can spend a name holds a
    // REGISTER-or-later owner coin; leaving that unset described a state
    // production never reaches, and the actions are refused without it.
    let action_ctx = ctx(
        false,
        false,
        false,
        true,
        Some(COV_REGISTER as i64),
        None,
        None,
        0,
        false,
        None,
        None,
        None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &action_ctx,
        true, // owns_name
        false,
        None,
        Network::Main,
    );
    assert!(caps.can_update.allowed);
    assert!(caps.can_transfer.allowed);
    assert!(caps.can_renew.allowed);
    assert!(caps.can_revoke.allowed);
}

#[test]
fn cap_not_owned_cannot_update_transfer_renew_revoke() {
    let action_ctx = ctx(
        false, false, false, false, None, None, None, 0, false, None, None, None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &action_ctx,
        false, // doesn't own
        false,
        None,
        Network::Main,
    );
    assert!(!caps.can_update.allowed);
    assert!(!caps.can_transfer.allowed);
    assert!(!caps.can_renew.allowed);
    assert!(!caps.can_revoke.allowed);
}

#[test]
fn cap_spend_locked_disables_all_spend_actions() {
    let action_ctx = ctx(
        false,
        false,
        false,
        true,
        Some(6),
        None,
        None,
        0,
        false,
        None,
        None,
        None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &action_ctx,
        true,
        true, // spend_locked
        None,
        Network::Main,
    );
    assert!(!caps.can_register.allowed);
    assert!(!caps.can_update.allowed);
    assert!(!caps.can_transfer.allowed);
    assert!(!caps.can_finalize.allowed);
    assert!(!caps.can_cancel_transfer.allowed);
    assert!(!caps.can_renew.allowed);
    assert!(!caps.can_revoke.allowed);
    // All should have the same reason.
    let reason = caps.can_register.reason.as_ref().unwrap();
    assert!(reason.contains("not synced"));
}

#[test]
fn cap_transfer_phase_can_finalize_with_items() {
    // A name mid-transfer is registered by definition — its owner coin is a
    // TRANSFER covenant, which is past REGISTER.
    let action_ctx = ctx(
        false,
        false,
        false,
        true,
        Some(COV_TRANSFER as i64),
        None,
        Some(true),
        0,
        false,
        None,
        None,
        None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "TRANSFER".into(),
        "TRANSFER",
        None,
        &action_ctx,
        true,
        false,
        None,
        Network::Main,
    );
    assert!(caps.can_finalize.allowed);
}

#[test]
fn cap_transfer_phase_cannot_finalize_without_items() {
    let action_ctx = ctx(
        false,
        false,
        false,
        false,
        None,
        None,
        Some(false),
        0,
        false,
        None,
        None,
        None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "TRANSFER".into(),
        "TRANSFER",
        None,
        &action_ctx,
        true,
        false,
        None,
        Network::Main,
    );
    assert!(!caps.can_finalize.allowed);
    assert!(caps
        .can_finalize
        .reason
        .as_ref()
        .unwrap()
        .contains("not in TRANSFER"));
}

#[test]
fn cap_expiring_soon_sets_correct_task_state() {
    let action_ctx = ctx(
        false,
        false,
        false,
        true,
        Some(6),
        None,
        None,
        0,
        false,
        None,
        None,
        None,
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "CLOSED".into(),
        "CLOSED",
        None,
        &action_ctx,
        true,
        false,
        Some(15.0), // days_until_expire = 15 (below 30-day threshold),
        Network::Main,
    );
    assert_eq!(caps.task_state, AuctionTaskState::ExpiringSoon);
    let (key, _label, _) = next_action_for_task(&caps.task_state);
    assert_eq!(key, Some("RENEW".into()));
}

#[test]
fn cap_preserves_bid_value_and_reveal_txid() {
    let action_ctx = ctx(
        true,
        true,
        false,
        false,
        None,
        None,
        None,
        0,
        false,
        Some("abc123def456".into()),
        None,
        Some(100_000),
    );
    let caps = build_name_action_capabilities(
        "example".into(),
        "REVEAL".into(),
        "REVEAL",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert_eq!(caps.bid_value_doos, Some(100_000));
    assert_eq!(caps.reveal_txid, Some("abc123def456".into()));
}

// The wallet's own lockup (the on-chain value of its BID output) is OUR value,
// so `build_name_action_capabilities` must surface it from the context onto the
// capability response. Lets the modal show the user both their true bid and the
// blinded on-chain amount during BIDDING/OPENING instead of the on-chain 0.
#[test]
fn capabilities_surface_local_lockup_value() {
    let mut action_ctx = ctx(
        true,
        true,
        false,
        false,
        None,
        Some(10),
        None,
        1,
        false,
        None,
        None,
        Some(200_000),
    );
    action_ctx.lockup_value_doos = Some(500_000);

    let caps = build_name_action_capabilities(
        "example".into(),
        "BIDDING".into(),
        "BIDDING",
        None,
        &action_ctx,
        false,
        false,
        None,
        Network::Main,
    );
    assert_eq!(caps.lockup_value_doos, Some(500_000));
    assert_eq!(caps.bid_value_doos, Some(200_000));
}

// ============================================================================
// conservative_capabilities tests
// ============================================================================

#[test]
fn conservative_all_disallowed() {
    let caps = conservative_capabilities("example", "node unreachable");
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
}

#[test]
fn conservative_reason_propagated() {
    let reason = "node unreachable";
    let caps = conservative_capabilities("example", reason);
    assert_eq!(caps.next_action_reason, Some(reason.into()));
}

#[test]
fn conservative_task_state_unavailable() {
    let caps = conservative_capabilities("example", "error");
    assert_eq!(caps.task_state, AuctionTaskState::UnavailableOther);
}

#[test]
fn conservative_no_evidence() {
    let caps = conservative_capabilities("example", "error");
    assert!(!caps.owns_name);
    assert!(!caps.has_bid_commitment);
    assert!(!caps.has_bid_coin);
    assert!(!caps.has_reveal_coin);
    assert!(!caps.has_owner_coin);
    assert_eq!(caps.reveal_txid, None);
    assert_eq!(caps.bid_value_doos, None);
}
