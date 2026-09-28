//! Garbage collection (SPEC §6.10): `lit gc`.
//!
//! Mark-and-sweep over the six object types. Roots: every version
//! hash named by an inscriptio in any existing history, the live
//! tabula's trios and manifest, and the contextio ledger's atom
//! hashes. The mark closure follows storage pointers only —
//! version/tabula → trios → spina → fasciculi → atoms — and the
//! sweep removes everything unvisited. Deterministic, and
//! invisible to every reachable object and identity.

use std::collections::HashSet;

use crate::error::Result;
use crate::repository::Repository;
use crate::theca::ObjectStore;
use crate::version_object::VersionObject;

/// Per-type counts of collected (or collectable) objects, in
/// SPEC §6.7's type order.
pub struct Report {
    pub counts: Vec<(&'static str, usize)>,
}

impl Report {
    pub fn total(&self) -> usize {
        self.counts.iter().map(|(_, n)| n).sum()
    }
}

/// Runs the collection (or previews it with `dry_run`).
pub fn collect(repo: &Repository, dry_run: bool) -> Result<Report> {
    let theca = repo.theca();

    // ── Roots ───────────────────────────────────────────────────
    let mut versions: HashSet<String> = HashSet::new();
    let mut histories = vec![repo.historia()];
    for name in repo.experiments() {
        if let Some(h) = repo.experiment(&name)? {
            histories.push(h);
        }
    }
    for h in &histories {
        for e in h.entries()? {
            versions.insert(e.vhash);
        }
    }

    let mut volumina: HashSet<String> = HashSet::new();
    let mut spinae: HashSet<String> = HashSet::new();
    let mut atom_spinae: HashSet<String> = HashSet::new();
    let mut fasciculi: HashSet<String> = HashSet::new();
    let mut atoms: HashSet<String> = HashSet::new();

    let mark_trios = |vo: &VersionObject,
                          volumina: &mut HashSet<String>,
                          spinae: &mut HashSet<String>,
                          atom_spinae: &mut HashSet<String>| {
        for (v, (s, a)) in vo.trios() {
            volumina.insert(v.clone());
            spinae.insert(s.clone());
            atom_spinae.insert(a.clone());
        }
    };

    for vhash in &versions {
        if let Some(vo) = VersionObject::load(&theca, vhash)? {
            mark_trios(&vo, &mut volumina, &mut spinae, &mut atom_spinae);
        }
    }
    if let Some(tab) = repo.read_tabula()? {
        mark_trios(&tab, &mut volumina, &mut spinae, &mut atom_spinae);
    }

    // spina → fasciculi → atoms.
    for s in &spinae {
        let Some(bytes) = theca.get("spina", s)? else {
            continue;
        };
        for fh in String::from_utf8_lossy(&bytes)
            .split('\n')
            .filter(|l| !l.is_empty())
        {
            fasciculi.insert(fh.to_string());
        }
    }
    for f in &fasciculi {
        let Some(bytes) = theca.get("fasciculus", f)? else {
            continue;
        };
        for ah in String::from_utf8_lossy(&bytes)
            .split('\n')
            .filter(|l| !l.is_empty())
        {
            atoms.insert(ah.to_string());
        }
    }

    // The contextio ledger's atom hashes stay displayable.
    if let Some(ledger) = repo.read_contextio()? {
        for c in &ledger.conflicts {
            for h in c.a_atoms.iter().chain(c.b_atoms.iter()) {
                atoms.insert(h.clone());
            }
        }
    }

    // ── Sweep ───────────────────────────────────────────────────
    let marked: [(&'static str, &HashSet<String>); 6] = [
        ("atom", &atoms),
        ("fasciculus", &fasciculi),
        ("volumen", &volumina),
        ("spina", &spinae),
        ("atom-spina", &atom_spinae),
        ("version", &versions),
    ];
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for (type_, keep) in marked {
        let mut hashes = theca.list(type_)?;
        hashes.sort();
        let mut n = 0usize;
        for h in hashes {
            if keep.contains(&h) {
                continue;
            }
            if !dry_run {
                theca.delete(type_, &h)?;
            }
            n += 1;
        }
        counts.push((type_, n));
    }
    Ok(Report { counts })
}
