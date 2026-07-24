//! Unit tests for the auction capability and task-state derivation model.
//!
//! These tests verify the capability matrix and task-state logic in
//! `commands::names` without requiring a live node or database — they test
//! the pure derivation functions directly.

use crate::commands::names::{
    derive_auction_task_state, next_action_for_task, AuctionTaskState, EXPIRING_SOON_THRESHOLD_DAYS,
};

// Helper: no owner coin (covenant_type = None). `has_bid_coin` (unspent
// COV_BID, gates REVEAL readiness) and `has_reveal_coin` (unspent COV_REVEAL,
// only ever exists AFTER a reveal, gates CLOSED/redeem readiness) are
// deliberately separate params — Part 3 / Task 6 split what used to be one
// (misused) flag.
fn state_no_owner(
    phase: &str,
    owns_name: bool,
    has_bid: bool,
    has_bid_coin: bool,
    has_reveal: bool,
) -> AuctionTaskState {
    derive_auction_task_state(
        phase,
        owns_name,
        has_bid,
        has_bid_coin,
        has_reveal,
        owns_name,
        None,
        None,
        false,
    )
}

// Helper: owner coin is REGISTER type (≥ COV_REGISTER = 6, meaning already registered).
fn state_registered(
    phase: &str,
    owns_name: bool,
    has_bid: bool,
    has_bid_coin: bool,
    has_reveal: bool,
) -> AuctionTaskState {
    derive_auction_task_state(
        phase,
        owns_name,
        has_bid,
        has_bid_coin,
        has_reveal,
        owns_name,
        Some(6),
        None,
        false,
    )
}

// Helper: owner coin is a pre-REGISTER type like REVEAL (4) — just won, not yet registered.
fn state_unregistered(
    phase: &str,
    owns_name: bool,
    has_bid: bool,
    has_bid_coin: bool,
    has_reveal: bool,
) -> AuctionTaskState {
    derive_auction_task_state(
        phase,
        owns_name,
        has_bid,
        has_bid_coin,
        has_reveal,
        owns_name,
        Some(4),
        None,
        false,
    )
}

// Helper: registered owner coin + a known days-until-expire value.
fn state_registered_days(phase: &str, days: Option<f64>) -> AuctionTaskState {
    derive_auction_task_state(phase, true, false, false, false, true, Some(6), days, false)
}

// ============================================================================
// Task-state derivation tests
// ============================================================================

#[test]
fn available_name_yields_available_to_open() {
    let state = state_no_owner("AVAILABLE", false, false, false, false);
    assert_eq!(state, AuctionTaskState::AvailableToOpen);
}

#[test]
fn empty_phase_yields_available_to_open() {
    let state = state_no_owner("", false, false, false, false);
    assert_eq!(state, AuctionTaskState::AvailableToOpen);
}

#[test]
fn opening_phase_yields_waiting_for_bidding() {
    let state = state_no_owner("OPENING", false, false, false, false);
    assert_eq!(state, AuctionTaskState::WaitingForBidding);
}

// ============================================================================
// Task 1: pending-OPEN reflection in task-state derivation.
// ============================================================================

#[test]
fn available_with_pending_open_yields_waiting_for_bidding() {
    // A pending OPEN (our own unconfirmed draft/coin) reuses the existing
    // WaitingForBidding variant instead of AvailableToOpen, before the phase
    // itself has advanced to OPENING.
    let state = derive_auction_task_state(
        "AVAILABLE",
        false,
        false,
        false,
        false,
        false,
        None,
        None,
        true,
    );
    assert_eq!(state, AuctionTaskState::WaitingForBidding);
}

#[test]
fn empty_phase_with_pending_open_yields_waiting_for_bidding() {
    let state = derive_auction_task_state("", false, false, false, false, false, None, None, true);
    assert_eq!(state, AuctionTaskState::WaitingForBidding);
}

#[test]
fn available_without_pending_open_still_yields_available_to_open() {
    // Regression: has_pending_open=false must not change the pre-existing
    // AVAILABLE behavior.
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
    );
    assert_eq!(state, AuctionTaskState::AvailableToOpen);
}

#[test]
fn bidding_without_commitment_yields_ready_to_bid() {
    let state = state_no_owner("BIDDING", false, false, false, false);
    assert_eq!(state, AuctionTaskState::ReadyToBid);
}

#[test]
fn bidding_with_commitment_yields_waiting() {
    let state = state_no_owner("BIDDING", false, true, false, false);
    assert_eq!(state, AuctionTaskState::WaitingForBidding);
}

