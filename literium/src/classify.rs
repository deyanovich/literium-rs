//! Edit-category classification (SPEC §7.2).
//!
//! Per-document classification by identity-trio comparison across
//! a version pair. O(1): three string equalities.

use crate::error::Result;

/// Trio = `[volumen, spina, atom-spina]` hashes.
pub fn classify(a: &[&str; 3], b: &[&str; 3]) -> Result<&'static str> {
    let v = a[0] == b[0];
    let s = a[1] == b[1];
    let t = a[2] == b[2];

    match (v, s, t) {
        (true, true, true) => Ok("no-change"),
        (false, true, true) => Ok("formatting"),
        (false, false, true) => Ok("structural"),
        (false, false, false) => Ok("substantial"),
        (true, false, false) => Ok("re-segmentation"),
        // SPEC §7.2: every other combination is impossible by
        // construction (the atom-spina is the concatenation of the
        // fasciculi the spina lists; derived objects are a
        // function of bytes + mode + segmentation version).
        _ => Err(crate::error::Error::new(
            "classify: impossible trio combination — storage corrupt?",
        )),
    }
}
