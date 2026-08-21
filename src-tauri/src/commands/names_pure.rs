//! Pure computation helpers extracted from `names.rs` — testable without RPC/DB.

use crate::noncustodial::network::NameParams;

use serde_json::Value;

/// Compute the reveal-window-close height for a bid given the auction's open height.
///
/// The auction timeline (from hsd consensus params):
/// - OPENING: `start + (tree_interval + 1)` blocks
/// - BIDDING: next `bidding_period` blocks
/// - REVEAL: next `reveal_period` blocks
///
/// So the reveal window closes at: `start + (tree_interval + 1) + bidding_period + reveal_period`.
pub fn reveal_end_height(start_height: i64, params: &NameParams) -> i64 {
    start_height
        + (params.tree_interval as i64 + 1)
        + params.bidding_period as i64
        + params.reveal_period as i64
}

/// Format a list of names for display: single name as-is, multiple as "{first} + {n-1} more".
pub fn display_names(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        n => format!("{} + {} more", names[0], n - 1),
    }
}

/// Extract countdown data from the node's stats object.
///
/// Returns `(label, blocks, hours)` — all `None` when stats is absent or
/// the phase doesn't have a relevant countdown field.
pub fn extract_countdown(
    phase: &str,
    stats: Option<&Value>,
) -> (Option<String>, Option<i64>, Option<f64>) {
    let stats = match stats {
        Some(s) => s,
        None => return (None, None, None),
    };

    match phase {
        "OPENING" => {
            let blocks = stats.get("blocksUntilBidding").and_then(|b| b.as_i64());
            let hours = stats.get("hoursUntilBidding").and_then(|h| h.as_f64());
            (blocks.map(|_| "Bidding opens in".into()), blocks, hours)
        }
        "BIDDING" => {
            let blocks = stats.get("blocksUntilReveal").and_then(|b| b.as_i64());
            let hours = stats.get("hoursUntilReveal").and_then(|h| h.as_f64());
            (blocks.map(|_| "Reveal starts in".into()), blocks, hours)
        }
        "REVEAL" => {
            let blocks = stats.get("blocksUntilClose").and_then(|b| b.as_i64());
            let hours = stats.get("hoursUntilClose").and_then(|h| h.as_f64());
            (blocks.map(|_| "Auction closes in".into()), blocks, hours)
        }
        "CLOSED" => {
            let blocks = stats.get("blocksUntilExpire").and_then(|b| b.as_i64());
            (blocks.map(|_| "Expires in".into()), blocks, None)
        }
        _ => (None, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::network::Network;
    use serde_json::json;

    #[test]
    fn reveal_end_height_mainnet() {
        let params = Network::Main.name_params();
        // start=100, tree_interval=36, bidding_period=720, reveal_period=1440
        // => 100 + (36+1) + 720 + 1440 = 2297
        let end = reveal_end_height(100, &params);
        assert_eq!(end, 100 + (36 + 1) + 720 + 1440);
    }

    #[test]
    fn reveal_end_height_testnet() {
        let params = Network::Testnet.name_params();
        let end = reveal_end_height(50, &params);
        // testnet has different params; just verify it's > start
        assert!(end > 50);
        // testnet: tree_interval=36, bidding_period=144, reveal_period=288
        assert_eq!(end, 50 + (36 + 1) + 144 + 288);
    }

    #[test]
    fn reveal_end_height_zero_start() {
        let params = Network::Main.name_params();
        let end = reveal_end_height(0, &params);
        assert_eq!(end, (36 + 1) + 720 + 1440);
    }

    #[test]
    fn display_names_empty() {
        assert_eq!(display_names(&[]), "");
    }

    #[test]
    fn display_names_single() {
        assert_eq!(display_names(&["example".into()]), "example");
    }

    #[test]
    fn display_names_two() {
        assert_eq!(
            display_names(&["alpha".into(), "beta".into()]),
            "alpha + 1 more"
        );
    }

    #[test]
    fn display_names_many() {
        let names: Vec<String> = (0..10).map(|i| format!("name{}", i)).collect();
        assert_eq!(display_names(&names), "name0 + 9 more");
    }

    // --- extract_countdown tests ---

    #[test]
    fn countdown_none_stats_returns_all_none() {
        let (label, blocks, hours) = extract_countdown("BIDDING", None);
        assert!(label.is_none());
        assert!(blocks.is_none());
        assert!(hours.is_none());
    }

    #[test]
    fn countdown_opening_phase() {
        let stats = json!({
            "blocksUntilBidding": 42,
            "hoursUntilBidding": 7.0
        });
        let (label, blocks, hours) = extract_countdown("OPENING", Some(&stats));
        assert_eq!(label.as_deref(), Some("Bidding opens in"));
        assert_eq!(blocks, Some(42));
        assert_eq!(hours, Some(7.0));
    }

    #[test]
    fn countdown_bidding_phase() {
        let stats = json!({
            "blocksUntilReveal": 100,
            "hoursUntilReveal": 16.7
        });
        let (label, blocks, hours) = extract_countdown("BIDDING", Some(&stats));
        assert_eq!(label.as_deref(), Some("Reveal starts in"));
        assert_eq!(blocks, Some(100));
        assert_eq!(hours, Some(16.7));
    }

    #[test]
    fn countdown_reveal_phase() {
        let stats = json!({
            "blocksUntilClose": 200,
            "hoursUntilClose": 33.3
        });
        let (label, blocks, hours) = extract_countdown("REVEAL", Some(&stats));
        assert_eq!(label.as_deref(), Some("Auction closes in"));
        assert_eq!(blocks, Some(200));
        assert_eq!(hours, Some(33.3));
    }

    #[test]
    fn countdown_closed_phase() {
        let stats = json!({
            "blocksUntilExpire": 5000
        });
        let (label, blocks, hours) = extract_countdown("CLOSED", Some(&stats));
        assert_eq!(label.as_deref(), Some("Expires in"));
        assert_eq!(blocks, Some(5000));
        assert!(hours.is_none()); // CLOSED doesn't report hours
    }

    #[test]
    fn countdown_unknown_phase() {
        let stats = json!({"blocksUntilBidding": 10});
        let (label, blocks, hours) = extract_countdown("AVAILABLE", Some(&stats));
        assert!(label.is_none());
        assert!(blocks.is_none());
        assert!(hours.is_none());
    }

    #[test]
    fn countdown_missing_fields_in_stats() {
        let stats = json!({}); // no relevant fields
        let (label, blocks, hours) = extract_countdown("BIDDING", Some(&stats));
        // blocksUntilReveal absent → None
        assert!(label.is_none());
        assert!(blocks.is_none());
        assert!(hours.is_none());
    }
}
