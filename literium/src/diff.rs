//! Diff presentation (SPEC §7.4).
//!
//! Git's unified diff, adapted with atoms in place of lines: one
//! coordinate language (`gN`, `gNaM`), whole-grex hunks, and
//! exact hash-leveraged annotations — moves by document-wide hash
//! matching, atom splits/merges by content-form join equality.
//! Output is plain deterministic bytes; the format is part of the
//! conformance surface.

use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use syndesmos::Syndesmos;

use crate::bonds;
use crate::classify::classify;
use crate::derive::Derivation;
use crate::error::Result;
use crate::lcs;
pub use crate::lcs::Op;
use crate::revision_detect::{align, CellType, DocCorrespondence, SideInput};

/// LCS over atom content forms, leftmost trace (compat shim over
/// the shared [`lcs`] machinery).
pub fn atom_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(Op, &'a str)> {
    lcs::ops(a, b)
        .into_iter()
        .map(|(op, idx)| match op {
            Op::Ins => (op, b[idx]),
            _ => (op, a[idx]),
        })
        .collect()
}

fn trio(state: &Derivation) -> [&str; 3] {
    [
        &state.volumen_hash,
        &state.spina_hash,
        &state.atom_spina_hash,
    ]
}

/// One rendered atom line of a hunk, pre-annotation.
struct AtomLine {
    op: Op,
    coord: String,
    text: String,
    hash: String,
    ann: Option<String>,
}

/// A block of substantial-document output: a regroup line or a
/// hunk of atom lines.
enum Block {
    Regroup(String),
    Hunk {
        old_gs: Vec<usize>, // 1-based ordinals
        new_gs: Vec<usize>,
        lines: Vec<AtomLine>,
    },
}

/// Collapses sorted 1-based ordinals into "g2, g5-g7".
fn ranges(ordinals: &[usize]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < ordinals.len() {
        let start = ordinals[i];
        let mut end = start;
        while i + 1 < ordinals.len() && ordinals[i + 1] == end + 1 {
            i += 1;
            end = ordinals[i];
        }
        parts.push(if start == end {
            format!("g{start}")
        } else {
            format!("g{start}-g{end}")
        });
        i += 1;
    }
    parts.join(", ")
}

fn hunk_side(gs: &[usize]) -> String {
    match (gs.first(), gs.last()) {
        (None, _) => ".".to_string(),
        (Some(a), Some(b)) if a == b => format!("g{a}"),
        (Some(a), Some(b)) => format!("g{a}..g{b}"),
        _ => unreachable!(),
    }
}

fn regroup_line(cell_p: &[usize], cell_t: &[usize]) -> String {
    let old = cell_p
        .iter()
        .map(|i| format!("g{}", i + 1))
        .collect::<Vec<_>>()
        .join("+");
    let new = cell_t
        .iter()
        .map(|j| format!("g{}", j + 1))
        .collect::<Vec<_>>()
        .join(",");
    let word = if cell_p.len() == 1 {
        "split"
    } else if cell_t.len() == 1 {
        "merged"
    } else {
        "regrouped"
    };
    format!("~ {old} {word} -> {new}")
}

/// Touched-anchor new-side ordinals (1-based, in order).
fn touched_ordinals(corr: &DocCorrespondence) -> Vec<usize> {
    corr.cells
        .iter()
        .filter(|c| c.cell_type == CellType::Anchor && c.touched)
        .map(|c| c.t[0] + 1)
        .collect()
}

fn atoms_of<'d>(d: &'d Derivation, grex: usize) -> Vec<&'d str> {
    d.grexes[grex].iter().map(String::as_str).collect()
}

/// One display unit of a grex: an atom, or — under bonds (SPEC §7.4
/// *Bonds*) — a molecule of consecutive atoms shown as one line. Its
/// hash is that of its text, so a one-atom unit's hash is the atom's
/// and the move/split annotations work unchanged over units.
struct Unit {
    text: String,
    hash: String,
    first: usize,
    last: usize,
}

