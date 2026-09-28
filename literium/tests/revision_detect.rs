// Port of literium-perl t/11-revision-detect.t — SPEC §8.5, §10 #5.

use std::collections::{BTreeMap, HashMap};

use literium::derive::derive;
use literium::revision_detect::{detect, Pending, RevKind, SideInput};

fn side(files: &[(&str, &str, &str)]) -> SideInput {
    let mut manifest = BTreeMap::new();
    let mut states = HashMap::new();
    for (path, bytes, mode) in files {
        let d = derive(bytes.as_bytes(), mode).unwrap_or_else(|| panic!("not a document: {path}"));
        manifest.insert(path.to_string(), d.volumen_hash.clone());
        states.insert(path.to_string(), d);
    }
    SideInput { manifest, states }
}

fn kinds(p: &Pending) -> Vec<RevKind> {
    p.revisions.iter().map(|r| r.kind).collect()
}

fn cats(p: &Pending) -> Vec<&str> {
    p.revisions
        .iter()
        .map(|r| r.category.unwrap_or("-"))
        .collect()
}

fn whats(p: &Pending) -> Vec<&str> {
    p.revisions.iter().map(|r| r.what.unwrap_or("-")).collect()
}

#[test]
fn no_change() {
    let p = detect(
        &side(&[("a.md", "One. Two.\n", "prose")]),
        &side(&[("a.md", "One. Two.\n", "prose")]),
    )
    .unwrap();
    assert!(p.revisions.is_empty(), "identical states: no revisions");
}