#[test]
fn reveal_with_bid_and_bid_coin_yields_ready_to_reveal() {
    // The realistic pre-reveal state: a bid commitment + the live, unspent
    // BID coin (no reveal coin yet — that only exists after revealing).
    let state = state_no_owner("REVEAL", false, true, true, false);
    assert_eq!(state, AuctionTaskState::ReadyToReveal);
}

#[test]
fn reveal_with_bid_but_bid_coin_not_synced_yields_ready_to_reveal() {
    // Even without a synced bid coin, if we have a bid commitment we should
    // still prompt the user to reveal (sync may be pending) — the actual
    // button gate is `can_reveal.allowed`, which DOES require `has_bid_coin`.
    let state = state_no_owner("REVEAL", false, true, false, false);
    assert_eq!(state, AuctionTaskState::ReadyToReveal);
}

#[test]
fn reveal_without_bid_yields_unavailable() {
    let state = state_no_owner("REVEAL", false, false, false, false);
    assert_eq!(state, AuctionTaskState::UnavailableOther);
}

#[test]
fn closed_with_owner_unregistered_yields_won_needs_register() {
    // Just won the auction: covenant type < COV_REGISTER (e.g. REVEAL=4).
    let state = state_unregistered("CLOSED", true, false, false, false);
    assert_eq!(state, AuctionTaskState::WonNeedsRegister);
}

#[test]
fn closed_with_owner_registered_yields_owned_no_urgent_action() {
    // Already registered: covenant type >= COV_REGISTER (6).
    let state = state_registered("CLOSED", true, false, false, false);
    assert_eq!(state, AuctionTaskState::OwnedNoUrgentAction);
}

#[test]
fn closed_with_reveal_coin_yields_lost_needs_redeem() {
    // Post-auction: no more bid coin (it was spent by the reveal), an unspent
    // REVEAL coin is what signals "lost, redeemable".
    let state = state_no_owner("CLOSED", false, false, false, true);
    assert_eq!(state, AuctionTaskState::LostNeedsRedeem);
}

#[test]
fn closed_without_owner_or_reveal_yields_owned_no_urgent_action() {
    let state = state_no_owner("CLOSED", false, false, false, false);
    assert_eq!(state, AuctionTaskState::OwnedNoUrgentAction);
}

#[test]
fn transfer_phase_yields_transfer_pending_finalize() {
    let state = state_no_owner("TRANSFER", true, false, false, false);
    assert_eq!(state, AuctionTaskState::TransferPendingFinalize);
}

#[test]
fn revoked_phase_yields_unavailable() {
    let state = state_no_owner("REVOKED", false, false, false, false);
    assert_eq!(state, AuctionTaskState::UnavailableOther);
}

#[test]
fn owned_name_in_unknown_phase_yields_owned_no_urgent_action() {
    let state = state_no_owner("UPDATE", true, false, false, false);
    assert_eq!(state, AuctionTaskState::OwnedNoUrgentAction);
}

#[test]
fn unowned_name_in_unknown_phase_yields_unavailable() {
    let state = state_no_owner("SOMETHING_ELSE", false, false, false, false);
    assert_eq!(state, AuctionTaskState::UnavailableOther);
}

// ============================================================================
// Expiring-soon derivation tests (Task 3 / C3)
// ============================================================================

#[test]
fn closed_registered_within_threshold_yields_expiring_soon() {
    let state = state_registered_days("CLOSED", Some(EXPIRING_SOON_THRESHOLD_DAYS - 1.0));
    assert_eq!(state, AuctionTaskState::ExpiringSoon);
}

#[test]
fn closed_registered_at_threshold_yields_expiring_soon() {
    let state = state_registered_days("CLOSED", Some(EXPIRING_SOON_THRESHOLD_DAYS));
    assert_eq!(state, AuctionTaskState::ExpiringSoon);
}

#[test]
fn closed_registered_beyond_threshold_yields_owned_no_urgent_action() {
    let state = state_registered_days("CLOSED", Some(EXPIRING_SOON_THRESHOLD_DAYS + 1.0));
    assert_eq!(state, AuctionTaskState::OwnedNoUrgentAction);
}

#[test]
fn closed_registered_past_expiry_yields_expiring_soon() {
    // Negative days: the window already lapsed per our data — still surface the
    // renewal alarm rather than a calm "Owned".
    let state = state_registered_days("CLOSED", Some(-3.0));
    assert_eq!(state, AuctionTaskState::ExpiringSoon);
}

#[test]
fn closed_registered_unknown_days_yields_owned_no_urgent_action() {
    let state = state_registered_days("CLOSED", None);
    assert_eq!(state, AuctionTaskState::OwnedNoUrgentAction);
}

