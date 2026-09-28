//! Literium — version control for literary texts (sentence-atom).
//!
//! Rust core of the Literium specification. The v0 Perl
//! implementation (`literium-perl`, a sibling repository) is the
//! conformance oracle: the two implementations must produce
//! byte-identical derived objects on the same input. The
//! specification is normative; this crate aims to match its
//! observable behavior exactly.
//!
//! The pure-functions identity layer — content extraction
//! (SPEC §3), segmentation (SPEC §4), and derivation of the
//! content-addressed objects (SPEC §5–§6) — lives in the
//! `literium-core` crate (a workspace member) and is re-exported
//! here unchanged. This crate adds the storage, version, history,
//! and CLI layers on top of it.

pub use literium_core::{
    derive, extract, interpreter_segmentation_version, segment, SEGMENTATION_VERSION,
};

#[macro_use]
pub mod error;

pub mod exemplar;

pub mod remote;

pub mod account;

pub mod serve;

pub mod attributes;
pub mod bonds;
pub mod classify;
pub mod cli;
pub mod diff;
pub mod elevate;
pub mod fsck;
pub mod gc;
pub mod glob;
pub mod grex_ranges;
pub mod history;
pub mod ignore;
pub mod lcs;
pub mod merge;
pub mod repository;
pub mod revision_detect;
pub mod spans;
pub mod splice;
pub mod theca;
pub mod version_object;

/// SPEC §10 #1 (provisional): repository format marker content.
pub const FORMAT_VERSION: &str = "1";

/// SPEC §10 #2 (provisional): control-file names. Both files are
/// always `prose` regardless of attributes entries (SPEC §1.1/§1.2).
pub const ATTRIBUTES_FILE: &str = ".litattributes";
pub const IGNORE_FILE: &str = ".litignore";

/// SPEC §7.4 *Bonds*: the display-bonds file (syndesmos `.desm`),
/// nearest-up-the-tree per document. Not a control file: an
/// ordinary document with no bearing on identity.
pub const BONDS_FILE: &str = ".desm";