/// The units of a grex: its atoms, or its molecules when `bonds`
/// apply (prose only — poetry atoms are lines, and a `.desm` speaks
/// of sentences).
fn units_of(d: &Derivation, grex: usize, bonds: Option<&Syndesmos>) -> Vec<Unit> {
    let atoms = atoms_of(d, grex);
    let bonds = if d.mode == "prose" { bonds } else { None };
    bonds::molecules(&atoms, bonds)
        .into_iter()
        .map(|(first, last)| {
            let text = bonds::molecule_text(&atoms, first, last);
            let hash = crate::derive::hash_bytes(text.as_bytes());
            Unit { text, hash, first, last }
        })
        .collect()
}

/// Builds the substantial-document blocks from the alignment. With
/// `bonds`, the lines are molecules and their coordinates ranges;
/// without, one atom per line (the `--unified` conformance form).
fn build_blocks(corr: &DocCorrespondence, bonds: Option<&Syndesmos>) -> Vec<Block> {
    let pd = &corr.parent.derivation;
    let td = &corr.tabula.derivation;
    let mut blocks: Vec<Block> = Vec::new();
    let mut hunk: Option<(Vec<usize>, Vec<usize>, Vec<AtomLine>)> = None;

    let flush = |hunk: &mut Option<(Vec<usize>, Vec<usize>, Vec<AtomLine>)>,
                 blocks: &mut Vec<Block>| {
        if let Some((old_gs, new_gs, lines)) = hunk.take() {
            blocks.push(Block::Hunk {
                old_gs,
                new_gs,
                lines,
            });
        }
    };

    for cell in &corr.cells {
        match cell.cell_type {
            CellType::Anchor => flush(&mut hunk, &mut blocks),
            CellType::Regroup => {
                flush(&mut hunk, &mut blocks);
                blocks.push(Block::Regroup(regroup_line(&cell.p, &cell.t)));
            }
            CellType::Pair | CellType::Del | CellType::Ins => {
                let (old_gs, new_gs, lines) =
                    hunk.get_or_insert((Vec::new(), Vec::new(), Vec::new()));
                for &i in &cell.p {
                    if !old_gs.contains(&(i + 1)) {
                        old_gs.push(i + 1);
                    }
                }
                for &j in &cell.t {
                    if !new_gs.contains(&(j + 1)) {
                        new_gs.push(j + 1);
                    }
                }
                match cell.cell_type {
                    CellType::Pair => {
                        let (pg, tg) = (cell.p[0], cell.t[0]);
                        let pu = units_of(pd, pg, bonds);
                        let tu = units_of(td, tg, bonds);
                        let pt: Vec<&str> = pu.iter().map(|u| u.text.as_str()).collect();
                        let tt: Vec<&str> = tu.iter().map(|u| u.text.as_str()).collect();
                        // Context lines show the new side's
                        // coordinate (SPEC §7.4); nj tracks the
                        // new-side unit index through the ops.
                        let mut nj = 0usize;
                        for (op, idx) in lcs::ops(&pt, &tt) {
                            match op {
                                Op::Equal => {
                                    let t = &tu[nj];
                                    lines.push(AtomLine {
                                        op,
                                        coord: bonds::coord_range(tg, t.first, t.last),
                                        text: pu[idx].text.clone(),
                                        hash: pu[idx].hash.clone(),
                                        ann: None,
                                    });
                                    nj += 1;
                                }
                                Op::Del => {
                                    let u = &pu[idx];
                                    lines.push(AtomLine {
                                        op,
                                        coord: bonds::coord_range(pg, u.first, u.last),
                                        text: u.text.clone(),
                                        hash: u.hash.clone(),
                                        ann: None,
                                    });
                                }
                                Op::Ins => {
                                    let u = &tu[idx];
                                    lines.push(AtomLine {
                                        op,
                                        coord: bonds::coord_range(tg, u.first, u.last),
                                        text: u.text.clone(),
                                        hash: u.hash.clone(),
                                        ann: None,
                                    });
                                    nj += 1;
                                }
                            }
                        }
                    }
                    CellType::Del => {
                        let pg = cell.p[0];
                        for u in units_of(pd, pg, bonds) {
                            lines.push(AtomLine {
                                op: Op::Del,
                                coord: bonds::coord_range(pg, u.first, u.last),
                                text: u.text,
                                hash: u.hash,
                                ann: None,
                            });
                        }
                    }
                    CellType::Ins => {
                        let tg = cell.t[0];
                        for u in units_of(td, tg, bonds) {
                            lines.push(AtomLine {
                                op: Op::Ins,
                                coord: bonds::coord_range(tg, u.first, u.last),
                                text: u.text,
                                hash: u.hash,
                                ann: None,
                            });
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    flush(&mut hunk, &mut blocks);
    blocks
}

/// Document-wide exact move matching (SPEC §7.4): first unmatched
/// deletion to the first identical-hash unmatched insertion.
fn annotate_moves(blocks: &mut [Block]) {
    let mut dels: Vec<(usize, usize)> = Vec::new(); // (block, line)
    let mut inses: Vec<(usize, usize)> = Vec::new();
    for (bi, b) in blocks.iter().enumerate() {
        if let Block::Hunk { lines, .. } = b {
            for (li, l) in lines.iter().enumerate() {
                match l.op {
                    Op::Del => dels.push((bi, li)),
                    Op::Ins => inses.push((bi, li)),
                    Op::Equal => {}
                }
            }
        }
    }
    let mut used: HashSet<usize> = HashSet::new();
    let mut pairs: Vec<((usize, usize), (usize, usize))> = Vec::new();
    for &(dbi, dli) in &dels {
        let dhash = match &blocks[dbi] {
            Block::Hunk { lines, .. } => lines[dli].hash.clone(),
            _ => continue,
        };
        for (k, &(ibi, ili)) in inses.iter().enumerate() {
            if used.contains(&k) {
                continue;
            }
            let ihash = match &blocks[ibi] {
                Block::Hunk { lines, .. } => &lines[ili].hash,
                _ => continue,
            };
            if *ihash == dhash {
                used.insert(k);
                pairs.push(((dbi, dli), (ibi, ili)));
                break;
            }
        }
    }
    for ((dbi, dli), (ibi, ili)) in pairs {
        let icoord = match &blocks[ibi] {
            Block::Hunk { lines, .. } => lines[ili].coord.clone(),
            _ => continue,
        };
        let dcoord = match &blocks[dbi] {
            Block::Hunk { lines, .. } => lines[dli].coord.clone(),
            _ => continue,
        };
        if let Block::Hunk { lines, .. } = &mut blocks[dbi] {
            lines[dli].ann = Some(format!("[-> {icoord}]"));
        }
        if let Block::Hunk { lines, .. } = &mut blocks[ibi] {
            lines[ili].ann = Some(format!("[<- {dcoord}]"));
        }
    }
}

/// Exact split/merge detection per replace run (SPEC §7.4): a
/// maximal deletions-then-insertions block; move-annotated atoms
/// are skipped.
fn annotate_splits(blocks: &mut [Block], joiner: &str) {
    for b in blocks.iter_mut() {
        let Block::Hunk { lines, .. } = b else {
            continue;
        };
        // Find maximal D+I+ runs.
        let n = lines.len();
        let mut i = 0;
        while i < n {
            if lines[i].op != Op::Del {
                i += 1;
                continue;
            }
            let d0 = i;
            while i < n && lines[i].op == Op::Del {
                i += 1;
            }
            let d1 = i; // exclusive
            if i >= n || lines[i].op != Op::Ins {
                continue;
            }
            let i0 = i;
            while i < n && lines[i].op == Op::Ins {
                i += 1;
            }
            let i1 = i; // exclusive

            // Scan the run.
            let mut c = i0; // insertion cursor
            let mut d = d0;
            while d < d1 {
                if lines[d].ann.is_some() {
                    d += 1;
                    continue;
                }
                // Split: content(d) == join of k >= 2 insertions.
                let mut matched = false;
                for k in 2..=(i1 - c) {
                    if lines[c..c + k].iter().any(|l| l.ann.is_some()) {
                        break;
                    }
                    let joined = lines[c..c + k]
                        .iter()
                        .map(|l| l.text.as_str())
                        .collect::<Vec<_>>()
                        .join(joiner);
                    if joined.len() > lines[d].text.len() {
                        break;
                    }
                    if joined == lines[d].text {
                        let coords = lines[c..c + k]
                            .iter()
                            .map(|l| l.coord.clone())
                            .collect::<Vec<_>>()
                            .join(",");
                        let dcoord = lines[d].coord.clone();
                        lines[d].ann = Some(format!("[split into {coords}]"));
                        for l in &mut lines[c..c + k] {
                            l.ann = Some(format!("[from {dcoord}]"));
                        }
                        c += k;
                        matched = true;
                        break;
                    }
                }
                if matched {
                    d += 1;
                    continue;
                }
                // Merge: join of k >= 2 deletions == content(c).
                if c < i1 && lines[c].ann.is_none() {
                    let mut found_k = None;
                    for k in 2..=(d1 - d) {
                        if lines[d..d + k].iter().any(|l| l.ann.is_some()) {
                            break;
                        }
                        let joined = lines[d..d + k]
                            .iter()
                            .map(|l| l.text.as_str())
                            .collect::<Vec<_>>()
                            .join(joiner);
                        if joined.len() > lines[c].text.len() {
                            break;
                        }
                        if joined == lines[c].text {
                            found_k = Some(k);
                            break;
                        }
                    }
                    if let Some(k) = found_k {
                        let icoord = lines[c].coord.clone();
                        let coords = lines[d..d + k]
                            .iter()
                            .map(|l| l.coord.clone())
                            .collect::<Vec<_>>()
                            .join(",");
                        for l in &mut lines[d..d + k] {
                            l.ann = Some(format!("[into {icoord}]"));
                        }
                        lines[c].ann = Some(format!("[merged from {coords}]"));
                        d += k;
                        c += 1;
                        continue;
                    }
                }
                d += 1;
            }
        }
    }
}

fn render_blocks(out: &mut Vec<String>, blocks: &[Block]) {
    for b in blocks {
        match b {
            Block::Regroup(line) => out.push(line.clone()),
            Block::Hunk {
                old_gs,
                new_gs,
                lines,
            } => {
                out.push(format!(
                    "@@ -{} +{} @@",
                    hunk_side(old_gs),
                    hunk_side(new_gs)
                ));
                for l in lines {
                    let sigil = match l.op {
                        Op::Equal => "  ",
                        Op::Del => "- ",
                        Op::Ins => "+ ",
                    };
                    let ann = match &l.ann {
                        Some(a) => format!(" {a}"),
                        None => String::new(),
                    };
                    out.push(format!("{sigil}{}  {}{ann}", l.coord, l.text));
                }
            }
        }
    }
}

/// Renders the difference between two whole-tree states per SPEC
/// §7.4.
pub fn render(a: &SideInput, b: &SideInput) -> Result<String> {
    let mut out: Vec<String> = Vec::new();

    let mut done_a: HashSet<&str> = HashSet::new();
    let mut done_b: HashSet<&str> = HashSet::new();

    for path in a.manifest.keys() {
        if !b.manifest.contains_key(path) {
            continue;
        }
        done_a.insert(path);
        done_b.insert(path);
        let sa = &a.states[path.as_str()];
        let sb = &b.states[path.as_str()];
        let cat = classify(&trio(sa), &trio(sb))?;
        match cat {
            "no-change" => {}
            "re-segmentation" => out.push(format!("{path}: re-segmentation")),
            "formatting" => {
                let corr = align(sa, sb)?;
                out.push(format!(
                    "{path}: formatting ({})",
                    ranges(&touched_ordinals(&corr))
                ));
            }
            "structural" => {
                let corr = align(sa, sb)?;
                out.push(format!("{path}: structural"));
                for cell in &corr.cells {
                    if cell.cell_type == CellType::Regroup {
                        out.push(regroup_line(&cell.p, &cell.t));
                    }
                }
                let touched = touched_ordinals(&corr);
                if !touched.is_empty() {
                    out.push(format!("formatting: {}", ranges(&touched)));
                }
            }
            _ => {
                // substantial
                let corr = align(sa, sb)?;
                out.push(format!("{path}: substantial"));
                // Atoms, never molecules: the unified grammar is the
                // conformance surface (SPEC §7.4 *Bonds*).
                let mut blocks = build_blocks(&corr, None);
                annotate_moves(&mut blocks);
                // SPEC §7.4: the detection join is a single space
                // in every mode — the words-preserved test.
                annotate_splits(&mut blocks, " ");
                render_blocks(&mut out, &blocks);
                let touched = touched_ordinals(&corr);
                if !touched.is_empty() {
                    out.push(format!("formatting: {}", ranges(&touched)));
                }
            }
        }
    }

    // Renames among the leftovers (same trio, different path).
    let mut a_by_trio: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for p in a.manifest.keys() {
        if !done_a.contains(p.as_str()) {
            a_by_trio
                .entry(trio(&a.states[p.as_str()]).join("|"))
                .or_default()
                .push(p);
        }
    }
    let mut b_by_trio: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for p in b.manifest.keys() {
        if !done_b.contains(p.as_str()) {
            b_by_trio
                .entry(trio(&b.states[p.as_str()]).join("|"))
                .or_default()
                .push(p);
        }
    }
    let mut renamed_a: HashSet<&str> = HashSet::new();
    let mut renamed_b: HashSet<&str> = HashSet::new();
    for (key, old_paths) in &a_by_trio {
        let Some(new_paths) = b_by_trio.get(key) else {
            continue;
        };
        for (o, n) in old_paths.iter().zip(new_paths.iter()) {
            out.push(format!("{o} -> {n}: renamed"));
            renamed_a.insert(o);
            renamed_b.insert(n);
        }
    }

    for p in a.manifest.keys() {
        if !done_a.contains(p.as_str()) && !renamed_a.contains(p.as_str()) {
            out.push(format!("{p}: removed"));
        }
    }
    for p in b.manifest.keys() {
        if !done_b.contains(p.as_str()) && !renamed_b.contains(p.as_str()) {
            out.push(format!("{p}: added"));
        }
    }

    if out.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{}\n", out.join("\n")))
    }
}

// ── structured collation (PROTOCOL §0.1) + apparatus rendering ───
//
// The same alignment `render` uses, surfaced as typed `Change`s in
// the copyedit categories, and rendered as a critical-apparatus
// change list rather than git's `-`/`+`. Shared by `lit diff`
// (terminal) and the `diff` protocol method.

/// A change kind — the copyedit register (PROTOCOL §0.1).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Move,
    Insertion,
    Deletion,
    Substitution,
    Split,
    Join,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Move => "move",
            Kind::Insertion => "insertion",
            Kind::Deletion => "deletion",
            Kind::Substitution => "substitution",
            Kind::Split => "split",
            Kind::Join => "join",
        }
    }
}

/// One change in the collation.
pub struct Change {
    pub kind: Kind,
    /// Primary coordinate: the new-side location, or the old side
    /// for a deletion/move origin.
    pub coord: String,
    /// Move destination, or the split/join target ordinals.
    pub to: Option<String>,
    /// The old reading (deletion, substitution, move).
    pub old: Option<String>,
    /// The new reading (insertion, substitution).
    pub new: Option<String>,
}

/// A document's collation: its edit category (§7.2) and the
/// ordered changes within it.
pub struct DocumentDiff {
    pub path: String,
    pub category: String,
    pub changes: Vec<Change>,
}

/// Structured move detection: first unmatched deletion to the
/// first identical-hash unmatched insertion (SPEC §7.4). Returns
/// (del (bi,li) -> dest coord) and the set of move-insertion
/// (bi,li).
fn move_pairs(
    blocks: &[Block],
) -> (BTreeMap<(usize, usize), String>, HashSet<(usize, usize)>) {
    let mut dels: Vec<(usize, usize)> = Vec::new();
    let mut inses: Vec<(usize, usize)> = Vec::new();
    for (bi, b) in blocks.iter().enumerate() {
        if let Block::Hunk { lines, .. } = b {
            for (li, l) in lines.iter().enumerate() {
                match l.op {
                    Op::Del => dels.push((bi, li)),
                    Op::Ins => inses.push((bi, li)),
                    Op::Equal => {}
                }
            }
        }
    }
    let line = |bi: usize, li: usize| -> &AtomLine {
        match &blocks[bi] {
            Block::Hunk { lines, .. } => &lines[li],
            _ => unreachable!(),
        }
    };
    let mut del_to: BTreeMap<(usize, usize), String> = BTreeMap::new();
    let mut ins_moved: HashSet<(usize, usize)> = HashSet::new();
    let mut used: HashSet<usize> = HashSet::new();
    for &(dbi, dli) in &dels {
        let dhash = &line(dbi, dli).hash;
        for (k, &(ibi, ili)) in inses.iter().enumerate() {
            if used.contains(&k) {
                continue;
            }
            if &line(ibi, ili).hash == dhash {
                used.insert(k);
                del_to.insert((dbi, dli), line(ibi, ili).coord.clone());
                ins_moved.insert((ibi, ili));
                break;
            }
        }
    }
    (del_to, ins_moved)
}

/// Turn a document's aligned blocks into typed changes; over
/// molecules when `bonds` apply (SPEC §7.4 *Bonds*).
fn document_changes(corr: &DocCorrespondence, bonds: Option<&Syndesmos>) -> Vec<Change> {
    let blocks = build_blocks(corr, bonds);
    let (del_to, ins_moved) = move_pairs(&blocks);
    let mut changes: Vec<Change> = Vec::new();

    // A pending run of plain (non-move) deletions/insertions,
    // paired into substitutions with deletion/insertion tails.
    let mut run_del: Vec<(String, String)> = Vec::new(); // (coord, text)
    let mut run_ins: Vec<(String, String)> = Vec::new();
    let flush_run =
        |run_del: &mut Vec<(String, String)>, run_ins: &mut Vec<(String, String)>, out: &mut Vec<Change>| {
            let paired = run_del.len().min(run_ins.len());
            for i in 0..paired {
                out.push(Change {
                    kind: Kind::Substitution,
                    coord: run_ins[i].0.clone(),
                    to: None,
                    old: Some(run_del[i].1.clone()),
                    new: Some(run_ins[i].1.clone()),
                });
            }
            for d in run_del.drain(paired..) {
                out.push(Change { kind: Kind::Deletion, coord: d.0, to: None, old: Some(d.1), new: None });
            }
            for ins in run_ins.drain(paired..) {
                out.push(Change { kind: Kind::Insertion, coord: ins.0, to: None, old: None, new: Some(ins.1) });
            }
            run_del.clear();
            run_ins.clear();
        };

    for (bi, b) in blocks.iter().enumerate() {
        match b {
            Block::Regroup(line) => {
                flush_run(&mut run_del, &mut run_ins, &mut changes);
                // "~ <old> <word> -> <new>"
                let word = line.split_whitespace().nth(2).unwrap_or("regrouped");
                let (kind, coord, to) = parse_regroup(line, word);
                changes.push(Change { kind, coord, to, old: None, new: None });
            }
            Block::Hunk { lines, .. } => {
                for (li, l) in lines.iter().enumerate() {
                    match l.op {
                        Op::Equal => flush_run(&mut run_del, &mut run_ins, &mut changes),
                        Op::Del => {
                            if let Some(dest) = del_to.get(&(bi, li)) {
                                flush_run(&mut run_del, &mut run_ins, &mut changes);
                                changes.push(Change {
                                    kind: Kind::Move,
                                    coord: l.coord.clone(),
                                    to: Some(dest.clone()),
                                    old: Some(l.text.clone()),
                                    new: None,
                                });
                            } else {
                                run_del.push((l.coord.clone(), l.text.clone()));
                            }
                        }
                        Op::Ins => {
                            if ins_moved.contains(&(bi, li)) {
                                // counterpart of a move, already emitted
                            } else {
                                run_ins.push((l.coord.clone(), l.text.clone()));
                            }
                        }
                    }
                }
                flush_run(&mut run_del, &mut run_ins, &mut changes);
            }
        }
    }
    flush_run(&mut run_del, &mut run_ins, &mut changes);
    changes
}

fn parse_regroup(line: &str, word: &str) -> (Kind, String, Option<String>) {
    // "~ g2 split -> g2,g3"
    let rest = line.trim_start_matches("~ ");
    let old = rest.split_whitespace().next().unwrap_or("").to_string();
    let new = rest.rsplit("-> ").next().unwrap_or("").trim().to_string();
    let kind = match word {
        "merged" => Kind::Join,
        _ => Kind::Split, // split, regrouped
    };
    (kind, old, Some(new))
}

/// The structured collation of two whole-tree states, atom by atom.
pub fn collate(a: &SideInput, b: &SideInput) -> Result<Vec<DocumentDiff>> {
    collate_with(a, b, &mut |_| Ok(None))
}

/// The structured collation with display bonds (SPEC §7.4 *Bonds*):
/// `bonds` names the bonds for each document path (`None` for
/// atoms); where they apply, the changes are over molecules and
/// their coordinates are ranges.
pub fn collate_with(
    a: &SideInput,
    b: &SideInput,
    bonds: &mut dyn FnMut(&str) -> Result<Option<Rc<Syndesmos>>>,
) -> Result<Vec<DocumentDiff>> {
    let mut out: Vec<DocumentDiff> = Vec::new();
    let mut done_a: HashSet<&str> = HashSet::new();
    let mut done_b: HashSet<&str> = HashSet::new();

    for path in a.manifest.keys() {
        if !b.manifest.contains_key(path) {
            continue;
        }
        done_a.insert(path);
        done_b.insert(path);
        let sa = &a.states[path.as_str()];
        let sb = &b.states[path.as_str()];
        let cat = classify(&trio(sa), &trio(sb))?;
        let changes = match cat {
            "no-change" => continue,
            "re-segmentation" | "formatting" => Vec::new(),
            _ => {
                // structural, substantial
                let b = bonds(path)?;
                document_changes(&align(sa, sb)?, b.as_deref())
            }
        };
        out.push(DocumentDiff { path: path.clone(), category: cat.to_string(), changes });
    }

    // Document-set changes: renames, removals, additions.
    let mut a_by_trio: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for p in a.manifest.keys() {
        if !done_a.contains(p.as_str()) {
            a_by_trio.entry(trio(&a.states[p.as_str()]).join("|")).or_default().push(p);
        }
    }
    let mut b_by_trio: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for p in b.manifest.keys() {
        if !done_b.contains(p.as_str()) {
            b_by_trio.entry(trio(&b.states[p.as_str()]).join("|")).or_default().push(p);
        }
    }
    let mut renamed_a: HashSet<&str> = HashSet::new();
    let mut renamed_b: HashSet<&str> = HashSet::new();
    for (key, old_paths) in &a_by_trio {
        if let Some(new_paths) = b_by_trio.get(key) {
            for (o, n) in old_paths.iter().zip(new_paths.iter()) {
                out.push(DocumentDiff {
                    path: format!("{o} -> {n}"),
                    category: "renamed".to_string(),
                    changes: Vec::new(),
                });
                renamed_a.insert(o);
                renamed_b.insert(n);
            }
        }
    }
    for p in a.manifest.keys() {
        if !done_a.contains(p.as_str()) && !renamed_a.contains(p.as_str()) {
            out.push(DocumentDiff { path: p.clone(), category: "removed".to_string(), changes: Vec::new() });
        }
    }
    for p in b.manifest.keys() {
        if !done_b.contains(p.as_str()) && !renamed_b.contains(p.as_str()) {
            out.push(DocumentDiff { path: p.clone(), category: "added".to_string(), changes: Vec::new() });
        }
    }
    Ok(out)
}

/// The apparatus rendering (PROTOCOL §0.1, HISTORY §17): a clean,
/// classified change list — the editor's diff, not `-`/`+`.
pub fn render_apparatus(diffs: &[DocumentDiff]) -> String {
    let mut out = String::new();
    for (i, d) in diffs.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match d.category.as_str() {
            "added" | "removed" | "renamed" | "re-segmentation" | "formatting" => {
                out.push_str(&format!("{}  ({})\n", d.path, d.category));
                continue;
            }
            _ => out.push_str(&format!("{}\n", d.path)),
        }
        for c in &d.changes {
            match c.kind {
                Kind::Substitution => {
                    out.push_str(&format!("  {}  substitution\n", c.coord));
                    out.push_str(&format!("      - {}\n", c.old.as_deref().unwrap_or("")));
                    out.push_str(&format!("      + {}\n", c.new.as_deref().unwrap_or("")));
                }
                Kind::Insertion => {
                    out.push_str(&format!("  {}  insertion\n      + {}\n", c.coord, c.new.as_deref().unwrap_or("")));
                }
                Kind::Deletion => {
                    out.push_str(&format!("  {}  deletion\n      - {}\n", c.coord, c.old.as_deref().unwrap_or("")));
                }
                Kind::Move => {
                    out.push_str(&format!(
                        "  {} -> {}  move\n      {}\n",
                        c.coord,
                        c.to.as_deref().unwrap_or("?"),
                        c.old.as_deref().unwrap_or("")
                    ));
                }
                Kind::Split => {
                    out.push_str(&format!("  {} -> {}  split\n", c.coord, c.to.as_deref().unwrap_or("?")));
                }
                Kind::Join => {
                    out.push_str(&format!("  {} -> {}  join\n", c.coord, c.to.as_deref().unwrap_or("?")));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod collate_tests {
    use super::*;
    use crate::derive::derive;
    use crate::revision_detect::SideInput;
    use std::collections::{BTreeMap, HashMap};

    fn side(path: &str, text: &str) -> SideInput {
        let d = derive(text.as_bytes(), "prose").unwrap();
        let vh = d.volumen_hash.clone();
        let mut manifest = BTreeMap::new();
        manifest.insert(path.to_string(), vh.clone());
        let mut states = HashMap::new();
        states.insert(path.to_string(), d);
        SideInput { manifest, states }
    }

    fn kinds(diffs: &[DocumentDiff]) -> Vec<&'static str> {
        diffs.iter().flat_map(|d| d.changes.iter().map(|c| c.kind.name())).collect()
    }

    #[test]
    fn substitution_insertion_deletion() {
        let a = side("d.md", "One. Two. Three.\n");
        let b = side("d.md", "One. Two, revised. Four.\n");
        let diffs = collate(&a, &b).unwrap();
        let ks = kinds(&diffs);
        // Two -> Two,revised is a substitution; Three -> Four ...
        assert!(ks.contains(&"substitution"), "kinds: {ks:?}");
    }

    #[test]
    fn move_detected_not_delete_add() {
        // Reorder two sentences within a paragraph: a pure move.
        let a = side("d.md", "Alpha here. Beta there. Gamma last.\n");
        let b = side("d.md", "Gamma last. Alpha here. Beta there.\n");
        let diffs = collate(&a, &b).unwrap();
        let ks = kinds(&diffs);
        assert!(ks.contains(&"move"), "a move must be named a move, not delete+add: {ks:?}");
    }

    #[test]
    fn no_change_empty() {
        let a = side("d.md", "Same. Text.\n");
        let b = side("d.md", "Same. Text.\n");
        assert!(collate(&a, &b).unwrap().is_empty());
    }
}
