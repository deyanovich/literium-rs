//! Literium core — the identity layer of the Literium
//! specification.
//!
//! Pure functions only: content extraction (SPEC §3), segmentation
//! (SPEC §4), and derivation of the content-addressed objects
//! (SPEC §5–§6). No I/O, no storage, no version-control machinery —
//! those live in the `literium` crate, which depends on this one
//! and re-exports it. The entire dependency closure of this crate
//! is identity-bearing: `sha2`/`hex` fix the hash encoding and the
//! exact-version `unicode-segmentation` pin fixes the UAX-29
//! sentence boundaries (Unicode 16.0.0).
//!
//! Consumers that need Literium's atom/grex identities without the
//! VCS — derivation pipelines, citation services, derived modes via
//! [`segment::register_mode`] — depend on this crate alone. The v0
//! Perl implementation (`literium-perl`, a sibling repository) is
//! the conformance oracle: the two implementations must produce
//! byte-identical derived objects on the same input.

pub mod derive;
pub mod extract;
pub mod segment;

/// SPEC §4: segmentation is pinned to Unicode 16.0.0. This is the
/// identifier a repository created by this implementation records,
/// and what hash-producing operations demand.
pub const SEGMENTATION_VERSION: &str = "uax29-16.0.0";

/// The segmentation version this build actually segments with, in
/// the same identifier scheme as [`SEGMENTATION_VERSION`]. Derived
/// from the pinned unicode-segmentation data tables (the analogue
/// of Perl's interpreter-bundled UCD, except pinned per-build).
pub fn interpreter_segmentation_version() -> String {
    let (a, b, c) = unicode_segmentation::UNICODE_VERSION;
    format!("uax29-{a}.{b}.{c}")
}

#[cfg(test)]
mod tests {
    /// The dependency pin carries the property tables the SPEC §4
    /// Unicode pin requires. If this fails, the pin was broken.
    #[test]
    fn segmentation_data_is_unicode_16_0_0() {
        assert_eq!(unicode_segmentation::UNICODE_VERSION, (16, 0, 0));
    }
}