#[test]
fn explorer_owned_without_owner_coin_within_threshold_yields_expiring_soon() {
    // Owned per explorer evidence only (no local owner coin): the expiry alarm
    // must still fire — renewals are exactly the case where staying silent
    // loses the name.
    let state = derive_auction_task_state(
        "CLOSED",
        true,
        false,
        false,
        false,
        false,
        None,
        Some(5.0),
        false,
    );
    assert_eq!(state, AuctionTaskState::ExpiringSoon);
}

#[test]
fn won_unregistered_within_threshold_still_needs_register_first() {
    // Registration takes precedence: an unregistered win can't be renewed.
    let state = derive_auction_task_state(
        "CLOSED",
        true,
        false,
        false,
        false,
        true,
        Some(4),
        Some(5.0),
        false,
    );
    assert_eq!(state, AuctionTaskState::WonNeedsRegister);
}

#[test]
fn unowned_closed_within_threshold_is_not_expiring_soon() {
    // Not our name — no renewal alarm.
    let state = derive_auction_task_state(
        "CLOSED",
        false,
        false,
        false,
        false,
        false,
        None,
        Some(5.0),
        false,
    );
    assert_eq!(state, AuctionTaskState::OwnedNoUrgentAction);
}

#[test]
fn expiring_soon_serializes_camel_case() {
    let json = serde_json::to_string(&AuctionTaskState::ExpiringSoon).unwrap();
    assert_eq!(json, "\"expiringSoon\"");
}

// ============================================================================
// Next-action derivation tests
// ============================================================================

#[test]
fn expiring_soon_next_action_is_renew() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::ExpiringSoon);
    assert_eq!(key.as_deref(), Some("RENEW"));
    assert_eq!(label.as_deref(), Some("Renew Name"));
    assert!(reason.is_some());
}

#[test]
fn available_to_open_next_action_is_open() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::AvailableToOpen);
    assert_eq!(key.as_deref(), Some("OPEN"));
    assert_eq!(label.as_deref(), Some("Open Auction"));
    assert!(reason.is_some());
}

#[test]
fn waiting_for_bidding_next_action_is_wait() {
    let (key, label, _) = next_action_for_task(&AuctionTaskState::WaitingForBidding);
    assert_eq!(key.as_deref(), Some("WAIT"));
    assert_eq!(label.as_deref(), Some("Wait for Bidding"));
}

#[test]
fn ready_to_bid_next_action_is_bid() {
    let (key, label, _) = next_action_for_task(&AuctionTaskState::ReadyToBid);
    assert_eq!(key.as_deref(), Some("BID"));
    assert_eq!(label.as_deref(), Some("Place Bid"));
}

#[test]
fn ready_to_reveal_next_action_is_reveal() {
    let (key, label, _) = next_action_for_task(&AuctionTaskState::ReadyToReveal);
    assert_eq!(key.as_deref(), Some("REVEAL"));
    assert_eq!(label.as_deref(), Some("Reveal Bid"));
}

#[test]
fn won_needs_register_next_action_is_register() {
    let (key, label, _) = next_action_for_task(&AuctionTaskState::WonNeedsRegister);
    assert_eq!(key.as_deref(), Some("REGISTER"));
    assert_eq!(label.as_deref(), Some("Register Name"));
}

#[test]
fn lost_needs_redeem_next_action_is_redeem() {
    let (key, label, _) = next_action_for_task(&AuctionTaskState::LostNeedsRedeem);
    assert_eq!(key.as_deref(), Some("REDEEM"));
    assert_eq!(label.as_deref(), Some("Redeem Lockup"));
}

#[test]
fn transfer_pending_finalize_next_action_is_finalize() {
    let (key, label, _) = next_action_for_task(&AuctionTaskState::TransferPendingFinalize);
    assert_eq!(key.as_deref(), Some("FINALIZE"));
    assert_eq!(label.as_deref(), Some("Finalize Transfer"));
}

#[test]
fn owned_no_urgent_action_next_action_is_manage() {
    let (key, label, _) = next_action_for_task(&AuctionTaskState::OwnedNoUrgentAction);
    assert_eq!(key.as_deref(), Some("MANAGE"));
    assert_eq!(label.as_deref(), Some("Manage Name"));
}

#[test]
fn unavailable_other_next_action_is_none() {
    let (key, label, reason) = next_action_for_task(&AuctionTaskState::UnavailableOther);
    assert!(key.is_none());
    assert!(label.is_none());
    assert!(reason.is_none());
}
