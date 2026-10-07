//! `noncustodial::types`: the HNS amount formatter every confirmation row and
//! summary uses.

use crate::noncustodial::types::doos_to_hns_string;

#[test]
fn doos_to_hns_string_formats_whole_and_fractional_amounts() {
    assert_eq!(doos_to_hns_string(0), "0.000000 HNS");
    assert_eq!(doos_to_hns_string(1_000_000), "1.000000 HNS");
    assert_eq!(doos_to_hns_string(1_500_000), "1.500000 HNS");
    assert_eq!(doos_to_hns_string(2_000_123), "2.000123 HNS");
    // Negative shouldn't occur, but must not panic and keeps a sane form.
    assert_eq!(doos_to_hns_string(-1_500_000), "-1.500000 HNS");
}

#[test]
fn doos_to_hns_string_covers_edge_values() {
    // 1 doo = 0.000001 HNS (6 dp).
    assert_eq!(doos_to_hns_string(1), "0.000001 HNS");
    // Large value — no thousands separators, no rounding.
    assert_eq!(doos_to_hns_string(1_234_567_890), "1234.567890 HNS");
    // Negative: the sign is kept below one HNS too, and the fraction
    // never carries a second one.
    assert_eq!(doos_to_hns_string(-1), "-0.000001 HNS");
    assert_eq!(doos_to_hns_string(-999_999), "-0.999999 HNS");
    assert_eq!(doos_to_hns_string(-2_000_123), "-2.000123 HNS");
    // Every u64 and i64, exactly.
    assert_eq!(doos_to_hns_string(u64::MAX), "18446744073709.551615 HNS");
    assert_eq!(doos_to_hns_string(i64::MIN), "-9223372036854.775808 HNS");
}