#[test]
fn single_modify() {
    let p = detect(
        &side(&[("a.md", "One. Two.\n\nThree.\n", "prose")]),
        &side(&[("a.md", "One. Two changed.\n\nThree.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(kinds(&p), vec![RevKind::Edit], "one edit revision");
    assert_eq!(
        p.revisions[0].category,
        Some("substantial"),
        "modify is substantial"
    );
    assert_eq!(p.revisions[0].parent_grexes, vec![0], "parent grex 0");
    assert_eq!(p.revisions[0].tabula_grexes, vec![0], "tabula grex 0");
}

#[test]
fn insert_and_delete_grexes() {
    let p = detect(
        &side(&[("a.md", "One.\n\nThree.\n", "prose")]),
        &side(&[("a.md", "One.\n\nTwo.\n\nThree.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(kinds(&p), vec![RevKind::Edit], "insertion is one revision");
    assert_eq!(
        p.revisions[0].what,
        Some("grex inserted"),
        "insert detected"
    );

    let p = detect(
        &side(&[("a.md", "One.\n\nTwo.\n\nThree.\n", "prose")]),
        &side(&[("a.md", "One.\n\nThree.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(
        p.revisions[0].what,
        Some("grex deleted"),
        "deletion detected"
    );

    // Wholesale rewrite: max(M,N) revisions by positional pairing.
    let p = detect(
        &side(&[("a.md", "Aaa.\n\nBbb.\n", "prose")]),
        &side(&[("a.md", "Xxx.\n\nYyy.\n\nZzz.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(
        whats(&p),
        vec!["modified", "modified", "grex inserted"],
        "Rule B positional pairing with leftover insert"
    );
}

#[test]
fn rule_a_structural_regroups() {
    // Split: one paragraph becomes two, same flat atom sequence.
    let p = detect(
        &side(&[("a.md", "One. Two. Three.\n", "prose")]),
        &side(&[("a.md", "One. Two.\n\nThree.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(cats(&p), vec!["structural"], "pure split is one structural");

    // Merge: two stanzas into one.
    let p = detect(
        &side(&[("a.md", "One. Two.\n\nThree.\n", "prose")]),
        &side(&[("a.md", "One. Two. Three.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(cats(&p), vec!["structural"], "pure merge is one structural");

    // M->N regroup (3 paragraphs -> 2) with equal flat atoms.
    let p = detect(
        &side(&[("a.md", "One.\n\nTwo. Three.\n\nFour.\n", "prose")]),
        &side(&[("a.md", "One. Two.\n\nThree. Four.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(
        cats(&p),
        vec!["structural"],
        "3->2 regroup is one structural"
    );

    // Split WITH an edit: Rule A fails, Rule B decomposes
    // (accepted coarseness, PROVISIONAL.md #5).
    let p = detect(
        &side(&[("a.md", "One. Two. Three.\n", "prose")]),
        &side(&[("a.md", "One. Two CHANGED.\n\nThree.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(p.revisions.len(), 2, "split+edit decomposes into two");
    assert_eq!(
        cats(&p),
        vec!["substantial", "substantial"],
        "both substantial"
    );
}

#[test]
fn formatting_revisions() {
    // Same content, different bytes: one revision per touched run.
    let p = detect(
        &side(&[("a.md", "One. Two.\n\nThree.\n", "prose")]),
        &side(&[("a.md", "One.  Two.\n\nThree.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(cats(&p), vec!["formatting"], "double space is formatting");

    // Whole-document rewrap: one contiguous run -> one revision.
    let p = detect(
        &side(&[(
            "a.md",
            "One two three four. Five six.\n\nSeven eight.\n",
            "prose",
        )]),
        &side(&[(
            "a.md",
            "One two three\nfour. Five six.\n\nSeven\neight.\n",
            "prose",
        )]),
    )
    .unwrap();
    assert_eq!(cats(&p), vec!["formatting"], "whole-document rewrap is one");

    // Two separated touched runs -> two formatting revisions.
    let p = detect(
        &side(&[("a.md", "Aa bb.\n\nCc dd.\n\nEe ff.\n", "prose")]),
        &side(&[("a.md", "Aa  bb.\n\nCc dd.\n\nEe  ff.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(
        cats(&p),
        vec!["formatting", "formatting"],
        "untouched anchor splits formatting runs"
    );

    // Separator-only change belongs to the preceding grex's range.
    let p = detect(
        &side(&[("a.md", "One.\n\nTwo.\n", "prose")]),
        &side(&[("a.md", "One.\n\n\nTwo.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(cats(&p), vec!["formatting"], "separator-only is formatting");
    assert_eq!(
        p.revisions[0].parent_grexes,
        vec![0],
        "separator delta assigned to preceding grex"
    );

    // Mixed: atom edit + formatting in one grex = one substantial.
    let p = detect(
        &side(&[("a.md", "One. Two.\n", "prose")]),
        &side(&[("a.md", "One.  Two CHANGED.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(
        cats(&p),
        vec!["substantial"],
        "mixed edit absorbs formatting"
    );
}

#[test]
fn duplicate_fasciculus_hashes_refrain() {
    let p = detect(
        &side(&[(
            "a.md",
            "Refrain here.\n\nVerse one.\n\nRefrain here.\n\nVerse two.\n",
            "prose",
        )]),
        &side(&[(
            "a.md",
            "Refrain here.\n\nVerse one.\n\nVerse two.\n",
            "prose",
        )]),
    )
    .unwrap();
    assert_eq!(p.revisions.len(), 1, "refrain deletion: one revision");
    assert_eq!(p.revisions[0].what, Some("grex deleted"), "deleted refrain");
    assert_eq!(
        p.revisions[0].parent_grexes,
        vec![2],
        "leftmost tie-break anchors the earliest instances"
    );
}

#[test]
fn manifest_kinds() {
    // Pure rename: same trio, different path.
    let p = detect(
        &side(&[("old.md", "Same text here.\n", "prose")]),
        &side(&[("new.md", "Same text here.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(kinds(&p), vec![RevKind::Rename], "pure rename detected");
    assert_eq!(p.revisions[0].path, "old.md", "ordering key is former path");
    assert_eq!(
        p.revisions[0].new_path.as_deref(),
        Some("new.md"),
        "new path recorded"
    );

    // Rename + edit: removal + first_version (PROVISIONAL.md #5).
    let p = detect(
        &side(&[("old.md", "Original text.\n", "prose")]),
        &side(&[("new.md", "Changed text.\n", "prose")]),
    )
    .unwrap();
    let mut k = kinds(&p);
    k.sort_by_key(|k| format!("{k:?}"));
    assert_eq!(
        k,
        vec![RevKind::FirstVersion, RevKind::Removal],
        "rename+edit decomposes as removal + first-version"
    );

    // Swapped paths: same-path pairing wins; two content edits.
    let p = detect(
        &side(&[
            ("a.md", "Text alpha.\n", "prose"),
            ("b.md", "Text beta.\n", "prose"),
        ]),
        &side(&[
            ("b.md", "Text alpha.\n", "prose"),
            ("a.md", "Text beta.\n", "prose"),
        ]),
    )
    .unwrap();
    assert_eq!(
        kinds(&p),
        vec![RevKind::Edit, RevKind::Edit],
        "swap decomposes as two edits"
    );
    assert_eq!(
        p.revisions
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a.md", "b.md"],
        "swap edits keyed by their own paths"
    );

    // One document at two paths renamed to two fresh paths: the
    // sort-and-zip tie-break pairs deterministically.
    let p = detect(
        &side(&[
            ("a.md", "Twin text.\n", "prose"),
            ("b.md", "Twin text.\n", "prose"),
        ]),
        &side(&[
            ("c.md", "Twin text.\n", "prose"),
            ("d.md", "Twin text.\n", "prose"),
        ]),
    )
    .unwrap();
    assert_eq!(
        kinds(&p),
        vec![RevKind::Rename, RevKind::Rename],
        "twin renames detected"
    );
    let map: BTreeMap<&str, &str> = p
        .revisions
        .iter()
        .map(|r| (r.path.as_str(), r.new_path.as_deref().unwrap()))
        .collect();
    assert_eq!(
        map,
        BTreeMap::from([("a.md", "c.md"), ("b.md", "d.md")]),
        "sort-and-zip pairing is deterministic"
    );

    // Document added / removed.
    let p = detect(
        &side(&[("a.md", "Keep me.\n", "prose")]),
        &side(&[
            ("a.md", "Keep me.\n", "prose"),
            ("b.md", "I am new.\n", "prose"),
        ]),
    )
    .unwrap();
    assert_eq!(kinds(&p), vec![RevKind::FirstVersion], "added document");

    let p = detect(
        &side(&[
            ("a.md", "Keep me.\n", "prose"),
            ("b.md", "Doomed.\n", "prose"),
        ]),
        &side(&[("a.md", "Keep me.\n", "prose")]),
    )
    .unwrap();
    assert_eq!(kinds(&p), vec![RevKind::Removal], "removed document");
}

#[test]
fn mode_flip_with_unchanged_bytes() {
    let p = detect(
        &side(&[("a.md", "Line one. Line two.\nLine three.\n", "prose")]),
        &side(&[("a.md", "Line one. Line two.\nLine three.\n", "poetry")]),
    )
    .unwrap();
    assert!(
        p.revisions.is_empty(),
        "same bytes, different mode: re-segmentation rides the attributes doc"
    );
}

#[test]
fn ordering_and_indices() {
    let p = detect(
        &side(&[
            ("b.md", "Bee one.\n\nBee two.\n", "prose"),
            ("a.md", "Aye one.\n", "prose"),
            ("c.md", "Sea.\n", "prose"),
        ]),
        &side(&[
            ("b.md", "Bee one CHANGED.\n\nBee two CHANGED.\n", "prose"),
            ("a.md", "Aye one CHANGED.\n", "prose"),
            ("d.md", "Sea.\n", "prose"),
        ]),
    )
    .unwrap();
    assert_eq!(
        p.revisions
            .iter()
            .map(|r| format!("{}:{}", r.index, r.path))
            .collect::<Vec<_>>(),
        vec!["1:a.md", "2:b.md", "3:b.md", "4:c.md"],
        "document order: path bytewise, then grex order; indices 1-based"
    );
    assert_eq!(
        p.revisions[3].kind,
        RevKind::Rename,
        "rename ordered by former path"
    );
}
