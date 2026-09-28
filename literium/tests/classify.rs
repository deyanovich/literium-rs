// Port of literium-perl t/18-classify.t — SPEC §7.2 + diff atom_ops.

use literium::classify::classify;
use literium::derive::derive;
use literium::diff::{atom_ops, Op};

fn trio(bytes: &[u8], mode: &str) -> [String; 3] {
    let d = derive(bytes, mode).unwrap();
    [d.volumen_hash, d.spina_hash, d.atom_spina_hash]
}

fn refs(t: &[String; 3]) -> [&str; 3] {
    [t[0].as_str(), t[1].as_str(), t[2].as_str()]
}

#[test]
fn spec_table_row_by_row() {
    let base = trio(b"One. Two.\n\nThree.\n", "prose");
    assert_eq!(
        classify(&refs(&base), &refs(&base)).unwrap(),
        "no-change",
        "row 1: no change"
    );

    assert_eq!(
        classify(
            &refs(&base),
            &refs(&trio(b"One.  Two.\n\n\nThree.\n", "prose"))
        )
        .unwrap(),
        "formatting",
        "row 2: volumen differs, derived objects equal"
    );

    assert_eq!(
        classify(&refs(&base), &refs(&trio(b"One. Two. Three.\n", "prose"))).unwrap(),
        "structural",
        "row 3: regrouping, flat atom sequence unchanged"
    );

    assert_eq!(
        classify(
            &refs(&base),
            &refs(&trio(b"One. Two CHANGED.\n\nThree.\n", "prose"))
        )
        .unwrap(),
        "substantial",
        "row 4: atom hashes changed"
    );

    // A cross-grex move reorders the flat sequence -> substantial
    // (SPEC §7.2 note).
    assert_eq!(
        classify(
            &refs(&trio(b"One. Two.\n\nThree.\n", "prose")),
            &refs(&trio(b"Three.\n\nOne. Two.\n", "prose"))
        )
        .unwrap(),
        "substantial",
        "cross-grex move classifies substantial"
    );

    // Row 5: same bytes, different mode (re-segmentation).
    let poemish = b"Alpha beta. Gamma.\nDelta epsilon.\n";
    assert_eq!(
        classify(
            &refs(&trio(poemish, "prose")),
            &refs(&trio(poemish, "poetry"))
        )
        .unwrap(),
        "re-segmentation",
        "row 5: same bytes, different reading"
    );

    // Impossible combination errors.
    let a = trio(b"One. Two.\n\nThree.\n", "prose");
    let fake = "f".repeat(64);
    let b = [a[0].as_str(), fake.as_str(), a[2].as_str()];
    let e = classify(&refs(&a), &b).unwrap_err();
    assert!(
        e.0.contains("impossible"),
        "impossible combination rejected"
    );
}

#[test]
fn atom_ops_lcs() {
    let ops = atom_ops(&["a", "b", "c"], &["a", "x", "c"]);
    assert_eq!(
        ops,
        vec![
            (Op::Equal, "a"),
            (Op::Del, "b"),
            (Op::Ins, "x"),
            (Op::Equal, "c")
        ],
        "modify in place"
    );
    assert_eq!(atom_ops(&[], &["a"]), vec![(Op::Ins, "a")], "pure insert");
    assert_eq!(atom_ops(&["a"], &[]), vec![(Op::Del, "a")], "pure delete");
    assert_eq!(
        atom_ops(&["a", "b"], &["a", "b"]),
        vec![(Op::Equal, "a"), (Op::Equal, "b")],
        "no change"
    );
    assert_eq!(
        atom_ops(&["a", "b", "c", "d"], &["c", "d", "a", "b"]),
        vec![
            (Op::Del, "a"),
            (Op::Del, "b"),
            (Op::Equal, "c"),
            (Op::Equal, "d"),
            (Op::Ins, "a"),
            (Op::Ins, "b")
        ],
        "move shown as delete + insert (leftmost trace)"
    );
}
