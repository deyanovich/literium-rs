//! Pending-revision decomposition (SPEC §8.5, §10 #5;
//! PROVISIONAL.md #5).
//!
//! Aligns the parent version against the tabula and decomposes
//! the difference into the specification's revision kinds:
//! per-grex edit revisions (at most one edit-bearing revision per
//! grex), formatting revisions over contiguous touched runs, and
//! the manifest kinds — rename, removal, first version.
//! Deterministic, similarity-scoring-free. The pending set is a
//! pure function of its two inputs; indices renumber on every
//! recomputation (SPEC §8.8 provisional contract).

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::derive::Derivation;
use crate::error::Result;
use crate::grex_ranges;
use crate::lcs;

/// One side of the comparison: manifest (path → volumen hash) and
/// per-path derivations.
pub struct SideInput {
    pub manifest: BTreeMap<String, String>,
    pub states: HashMap<String, Derivation>,
}

/// A derivation plus its verbatim grex ranges.
pub struct DocState {
    pub derivation: Derivation,
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellType {
    Anchor,
    Regroup,
    Pair,
    Del,
    Ins,
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub cell_type: CellType,
    pub p: Vec<usize>,
    pub t: Vec<usize>,
    pub touched: bool,
    /// The id of the revision this cell belongs to, if any.
    pub rev: Option<String>,
}

/// Parent↔tabula correspondence for one changed document.
pub struct DocCorrespondence {
    pub cells: Vec<Cell>,
    pub parent: DocState,
    pub tabula: DocState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevKind {
    Edit,
    Formatting,
    Rename,
    Removal,
    FirstVersion,
}

#[derive(Debug, Clone)]
pub struct Revision {
    pub id: String,
    pub index: usize,
    pub kind: RevKind,
    /// Edit category for edit/formatting kinds.
    pub category: Option<&'static str>,
    /// Display detail for edit kinds (modified / regrouped / ...).
    pub what: Option<&'static str>,
    /// Ordering key: manifest path (former path for removals and
    /// renames, SPEC §8.5).
    pub path: String,
    pub new_path: Option<String>,
    /// The document this revision's cells belong to, if any.
    pub doc: Option<String>,
    pub cells: Vec<usize>,
    pos: usize,
    pub parent_grexes: Vec<usize>,
    pub tabula_grexes: Vec<usize>,
    pub snippet: String,
}

pub struct Pending {
    pub revisions: Vec<Revision>,
    pub docs: BTreeMap<String, DocCorrespondence>,
}

pub fn detect(parent: &SideInput, tabula: &SideInput) -> Result<Pending> {
    let mut revisions: Vec<Revision> = Vec::new();
    let mut docs: BTreeMap<String, DocCorrespondence> = BTreeMap::new();

    // ── Document-level pairing ──────────────────────────────────
    let mut paired: Vec<&String> = Vec::new();
    let mut consumed_p: HashSet<&str> = HashSet::new();
    let mut consumed_t: HashSet<&str> = HashSet::new();
    for path in parent.manifest.keys() {
        if tabula.manifest.contains_key(path) {
            paired.push(path);
            consumed_p.insert(path);
            consumed_t.insert(path);
        }
    }

    // Leftovers pair as renames on equal identity trios; the
    // tie-break when one trio sits at several paths is
    // sort-and-zip (PROVISIONAL.md #5).
    let mut p_by_trio: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for path in parent.manifest.keys() {
        if !consumed_p.contains(path.as_str()) {
            p_by_trio
                .entry(trio_key(&parent.states[path.as_str()]))
                .or_default()
                .push(path);
        }
    }
    let mut t_by_trio: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for path in tabula.manifest.keys() {
        if !consumed_t.contains(path.as_str()) {
            t_by_trio
                .entry(trio_key(&tabula.states[path.as_str()]))
                .or_default()
                .push(path);
        }
    }
    let mut renames: Vec<(String, String)> = Vec::new();
    for (trio, old_paths) in &p_by_trio {
        let Some(new_paths) = t_by_trio.get(trio) else {
            continue;
        };
        for (o, n) in old_paths.iter().zip(new_paths.iter()) {
            renames.push((o.to_string(), n.to_string()));
            consumed_p.insert(o);
            consumed_t.insert(n);
        }
    }

    let removed: Vec<&String> = parent
        .manifest
        .keys()
        .filter(|p| !consumed_p.contains(p.as_str()))
        .collect();
    let added: Vec<&String> = tabula
        .manifest
        .keys()
        .filter(|p| !consumed_t.contains(p.as_str()))
        .collect();

    // ── Per-document decomposition for paired documents ─────────
    for path in &paired {
        let ps = &parent.states[path.as_str()];
        let ts = &tabula.states[path.as_str()];
        // Same bytes: zero revisions — a mode change with
        // unchanged bytes rides the attributes document's own
        // revision (re-segmentation, SPEC §1.1/§7.2).
        if ps.volumen_hash == ts.volumen_hash {
            continue;
        }
        let (corr, revs) = decompose_document(path, ps, ts)?;
        docs.insert(path.to_string(), corr);
        revisions.extend(revs);
    }

    // ── Manifest-kind revisions ─────────────────────────────────
    for (old, new) in &renames {
        revisions.push(Revision {
            id: format!("rename:{old}"),
            index: 0,
            kind: RevKind::Rename,
            category: None,
            what: None,
            path: old.clone(), // ordering key: former path (§8.5)
            new_path: Some(new.clone()),
            doc: None,
            cells: Vec::new(),
            pos: 0,
            parent_grexes: Vec::new(),
            tabula_grexes: Vec::new(),
            snippet: format!("{old} -> {new}"),
        });
    }
    for path in &removed {
        revisions.push(Revision {
            id: format!("removal:{path}"),
            index: 0,
            kind: RevKind::Removal,
            category: None,
            what: None,
            path: path.to_string(),
            new_path: None,
            doc: None,
            cells: Vec::new(),
            pos: 0,
            parent_grexes: Vec::new(),
            tabula_grexes: Vec::new(),
            snippet: "document removed".to_string(),
        });
    }
    for path in &added {
        let first_atom = tabula.states[path.as_str()]
            .grexes
            .first()
            .and_then(|g| g.first())
            .map(String::as_str);
        revisions.push(Revision {
            id: format!("first:{path}"),
            index: 0,
            kind: RevKind::FirstVersion,
            category: None,
            what: None,
            path: path.to_string(),
            new_path: None,
            doc: None,
            cells: Vec::new(),
            pos: 0,
            parent_grexes: Vec::new(),
            tabula_grexes: Vec::new(),
            snippet: snip(first_atom),
        });
    }

    // ── Ordering and index assignment (SPEC §8.5) ───────────────
    // Document order: manifest path (former path for removals and
    // renames), bytewise; then grex order within a document (the
    // cell position, already encoded in each edit revision).
    revisions.sort_by(|a, b| a.path.cmp(&b.path).then(a.pos.cmp(&b.pos)));
    for (i, rev) in revisions.iter_mut().enumerate() {
        rev.index = i + 1;
    }

    Ok(Pending { revisions, docs })
}

fn trio_key(d: &Derivation) -> String {
    format!("{} {} {}", d.volumen_hash, d.spina_hash, d.atom_spina_hash)
}

fn snip(atom: Option<&str>) -> String {
    let Some(atom) = atom else {
        return String::new();
    };
    // Collapse whitespace runs (Unicode \s) to single spaces.
    let mut out = String::with_capacity(atom.len());
    let mut in_run = false;
    for c in atom.chars() {
        if c.is_whitespace() {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            out.push(c);
            in_run = false;
        }
    }
    let chars: Vec<char> = out.chars().collect();
    if chars.len() > 60 {
        let head: String = chars[..57].iter().collect();
        format!("{head}...")
    } else {
        out
    }
}

// ── Per-document alignment ──────────────────────────────────────

/// Aligns two derivations of one document into the interleaved
/// cell list (SPEC §8.8's anchors + Rule A/Rule B gap cells), with
/// touched flags and no revision assignment. Shared by revision
/// detection, diff display (SPEC §7.4), and the combining merge.
pub fn align(ps: &Derivation, ts: &Derivation) -> Result<DocCorrespondence> {
    let p_ranges = grex_ranges::extract(&ps.volumen, &ps.atom_bearing)?;
    let t_ranges = grex_ranges::extract(&ts.volumen, &ts.atom_bearing)?;

    let p_state = DocState {
        derivation: ps.clone(),
        ranges: p_ranges,
    };
    let t_state = DocState {
        derivation: ts.clone(),
        ranges: t_ranges,
    };

    let pf = &ps.fasc_hashes;
    let tf = &ts.fasc_hashes;

    // LCS over fasciculus hashes, unique leftmost trace.
    let anchors = lcs::anchors(pf, tf);

    // Interleaved cell list: anchors and gap cells, monotone in
    // both grex index sequences.
    let mut cells: Vec<Cell> = Vec::new();
    let (mut pi, mut tj) = (0usize, 0usize);
    let sentinel = [(pf.len(), tf.len())];
    for &(ai, aj) in anchors.iter().chain(sentinel.iter()) {
        if ai > pi || aj > tj {
            gap_cells(&mut cells, &p_state, &t_state, pi, ai, tj, aj);
        }
        if ai < pf.len() {
            let touched = range_bytes(&p_state, ai) != range_bytes(&t_state, aj);
            cells.push(Cell {
                cell_type: CellType::Anchor,
                p: vec![ai],
                t: vec![aj],
                touched,
                rev: None,
            });
        }
        pi = ai + 1;
        tj = aj + 1;
    }

    Ok(DocCorrespondence {
        cells,
        parent: p_state,
        tabula: t_state,
    })
}

fn decompose_document(
    path: &str,
    ps: &Derivation,
    ts: &Derivation,
) -> Result<(DocCorrespondence, Vec<Revision>)> {
    let mut corr = align(ps, ts)?;
    let cells = &mut corr.cells;

    // Assign revisions: edit cells individually; maximal runs of
    // adjacent touched anchors as one formatting revision each.
    let mut revs: Vec<Revision> = Vec::new();
    let mut fmt_run: Option<usize> = None; // index into revs
    for ci in 0..cells.len() {
        if cells[ci].cell_type == CellType::Anchor {
            if !cells[ci].touched {
                fmt_run = None;
                continue;
            }
            if fmt_run.is_none() {
                let snippet = snip(
                    ts.grexes
                        .get(cells[ci].t[0])
                        .and_then(|g| g.first())
                        .map(String::as_str),
                );
                revs.push(Revision {
                    id: format!("fmt:{path}:{ci}"),
                    index: 0,
                    kind: RevKind::Formatting,
                    category: Some("formatting"),
                    what: None,
                    path: path.to_string(),
                    new_path: None,
                    doc: Some(path.to_string()),
                    cells: Vec::new(),
                    pos: ci,
                    parent_grexes: Vec::new(),
                    tabula_grexes: Vec::new(),
                    snippet,
                });
                fmt_run = Some(revs.len() - 1);
            }
            let run = fmt_run.unwrap();
            revs[run].cells.push(ci);
            cells[ci].rev = Some(revs[run].id.clone());
            continue;
        }
        fmt_run = None;
        let (cat, what) = match cells[ci].cell_type {
            CellType::Regroup => ("structural", "regrouped"),
            CellType::Pair => ("substantial", "modified"),
            CellType::Del => ("substantial", "grex deleted"),
            _ => ("substantial", "grex inserted"),
        };
        let snippet = if !cells[ci].t.is_empty() {
            snip(
                ts.grexes
                    .get(cells[ci].t[0])
                    .and_then(|g| g.first())
                    .map(String::as_str),
            )
        } else {
            snip(
                ps.grexes
                    .get(cells[ci].p[0])
                    .and_then(|g| g.first())
                    .map(String::as_str),
            )
        };
        let rev = Revision {
            id: format!("edit:{path}:{ci}"),
            index: 0,
            kind: RevKind::Edit,
            category: Some(cat),
            what: Some(what),
            path: path.to_string(),
            new_path: None,
            doc: Some(path.to_string()),
            cells: vec![ci],
            pos: ci,
            parent_grexes: Vec::new(),
            tabula_grexes: Vec::new(),
            snippet,
        };
        cells[ci].rev = Some(rev.id.clone());
        revs.push(rev);
    }

    // Record grex ordinals for display.
    for rev in &mut revs {
        for &ci in &rev.cells {
            rev.parent_grexes.extend(cells[ci].p.iter().copied());
            rev.tabula_grexes.extend(cells[ci].t.iter().copied());
        }
    }

    Ok((corr, revs))
}

fn range_bytes<'a>(side: &'a DocState, i: usize) -> &'a [u8] {
    let (s, e) = side.ranges[i];
    &side.derivation.volumen[s..e]
}

