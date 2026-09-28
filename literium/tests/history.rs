// Port of literium-perl t/08-history.t — SPEC §8.4, §9.

use literium::derive::hash_bytes;
use literium::history::History;
use regex_lite_check::timestamp_shape;

#[test]
fn history_behavior() {
    let dir = tempfile::tempdir().unwrap();
    let h = History::new(dir.path().join("historia"), "historia");

    assert!(h.is_empty().unwrap(), "fresh history is empty");
    assert!(h.latest().unwrap().is_none(), "no latest on empty history");

    let v1 = hash_bytes(b"version one");
    let v2 = hash_bytes(b"version two");

    h.append(&v1, "first draft", None).unwrap();
    h.append(&v2, "notes:\tafter Anna's pass\nsecond line", None)
        .unwrap();

    let entries = h.entries().unwrap();
    assert_eq!(entries.len(), 2, "two inscriptiones");
    assert_eq!(entries[0].vhash, v1, "entry order preserved");
    assert_eq!(
        entries[0].message, "first draft",
        "plain message round-trips"
    );
    assert_eq!(
        entries[1].message, "notes:\tafter Anna's pass\nsecond line",
        "tabs and newlines in message round-trip"
    );
    assert!(
        timestamp_shape(&entries[1].timestamp),
        "ISO-8601 UTC microsecond timestamp: {}",
        entries[1].timestamp
    );
    assert_eq!(
        h.latest().unwrap().unwrap().vhash,
        v2,
        "latest is last entry"
    );

    // Re-inscription of the same version hash is ordinary (SPEC §9.5).
    h.append(&v1, "returned to the earlier state", None)
        .unwrap();
    assert_eq!(
        h.latest().unwrap().unwrap().vhash,
        v1,
        "re-inscription appends normally"
    );
    assert_eq!(h.entries().unwrap().len(), 3, "multiset keeps every entry");

    // Backslash-heavy message survives.
    let tricky = "a\\b \\t literal, then real tab:\tend \\";
    h.append(&v2, tricky, None).unwrap();
    assert_eq!(
        h.entries().unwrap().last().unwrap().message,
        tricky,
        "escape-heavy message round-trips"
    );

    // Entry order is authoritative even against timestamps (§8.4).
    let h2 = History::new(dir.path().join("exp"), "peira-1");
    h2.append(&v1, "later ts first", Some("2026-07-02T10:00:00.000000Z"))
        .unwrap();
    h2.append(&v2, "earlier ts last", Some("2026-07-01T10:00:00.000000Z"))
        .unwrap();
    assert_eq!(
        h2.latest().unwrap().unwrap().vhash,
        v2,
        "order is entry order, not timestamp order"
    );

    let e = h.append("nothex", "x", None).unwrap_err();
    assert!(
        e.0.contains("malformed version hash"),
        "malformed hash rejected"
    );
}

// Tiny shape check without pulling a regex into the test: matches
// \d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6}Z
mod regex_lite_check {
    pub fn timestamp_shape(s: &str) -> bool {
        let b = s.as_bytes();
        if b.len() != 27 {
            return false;
        }
        let digit = |i: usize| b[i].is_ascii_digit();
        let lit = |i: usize, c: u8| b[i] == c;
        (0..4).all(digit)
            && lit(4, b'-')
            && (5..7).all(digit)
            && lit(7, b'-')
            && (8..10).all(digit)
            && lit(10, b'T')
            && (11..13).all(digit)
            && lit(13, b':')
            && (14..16).all(digit)
            && lit(16, b':')
            && (17..19).all(digit)
            && lit(19, b'.')
            && (20..26).all(digit)
            && lit(26, b'Z')
    }
}
