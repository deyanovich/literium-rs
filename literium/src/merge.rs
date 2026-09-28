//! The combining merge (SPEC §9.6): `lit contexe`.
//!
//! Two-level diff3 over content-addressed hashes — fasciculus
//! skeleton, flat-atom descent — with trio fast paths,
//! target-wins formatting, inline conflict materialization from
//! verbatim atom spans, and the contextio ledger.

use std::collections::BTreeMap;

use crate::derive::Derivation;
use crate::error::Result;
use crate::lcs;
use crate::revision_detect::SideInput;
use crate::spans::{assembly_bytes, atom_spans, Span};

/// One diff3 chunk over some hash sequence. Ranges are half-open
/// index ranges into the A and B sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Chunk {
    /// Anchored in both alignments (or chosen): take A's element.
    TakeA(usize, usize),
    TakeB(usize, usize),
    Conflict {
        /// The corresponding O gap ((0, 0) when baseless).
        o: (usize, usize),
        a: (usize, usize),
        b: (usize, usize),
    },
}

/// Classic diff3 composition: skeleton anchors common to LCS(O,A)
/// and LCS(O,B); per gap, one-side changes compose, both-side
/// changes conflict. Baseless (o = None) degenerates to a 2-way.
fn diff3<H: PartialEq + Eq + std::hash::Hash + Clone>(
    o: Option<&[H]>,
    a: &[H],
    b: &[H],
) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    match o {
        None => {
            // 2-way: anchors take A's element; gaps with one side
            // empty compose; both non-empty conflict.
            let anchors = lcs::anchors(a, b);
            let (mut ai, mut bi) = (0usize, 0usize);
            let sentinel = [(a.len(), b.len())];
            for &(aa, bb) in anchors.iter().chain(sentinel.iter()) {
                if aa > ai && bb > bi {
                    chunks.push(Chunk::Conflict {
                        o: (0, 0),
                        a: (ai, aa),
                        b: (bi, bb),
                    });
                } else if aa > ai {
                    chunks.push(Chunk::TakeA(ai, aa));
                } else if bb > bi {
                    chunks.push(Chunk::TakeB(bi, bb));
                }
                if aa < a.len() {
                    chunks.push(Chunk::TakeA(aa, aa + 1));
                }
                ai = aa + 1;
                bi = bb + 1;
            }
        }
        Some(o) => {
            // Skeleton: O indices anchored in both alignments.
            let oa: BTreeMap<usize, usize> = lcs::anchors(o, a).into_iter().collect();
            let ob: BTreeMap<usize, usize> = lcs::anchors(o, b).into_iter().collect();
            let skeleton: Vec<(usize, usize, usize)> = (0..o.len())
                .filter_map(|oi| match (oa.get(&oi), ob.get(&oi)) {
                    (Some(&ai), Some(&bi)) => Some((oi, ai, bi)),
                    _ => None,
                })
                .collect();

            let (mut oi, mut ai, mut bi) = (0usize, 0usize, 0usize);
            let sentinel = [(o.len(), a.len(), b.len())];
            for &(so, sa, sb) in skeleton.iter().chain(sentinel.iter()) {
                let o_gap = &o[oi..so];
                let a_gap = &a[ai..sa];
                let b_gap = &b[bi..sb];
                let a_changed = a_gap != o_gap;
                let b_changed = b_gap != o_gap;
                match (a_changed, b_changed) {
                    (false, false) => {
                        // Unchanged region: target's copy.
                        if sa > ai {
                            chunks.push(Chunk::TakeA(ai, sa));
                        }
                    }
                    (true, false) => {
                        if sa > ai {
                            chunks.push(Chunk::TakeA(ai, sa));
                        }
                    }
                    (false, true) => {
                        if sb > bi {
                            chunks.push(Chunk::TakeB(bi, sb));
                        }
                    }
                    (true, true) => {
                        if a_gap == b_gap {
                            // Identical change on both sides.
                            if sa > ai {
                                chunks.push(Chunk::TakeA(ai, sa));
                            }
                        } else {
                            chunks.push(Chunk::Conflict {
                                o: (oi, so),
                                a: (ai, sa),
                                b: (bi, sb),
                            });
                        }
                    }
                }
                if so < o.len() {
                    // The skeleton element itself: target's copy.
                    chunks.push(Chunk::TakeA(sa, sa + 1));
                }
                oi = so + 1;
                ai = sa + 1;
                bi = sb + 1;
            }
        }
    }
    // Coalesce adjacent same-kind takes.
    let mut out: Vec<Chunk> = Vec::new();
    for c in chunks {
        match (out.last_mut(), &c) {
            (Some(Chunk::TakeA(_, e)), Chunk::TakeA(s2, e2)) if *e == *s2 => *e = *e2,
            (Some(Chunk::TakeB(_, e)), Chunk::TakeB(s2, e2)) if *e == *s2 => *e = *e2,
            _ => out.push(c),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

/// One output atom of a content merge: its side and (grex, atom)
/// position there, plus whether its side had a grex boundary
/// after it and whether it ends a conflict's A-variant (boundary
/// suppressed — the variants join within one grex, SPEC §9.6).
struct OutAtom {
    side: Side,
    g: usize,
    a: usize,
    boundary_after: bool,
    joins_conflict: bool,
    conflict: Option<usize>, // index into the document's conflicts
}

/// One conflict of a content merge, atom-level.
#[derive(Debug, Clone)]
pub struct DocConflict {
    pub kind: &'static str,
    /// Coordinate range of the inline region in the materialized
    /// document ("g2a3..g2a5"), or "-" for document-level kinds.
    pub coords: String,
    pub a_atoms: Vec<String>,
    pub b_atoms: Vec<String>,
}

/// A whole-tree conflict record (ledger entry).
#[derive(Debug, Clone)]
pub struct Conflict {
    pub path: String,
    pub kind: String,
    pub coords: String,
    pub a_atoms: Vec<String>,
    pub b_atoms: Vec<String>,
}

pub struct Outcome {
    /// path → materialized bytes of the merged document set.
    pub documents: BTreeMap<String, Vec<u8>>,
    pub conflicts: Vec<Conflict>,
    /// Paths taken wholesale vs content-merged (for the summary).
    pub merged_paths: Vec<String>,
}

/// Flat atom view of one side's grex range, with provenance.
struct FlatAtoms<'d> {
    hashes: Vec<&'d str>,
    /// (grex index, atom index) per flat position.
    pos: Vec<(usize, usize)>,
}

fn flat(d: &Derivation, grexes: std::ops::Range<usize>) -> FlatAtoms<'_> {
    let mut hashes = Vec::new();
    let mut pos = Vec::new();
    for g in grexes {
        let fh = &d.fasc_hashes[g];
        let fb = &d.fasciculi[fh];
        let atom_hashes: Vec<&str> = std::str::from_utf8(fb)
            .expect("fasciculus is hex + LF")
            .split('\n')
            .filter(|l| !l.is_empty())
            .collect();
        for (a, h) in atom_hashes.into_iter().enumerate() {
            hashes.push(h);
            pos.push((g, a));
        }
    }
    FlatAtoms { hashes, pos }
}

