//! The verification walk (SPEC §10 #7).
//!
//! Verifies the repository: content-object hashes, version-object
//! pre-image hashes with structural manifest checks, the SPEC
//! §6.6 atom-spina/walk equality, the SPEC §6.5 re-derivation of
//! every version's tree under its historical modes, and
//! history-entry resolution.

use crate::derive::hash_bytes;
use crate::repository::Repository;
use crate::theca::ObjectStore;
use crate::version_object::VersionObject;

/// Problem strings; empty = sound.
pub fn check(repo: &Repository) -> Vec<String> {
    let theca = repo.theca();
    let mut problems: Vec<String> = Vec::new();

    // 1. Content objects hash to their addresses.
    for type_ in ["atom", "fasciculus", "volumen", "spina", "atom-spina"] {
        let mut hashes = theca.list(type_).unwrap_or_default();
        hashes.sort();
        for hash in hashes {
            let Ok(Some(bytes)) = theca.get(type_, &hash) else {
                problems.push(format!("{type_} {hash}: unreadable"));
                continue;
            };
            if hash_bytes(&bytes) != hash {
                problems.push(format!("{type_} {hash}: content hash mismatch"));
            }
        }
    }

    // 2–4. Version objects.
    let mut vhashes = theca.list("version").unwrap_or_default();
    vhashes.sort();
    for vhash in vhashes {
        let vo = match VersionObject::load(&theca, &vhash) {
            Ok(Some(vo)) => vo,
            Ok(None) => {
                problems.push(format!("version {vhash}: unreadable"));
                continue;
            }
            Err(e) => {
                problems.push(format!("version {vhash}: {}", e.0.trim_end()));
                continue;
            }
        };

        // Structural manifest check happens in parse/load
        // (well-formed lines, every path covered by a trio, every
        // trio at a path); here: the trios' objects must exist.
        let states = match repo.doc_states(&vo) {
            Ok(s) => s,
            Err(e) => {
                problems.push(format!("version {vhash}: {}", e.0.trim_end()));
                continue;
            }
        };
        let mut paths: Vec<&String> = states.keys().collect();
        paths.sort();
        for path in paths {
            let doc = vo.document_at(path).expect("path from states");
            let trio = vo.trio_for(doc).expect("trio for manifest doc");
            let d = &states[path]; // re-derivation (bytes + historical mode)

            // 4. Re-derivation must reproduce the stored trio.
            if d.spina_hash != trio[1] {
                problems.push(format!(
                    "version {vhash}: {path}: re-derived spina {} != stored {}",
                    d.spina_hash, trio[1]
                ));
            }
            if d.atom_spina_hash != trio[2] {
                problems.push(format!(
                    "version {vhash}: {path}: re-derived atom-spina {} != stored {}",
                    d.atom_spina_hash, trio[2]
                ));
            }

            // Every derived object must be present in the theca.
            if !theca.has("spina", &d.spina_hash).unwrap_or(false) {
                problems.push(format!("version {vhash}: {path}: spina object missing"));
            }
            if !theca.has("atom-spina", &d.atom_spina_hash).unwrap_or(false) {
                problems.push(format!(
                    "version {vhash}: {path}: atom-spina object missing"
                ));
            }
            let mut fhs: Vec<&String> = d.fasciculi.keys().collect();
            fhs.sort();
            for fh in fhs {
                if !theca.has("fasciculus", fh).unwrap_or(false) {
                    problems.push(format!("version {vhash}: {path}: fasciculus {fh} missing"));
                }
            }
            let mut ahs: Vec<&String> = d.atoms.keys().collect();
            ahs.sort();
            for ah in ahs {
                if !theca.has("atom", ah).unwrap_or(false) {
                    problems.push(format!("version {vhash}: {path}: atom {ah} missing"));
                }
            }

            // 3. atom-spina == walk(spina -> fasciculi) (SPEC §6.6).
            let walked: Option<Vec<u8>> = match theca.get("spina", trio[1]).ok().flatten() {
                Some(spina) => {
                    let text = String::from_utf8_lossy(&spina).into_owned();
                    let mut out: Vec<u8> = Vec::new();
                    let mut ok = true;
                    for fh in text.split('\n').filter(|l| !l.is_empty()) {
                        match theca.get("fasciculus", fh).ok().flatten() {
                            Some(fb) => out.extend_from_slice(&fb),
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    ok.then_some(out)
                }
                None => None,
            };
            let aspina = theca.get("atom-spina", trio[2]).ok().flatten();
            let equal = matches!((&walked, &aspina), (Some(w), Some(a)) if w == a);
            if !equal {
                problems.push(format!(
                    "version {vhash}: {path}: atom-spina != walk(spina) (SPEC §6.6)"
                ));
            }
        }
    }

    // 5. Histories resolve.
    let mut histories = vec![repo.historia()];
    for name in repo.experiments() {
        if let Ok(Some(h)) = repo.experiment(&name) {
            histories.push(h);
        }
    }
    for history in histories {
        let entries = match history.entries() {
            Ok(e) => e,
            Err(e) => {
                problems.push(format!("{}: {}", history.name(), e.0.trim_end()));
                continue;
            }
        };
        for e in entries {
            if !theca.has("version", &e.vhash).unwrap_or(false) {
                problems.push(format!(
                    "{}: inscriptio names missing version {}",
                    history.name(),
                    e.vhash
                ));
            }
        }
    }

    problems
}