/// Gap between anchors: Rule A (structural regroup) when both
/// sides are non-empty and the flat atom-hash lists are equal;
/// Rule B (positional pairing) otherwise. (PROVISIONAL.md #5.)
/// `p_end`/`t_end` are exclusive (the anchor positions).
fn gap_cells(
    cells: &mut Vec<Cell>,
    ps: &DocState,
    ts: &DocState,
    p0: usize,
    p_end: usize,
    t0: usize,
    t_end: usize,
) {
    let m = p_end.saturating_sub(p0);
    let n = t_end.saturating_sub(t0);

    if m >= 1 && n >= 1 {
        let pflat: Vec<&str> = (p0..p_end)
            .flat_map(|i| fasc_atoms(&ps.derivation, i))
            .collect();
        let tflat: Vec<&str> = (t0..t_end)
            .flat_map(|i| fasc_atoms(&ts.derivation, i))
            .collect();
        if pflat == tflat {
            cells.push(Cell {
                cell_type: CellType::Regroup,
                p: (p0..p_end).collect(),
                t: (t0..t_end).collect(),
                touched: false,
                rev: None,
            });
            return;
        }
    }
    let k = m.min(n);
    for i in 0..k {
        cells.push(Cell {
            cell_type: CellType::Pair,
            p: vec![p0 + i],
            t: vec![t0 + i],
            touched: false,
            rev: None,
        });
    }
    for p in (p0 + k)..p_end {
        cells.push(Cell {
            cell_type: CellType::Del,
            p: vec![p],
            t: Vec::new(),
            touched: false,
            rev: None,
        });
    }
    for t in (t0 + k)..t_end {
        cells.push(Cell {
            cell_type: CellType::Ins,
            p: Vec::new(),
            t: vec![t],
            touched: false,
            rev: None,
        });
    }
}

fn fasc_atoms(d: &Derivation, i: usize) -> Vec<&str> {
    let fh = &d.fasc_hashes[i];
    let bytes = &d.fasciculi[fh];
    std::str::from_utf8(bytes)
        .expect("fasciculus bytes are hex + LF")
        .split('\n')
        .filter(|l| !l.is_empty())
        .collect()
}
