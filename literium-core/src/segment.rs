//! Segmentation modes and the atom-edge strip (SPEC §4).
//!
//! The layer is parameterized by two boundary functions, together
//! called a mode. Implementations MUST expose this parametrization
//! as a public extension point (SPEC §4); [`register_mode`] is
//! that point. Any boundary function supplied MUST be
//! deterministic and language-agnostic — heuristic or per-locale
//! tailoring is rejected at the API layer by contract (it cannot
//! be verified mechanically; the requirement is normative on the
//! caller).

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use unicode_segmentation::UnicodeSegmentation;

use crate::extract::Extraction;

/// SPEC §4.3 — the atom-edge strip class: characters whose UAX-29
/// `Sentence_Break` property (Unicode 16.0.0) is Sp, Sep, CR, or
/// LF. The table below is the exact class extracted from perl
/// 5.42.2 (UCD 16.0.0) via
/// `/[\p{SB=Sp}\p{SB=Sep}\p{SB=CR}\p{SB=LF}]/` over all code
/// points — 25 code points; the conformance-oracle differential
/// tests hold the two implementations to the same class. This is
/// part of atom identity; it cannot change after repositories
/// exist (SPEC §4.3).
const EDGE_RANGES: &[(u32, u32)] = &[
    (0x0009, 0x000D), // TAB, LF, VT, FF, CR
    (0x0020, 0x0020), // SPACE
    (0x0085, 0x0085), // NEL (Sep)
    (0x00A0, 0x00A0), // NBSP
    (0x1680, 0x1680), // OGHAM SPACE MARK
    (0x2000, 0x200A), // EN QUAD .. HAIR SPACE
    (0x2028, 0x2029), // LINE SEPARATOR, PARAGRAPH SEPARATOR (Sep)
    (0x202F, 0x202F), // NARROW NO-BREAK SPACE
    (0x205F, 0x205F), // MEDIUM MATHEMATICAL SPACE
    (0x3000, 0x3000), // IDEOGRAPHIC SPACE
];

fn is_edge(c: char) -> bool {
    let cp = c as u32;
    EDGE_RANGES.iter().any(|&(lo, hi)| cp >= lo && cp <= hi)
}

/// The SPEC §4.3 strip, applied to both ends. Returns a subslice.
pub fn edge_strip(s: &str) -> &str {
    s.trim_matches(is_edge)
}

/// A mode spec (SPEC §4): the atom-boundary function mapping one
/// grex text to an ordered list of atom substrings (pre-strip),
/// the §3.4 within-grex join separator, and which §3 extraction
/// rules apply. `Copy` so registry lookups hand out values.
#[derive(Clone, Copy)]
pub struct ModeSpec {
    pub atom_boundary: for<'a> fn(&'a str) -> Vec<&'a str>,
    pub atom_join: &'static str,
    pub extraction: Extraction,
}

// SPEC §4.1: UAX-29 sentence boundaries, Unicode 16.0.0, without
// tailoring. `split_sentence_bounds` is unicode-segmentation's
// UAX-29 SB implementation over the crate's pinned property
// tables (verified boundary-equivalent to perl `\b{sb}` at UCD
// 16.0.0 — HISTORY §2); the repository layer enforces the pin
// (SPEC §4 mismatch rule).
fn prose_boundary(grex: &str) -> Vec<&str> {
    grex.split_sentence_bounds().collect()
}

// SPEC §4.2: split at U+000A LF; each non-empty line is one atom;
// no further segmentation.
fn poetry_boundary(grex: &str) -> Vec<&str> {
    grex.split('\n').filter(|l| !l.is_empty()).collect()
}

pub const PROSE: ModeSpec = ModeSpec {
    atom_boundary: prose_boundary,
    atom_join: " ",
    extraction: Extraction::Prose,
};

pub const POETRY: ModeSpec = ModeSpec {
    atom_boundary: poetry_boundary,
    atom_join: "\n",
    extraction: Extraction::Poetry,
};

fn registry() -> &'static RwLock<HashMap<String, ModeSpec>> {
    static REGISTRY: OnceLock<RwLock<HashMap<String, ModeSpec>>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("prose".to_string(), PROSE);
        m.insert("poetry".to_string(), POETRY);
        RwLock::new(m)
    })
}

/// The registered spec for a mode name; `None` for unknown modes.
pub fn mode_spec(name: &str) -> Option<ModeSpec> {
    registry().read().unwrap().get(name).copied()
}

/// The SPEC §4 public extension point. Errors when the name is
/// already registered. The boundary function MUST be deterministic
/// and language-agnostic (SPEC §4) — normative on the caller.
pub fn register_mode(name: &str, spec: ModeSpec) -> Result<(), ModeError> {
    let mut reg = registry().write().unwrap();
    if reg.contains_key(name) {
        return Err(ModeError::AlreadyRegistered(name.to_string()));
    }
    reg.insert(name.to_string(), spec);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeError {
    AlreadyRegistered(String),
}

impl std::fmt::Display for ModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModeError::AlreadyRegistered(n) => {
                write!(f, "mode '{n}' already registered")
            }
        }
    }
}

impl std::error::Error for ModeError {}

/// The list of atom content forms for one grex: the mode's
/// boundary-function output with the §4.3 edge strip applied to
/// every segment; segments whose content form is empty after the
/// strip are not atoms and are dropped (SPEC §4.3).
pub fn atoms_of_grex<'a>(grex: &'a str, spec: &ModeSpec) -> Vec<&'a str> {
    (spec.atom_boundary)(grex)
        .into_iter()
        .map(edge_strip)
        .filter(|a| !a.is_empty())
        .collect()
}