fn last_in_grex(d: &Derivation, g: usize, a: usize) -> bool {
    a + 1 == d.grexes[g].len()
}

/// Content-merges one document (A = target, B = source, O = base
/// when present). Returns the materialized bytes and atom-level
/// conflicts.
pub fn merge_document(
    a: &Derivation,
    b: &Derivation,
    o: Option<&Derivation>,
) -> Result<(Vec<u8>, Vec<DocConflict>)> {
    let a_spans = atom_spans(a)?;
    let b_spans = atom_spans(b)?;
    let a_ranges = crate::grex_ranges::extract(&a.volumen, &a.atom_bearing)?;
    let b_ranges = crate::grex_ranges::extract(&b.volumen, &b.atom_bearing)?;

    // Level 1: fasciculus-hash diff3.
    let level1 = diff3(
        o.map(|o| o.fasc_hashes.as_slice()),
        &a.fasc_hashes,
        &b.fasc_hashes,
    );

    // The output stream: whole grexes taken verbatim, or merged
    // atom streams from the level-2 descent.
    enum Piece {
        WholeA(usize),
        WholeB(usize),
        Atoms(Vec<OutAtom>),
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut conflicts: Vec<DocConflict> = Vec::new();

    for chunk in level1 {
        match chunk {
            Chunk::TakeA(s, e) => pieces.extend((s..e).map(Piece::WholeA)),
            Chunk::TakeB(s, e) => pieces.extend((s..e).map(Piece::WholeB)),
            Chunk::Conflict {
                o: or_,
                a: ar,
                b: br,
            } => {
                // Level 2: flat-atom diff3 over the gap — each
                // side's atoms clipped to its own gap range.
                let fa = flat(a, ar.0..ar.1);
                let fb = flat(b, br.0..br.1);
                let fo = o.map(|od| flat(od, or_.0..or_.1));
                let level2 = diff3(
                    fo.as_ref().map(|f| f.hashes.as_slice()),
                    &fa.hashes,
                    &fb.hashes,
                );
                let mut atoms: Vec<OutAtom> = Vec::new();
                for c2 in level2 {
                    match c2 {
                        Chunk::TakeA(s, e) => {
                            for i in s..e {
                                let (g, at) = fa.pos[i];
                                atoms.push(OutAtom {
                                    side: Side::A,
                                    g,
                                    a: at,
                                    boundary_after: last_in_grex(a, g, at),
                                    joins_conflict: false,
                                    conflict: None,
                                });
                            }
                        }
                        Chunk::TakeB(s, e) => {
                            for i in s..e {
                                let (g, at) = fb.pos[i];
                                atoms.push(OutAtom {
                                    side: Side::B,
                                    g,
                                    a: at,
                                    boundary_after: last_in_grex(b, g, at),
                                    joins_conflict: false,
                                    conflict: None,
                                });
                            }
                        }
                        Chunk::Conflict { a: ar2, b: br2, .. } => {
                            let ci = conflicts.len();
                            conflicts.push(DocConflict {
                                kind: "text",
                                coords: String::new(),
                                a_atoms: fa.hashes[ar2.0..ar2.1]
                                    .iter()
                                    .map(|h| h.to_string())
                                    .collect(),
                                b_atoms: fb.hashes[br2.0..br2.1]
                                    .iter()
                                    .map(|h| h.to_string())
                                    .collect(),
                            });
                            for i in ar2.0..ar2.1 {
                                let (g, at) = fa.pos[i];
                                let last = i + 1 == ar2.1;
                                atoms.push(OutAtom {
                                    side: Side::A,
                                    g,
                                    a: at,
                                    boundary_after: last_in_grex(a, g, at),
                                    joins_conflict: last, // suppress boundary at the seam
                                    conflict: Some(ci),
                                });
                            }
                            for i in br2.0..br2.1 {
                                let (g, at) = fb.pos[i];
                                atoms.push(OutAtom {
                                    side: Side::B,
                                    g,
                                    a: at,
                                    boundary_after: last_in_grex(b, g, at),
                                    joins_conflict: false,
                                    conflict: Some(ci),
                                });
                            }
                        }
                    }
                }
                pieces.push(Piece::Atoms(atoms));
            }
        }
    }

    // Assemble: each output grex becomes a byte chunk without its
    // trailing separator; chunks join with "\n\n"; the document is
    // terminated with a single LF (v1 micro-rule).
    let joiner: &[u8] = match a.mode.as_str() {
        "poetry" => b"\n",
        _ => b" ",
    };
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    // Track output coordinates for conflict regions.
    let mut coord_ranges: Vec<Option<((usize, usize), (usize, usize))>> =
        vec![None; conflicts.len()];

    let mut cur: Vec<Vec<u8>> = Vec::new(); // atoms of the open merged grex
    let mut cur_atoms = 0usize;

    let rtrim = |bytes: &[u8]| -> Vec<u8> {
        let s = std::str::from_utf8(bytes).expect("volumen is UTF-8");
        s.trim_end_matches(|c: char| crate::segment::edge_strip(&c.to_string()).is_empty())
            .as_bytes()
            .to_vec()
    };

    for piece in &pieces {
        match piece {
            Piece::WholeA(g) => {
                let (s, e) = a_ranges[*g];
                chunks.push(rtrim(&a.volumen[s..e]));
            }
            Piece::WholeB(g) => {
                let (s, e) = b_ranges[*g];
                chunks.push(rtrim(&b.volumen[s..e]));
            }
            Piece::Atoms(atoms) => {
                for oa in atoms {
                    let (vol, spans): (&[u8], &Vec<Vec<Span>>) = match oa.side {
                        Side::A => (&a.volumen, &a_spans),
                        Side::B => (&b.volumen, &b_spans),
                    };
                    let span = spans[oa.g][oa.a];
                    let bytes = assembly_bytes(vol, &span).to_vec();
                    // Output coordinate of this atom: grex ordinal =
                    // chunks.len() + 1 (the open grex), atom ordinal
                    // = cur_atoms + 1.
                    let coord = (chunks.len() + 1, cur_atoms + 1);
                    if let Some(ci) = oa.conflict {
                        let entry = &mut coord_ranges[ci];
                        match entry {
                            None => *entry = Some((coord, coord)),
                            Some((_, end)) => *end = coord,
                        }
                    }
                    cur.push(bytes);
                    cur_atoms += 1;
                    let boundary = oa.boundary_after && !oa.joins_conflict;
                    if boundary {
                        chunks.push(cur.join(joiner));
                        cur.clear();
                        cur_atoms = 0;
                    }
                }
                if !cur.is_empty() {
                    chunks.push(cur.join(joiner));
                    cur.clear();
                    cur_atoms = 0;
                }
            }
        }
    }

    for (ci, range) in coord_ranges.iter().enumerate() {
        if let Some(((g1, a1), (g2, a2))) = range {
            conflicts[ci].coords = format!("g{g1}a{a1}..g{g2}a{a2}");
        }
    }

    let mut doc: Vec<u8> = chunks.join(&b"\n\n"[..]);
    doc.push(b'\n');
    Ok((doc, conflicts))
}

/// Whole-tree combine (SPEC §9.6 document pairing + content
/// merges). Sides are the usual manifest+states shape.
pub fn combine(a: &SideInput, b: &SideInput, o: Option<&SideInput>) -> Result<Outcome> {
    let mut documents: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut conflicts: Vec<Conflict> = Vec::new();
    let mut merged_paths: Vec<String> = Vec::new();

    let mut paths: Vec<&String> = a.manifest.keys().collect();
    for p in b.manifest.keys() {
        if !a.manifest.contains_key(p) {
            paths.push(p);
        }
    }
    if let Some(o) = o {
        for p in o.manifest.keys() {
            if !a.manifest.contains_key(p) && !b.manifest.contains_key(p) {
                paths.push(p);
            }
        }
    }
    paths.sort();

    for path in paths {
        let ah = a.manifest.get(path);
        let bh = b.manifest.get(path);
        let oh = o.and_then(|o| o.manifest.get(path));
        match (ah, bh) {
            (None, None) => {} // removed on both sides (or O-only)
            (Some(_), None) => {
                let a_state = &a.states[path.as_str()];
                match oh {
                    None => {
                        // Added in A only: keep.
                        documents.insert(path.clone(), a_state.volumen.clone());
                    }
                    Some(oh) if oh == &a_state.volumen_hash => {
                        // Removed in B, unchanged in A: removed.
                    }
                    Some(_) => {
                        // Modified in A, removed in B: keep A,
                        // record the conflict.
                        documents.insert(path.clone(), a_state.volumen.clone());
                        conflicts.push(Conflict {
                            path: path.clone(),
                            kind: "modify-delete".to_string(),
                            coords: "-".to_string(),
                            a_atoms: a_state
                                .grexes
                                .iter()
                                .flatten()
                                .map(|t| crate::derive::hash_bytes(t.as_bytes()))
                                .collect(),
                            b_atoms: Vec::new(),
                        });
                    }
                }
            }
            (None, Some(_)) => {
                let b_state = &b.states[path.as_str()];
                match oh {
                    None => {
                        documents.insert(path.clone(), b_state.volumen.clone());
                    }
                    Some(oh) if oh == &b_state.volumen_hash => {}
                    Some(_) => {
                        documents.insert(path.clone(), b_state.volumen.clone());
                        conflicts.push(Conflict {
                            path: path.clone(),
                            kind: "delete-modify".to_string(),
                            coords: "-".to_string(),
                            a_atoms: Vec::new(),
                            b_atoms: b_state
                                .grexes
                                .iter()
                                .flatten()
                                .map(|t| crate::derive::hash_bytes(t.as_bytes()))
                                .collect(),
                        });
                    }
                }
            }
            (Some(ah), Some(bh)) => {
                let a_state = &a.states[path.as_str()];
                let b_state = &b.states[path.as_str()];
                if ah == bh {
                    documents.insert(path.clone(), a_state.volumen.clone());
                    continue;
                }
                if let Some(oh) = oh {
                    if oh == ah {
                        // Only B changed.
                        documents.insert(path.clone(), b_state.volumen.clone());
                        continue;
                    }
                    if oh == bh {
                        documents.insert(path.clone(), a_state.volumen.clone());
                        continue;
                    }
                }
                // Content merge.
                let o_state = match (o, oh) {
                    (Some(o), Some(_)) => o.states.get(path.as_str()),
                    _ => None,
                };
                let (bytes, doc_conflicts) = merge_document(a_state, b_state, o_state)?;
                documents.insert(path.clone(), bytes);
                merged_paths.push(path.clone());
                for dc in doc_conflicts {
                    conflicts.push(Conflict {
                        path: path.clone(),
                        kind: dc.kind.to_string(),
                        coords: if dc.coords.is_empty() {
                            "-".to_string()
                        } else {
                            dc.coords
                        },
                        a_atoms: dc.a_atoms,
                        b_atoms: dc.b_atoms,
                    });
                }
            }
        }
    }

    Ok(Outcome {
        documents,
        conflicts,
        merged_paths,
    })
}

// ── The contextio ledger (SPEC §9.6, §6.9) ──────────────────────

/// The persisted merge/conflict state at `.lit/contextio`.
#[derive(Debug, Clone)]
pub struct Ledger {
    pub target_name: String,
    pub target_vhash: Option<String>,
    pub source_label: String,
    pub source_vhash: String,
    pub base: Option<String>,
    pub conflicts: Vec<Conflict>,
}

fn encode_field(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\t', "%09")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

fn decode_field(s: &str) -> String {
    s.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%09", "\t")
        .replace("%25", "%")
}

impl Ledger {
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = String::from("lit-contextio 1\n");
        out.push_str(&format!(
            "target {} {}\n",
            encode_field(&self.target_name),
            self.target_vhash.as_deref().unwrap_or("none")
        ));
        out.push_str(&format!(
            "source {} {}\n",
            encode_field(&self.source_label),
            self.source_vhash
        ));
        out.push_str(&format!(
            "base {}\n",
            self.base.as_deref().unwrap_or("none")
        ));
        for c in &self.conflicts {
            let a = if c.a_atoms.is_empty() {
                "-".to_string()
            } else {
                c.a_atoms.join(",")
            };
            let b = if c.b_atoms.is_empty() {
                "-".to_string()
            } else {
                c.b_atoms.join(",")
            };
            out.push_str(&format!(
                "conflict\t{}\t{}\t{}\t{a}\t{b}\n",
                c.kind,
                encode_field(&c.path),
                c.coords
            ));
        }
        out.into_bytes()
    }

    pub fn parse(bytes: &[u8]) -> Result<Ledger> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| crate::error::Error::new("contextio: not UTF-8"))?;
        let mut lines = text.split('\n');
        if lines.next() != Some("lit-contextio 1") {
            err!("contextio: bad header");
        }
        let target_line = lines.next().unwrap_or("");
        let source_line = lines.next().unwrap_or("");
        let base_line = lines.next().unwrap_or("");
        let (Some(t), Some(s), Some(b)) = (
            target_line.strip_prefix("target "),
            source_line.strip_prefix("source "),
            base_line.strip_prefix("base "),
        ) else {
            err!("contextio: malformed header lines");
        };
        let (tname, tvh) = t
            .rsplit_once(' ')
            .ok_or_else(|| crate::error::Error::new("contextio: bad target line"))?;
        let (slabel, svh) = s
            .rsplit_once(' ')
            .ok_or_else(|| crate::error::Error::new("contextio: bad source line"))?;
        let mut conflicts = Vec::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() != 6 || parts[0] != "conflict" {
                err!("contextio: malformed conflict line '{line}'");
            }
            let split_hashes = |s: &str| -> Vec<String> {
                if s == "-" {
                    Vec::new()
                } else {
                    s.split(',').map(str::to_string).collect()
                }
            };
            conflicts.push(Conflict {
                kind: parts[1].to_string(),
                path: decode_field(parts[2]),
                coords: parts[3].to_string(),
                a_atoms: split_hashes(parts[4]),
                b_atoms: split_hashes(parts[5]),
            });
        }
        Ok(Ledger {
            target_name: decode_field(tname),
            target_vhash: (tvh != "none").then(|| tvh.to_string()),
            source_label: decode_field(slabel),
            source_vhash: svh.to_string(),
            base: (b != "none").then(|| b.to_string()),
            conflicts,
        })
    }
}
