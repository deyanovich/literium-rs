//! Derived objects from volumen bytes (SPEC §5–§6).
//!
//! Computes, from a document's verbatim bytes and its resolved
//! mode, every derived content-addressed object the specification
//! defines: atoms (content forms), fasciculi, the spina, the
//! atom-spina, and the document content form. All hashes are
//! SHA-256 over raw object bytes (SPEC §0). Derived objects are a
//! function of *(volumen bytes, resolved mode, segmentation
//! version)* — never of bytes alone (SPEC §5).

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::extract::extract_grexes;
use crate::segment::{atoms_of_grex, mode_spec, ModeSpec};

/// SHA-256 hex digest — the specification's hash encoding (SPEC §0).
pub fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The full derivation of one document (see [`derive`]).
#[derive(Debug, Clone)]
pub struct Derivation {
    /// Raw bytes, as given.
    pub volumen: Vec<u8>,
    /// SHA-256 hex of the volumen — the document identity (SPEC §5).
    pub volumen_hash: String,
    /// The resolved mode used.
    pub mode: String,
    /// Atom content forms per atom-bearing grex.
    pub grexes: Vec<Vec<String>>,
    /// One flag per raw grex (phantom-fold input).
    pub atom_bearing: Vec<bool>,
    /// atom hash → atom bytes (repository-deduplicated).
    pub atoms: HashMap<String, Vec<u8>>,
    /// fasciculus hash → fasciculus bytes.
    pub fasciculi: HashMap<String, Vec<u8>>,
    /// Fasciculus hash per atom-bearing grex, document order.
    pub fasc_hashes: Vec<String>,
    /// Spina bytes (SPEC §6.4).
    pub spina: Vec<u8>,
    pub spina_hash: String,
    /// Atom-spina bytes (SPEC §6.6).
    pub atom_spina: Vec<u8>,
    pub atom_spina_hash: String,
    /// Document content form, bytes (SPEC §3.4).
    pub content_form: Vec<u8>,
}

/// Derives a document's objects. Returns `None` when the bytes are
/// not a document (invalid UTF-8, or empty content form — SPEC
/// §1.3).
///
/// # Panics
///
/// On an unknown mode name (programmer error; mode names reaching
/// this layer come from the validated attributes file).
pub fn derive(bytes: &[u8], mode: &str) -> Option<Derivation> {
    let spec: ModeSpec = mode_spec(mode).unwrap_or_else(|| panic!("unknown mode '{mode}'"));

    // SPEC §1.3: a document's bytes must decode as valid UTF-8.
    // Strict decode; the volumen itself stays raw bytes.
    // SPEC §0: no Unicode normalization is performed anywhere.
    let text = std::str::from_utf8(bytes).ok()?;

    let grex_texts = extract_grexes(text, spec.extraction);

    let mut grexes: Vec<Vec<String>> = Vec::new();
    let mut atom_bearing: Vec<bool> = Vec::new();
    for g in &grex_texts {
        let atoms: Vec<String> = atoms_of_grex(g, &spec)
            .into_iter()
            .map(str::to_string)
            .collect();
        atom_bearing.push(!atoms.is_empty());
        if !atoms.is_empty() {
            grexes.push(atoms);
        }
    }

    // SPEC §1.3 / §3.4: zero grex-instances -> not a document.
    if grexes.is_empty() {
        return None;
    }

    let mut atoms: HashMap<String, Vec<u8>> = HashMap::new();
    let mut fasciculi: HashMap<String, Vec<u8>> = HashMap::new();
    let mut fasc_hashes: Vec<String> = Vec::new();
    let mut all_atom_hashes: Vec<String> = Vec::new();
    for g in &grexes {
        let mut hashes: Vec<String> = Vec::new();
        for atom in g {
            // SPEC §6.2: atom objects hold the content form as
            // bytes; identity is sha256 of those bytes (SPEC §5).
            let ab = atom.as_bytes().to_vec();
            let ah = hash_bytes(&ab);
            atoms.insert(ah.clone(), ab);
            hashes.push(ah);
        }
        // SPEC §6.3: one atom hash per line, grex order, single
        // trailing LF; at least one atom hash.
        let fb = format!("{}\n", hashes.join("\n")).into_bytes();
        let fh = hash_bytes(&fb);
        fasciculi.insert(fh.clone(), fb);
        fasc_hashes.push(fh);
        all_atom_hashes.extend(hashes);
    }

    // SPEC §6.4: spina — one fasciculus hash per line, document
    // order, trailing LF.
    let spina = format!("{}\n", fasc_hashes.join("\n")).into_bytes();

    // SPEC §6.6: atom-spina — every atom hash in document order.
    let atom_spina = format!("{}\n", all_atom_hashes.join("\n")).into_bytes();

    // SPEC §3.4: the document content form is the join by
    // construction — atoms within a grex joined with the mode's
    // separator, grexes joined with a blank line, one trailing LF.
    let cf = format!(
        "{}\n",
        grexes
            .iter()
            .map(|g| g.join(spec.atom_join))
            .collect::<Vec<_>>()
            .join("\n\n")
    )
    .into_bytes();

    let spina_hash = hash_bytes(&spina);
    let atom_spina_hash = hash_bytes(&atom_spina);
    Some(Derivation {
        volumen: bytes.to_vec(),
        volumen_hash: hash_bytes(bytes),
        mode: mode.to_string(),
        grexes,
        atom_bearing,
        atoms,
        fasciculi,
        fasc_hashes,
        spina,
        spina_hash,
        atom_spina,
        atom_spina_hash,
        content_form: cf,
    })
}

/// SPEC §6.5 — the reconstruction walk: spina → fasciculi →
/// atoms, joined per §3.4. Returns content-form bytes.
///
/// # Panics
///
/// On a dangling hash (the croak-equivalent; fsck-level callers
/// catch this by checking presence first).
pub fn reconstruct(
    spina: &[u8],
    fasciculi: &HashMap<String, Vec<u8>>,
    atoms: &HashMap<String, Vec<u8>>,
    mode: &str,
) -> Vec<u8> {
    let spec = mode_spec(mode).unwrap_or_else(|| panic!("unknown mode '{mode}'"));
    let join_bytes = spec.atom_join.as_bytes();

    let spina_text = std::str::from_utf8(spina).expect("spina must be UTF-8");
    let mut grex_bytes: Vec<Vec<u8>> = Vec::new();
    for fh in spina_text.split('\n').filter(|l| !l.is_empty()) {
        let fb = fasciculi
            .get(fh)
            .unwrap_or_else(|| panic!("reconstruct: missing fasciculus {fh}"));
        let fb_text = std::str::from_utf8(fb).expect("fasciculus must be UTF-8");
        let mut ab: Vec<&[u8]> = Vec::new();
        for ah in fb_text.split('\n').filter(|l| !l.is_empty()) {
            ab.push(
                atoms
                    .get(ah)
                    .unwrap_or_else(|| panic!("reconstruct: missing atom {ah}")),
            );
        }
        grex_bytes.push(ab.join(join_bytes));
    }
    let mut out = grex_bytes.join(&b"\n\n"[..]);
    out.push(b'\n');
    out
}
