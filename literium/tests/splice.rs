// Port of literium-perl t/12-splice.t — SPEC §8.6.

use std::collections::{BTreeMap, HashMap, HashSet};

use literium::derive::derive;
use literium::revision_detect::{detect, Pending, RevKind, SideInput};
use literium::splice::volumen_for;

fn side(files: &[(&str, &[u8], &str)]) -> SideInput {
    let mut manifest = BTreeMap::new();
    let mut states = HashMap::new();
    for (path, bytes, mode) in files {
        let d = derive(bytes, mode).unwrap_or_else(|| panic!("not a document: {path}"));
        manifest.insert(path.to_string(), d.volumen_hash.clone());
        states.insert(path.to_string(), d);
    }
    SideInput { manifest, states }
}

fn ids(p: &Pending) -> HashSet<String> {
    p.revisions.iter().map(|r| r.id.clone()).collect()
}

/// Asserts the two splice boundary properties, then returns the
/// pending set for further single-subset checks.
fn check(pb: &[u8], tb: &[u8], label: &str) -> Pending {
    let p = detect(
        &side(&[("a.md", pb, "prose")]),
        &side(&[("a.md", tb, "prose")]),
    )
    .unwrap();
    let doc = p.docs.get("a.md").unwrap();
    assert_eq!(
        volumen_for(doc, &HashSet::new()),
        pb,
        "{label}: empty subset = parent"
    );
    assert_eq!(
        volumen_for(doc, &ids(&p)),
        tb,
        "{label}: full subset = tabula"
    );
    p
}

#[test]
fn spec_designed_examples() {
    // Insertion at document start; leading blanks stay with
    // parent's first range (the design's worked example).
    let p = check(
        b"\nAlpha one.\n",
        b"New start.\n\nAlpha one.\n",
        "head insert",
    );
    let ins = p
        .revisions
        .iter()
        .find(|r| r.kind == RevKind::Edit)
        .unwrap();
    assert!(
        p.revisions.iter().any(|r| r.kind == RevKind::Formatting),
        "insert + formatting revisions detected"
    );
    let doc = p.docs.get("a.md").unwrap();
    assert_eq!(
        volumen_for(doc, &HashSet::from([ins.id.clone()])),
        b"New start.\n\n\nAlpha one.\n",
        "insert-only intermediate carries the longer blank run"
    );

    // Delete the last grex.
    let p = check(b"Alpha.\n\nBeta.\n", b"Alpha.\n", "tail delete");
    let del = p
        .revisions
        .iter()
        .find(|r| r.kind == RevKind::Edit)
        .unwrap();
    let doc = p.docs.get("a.md").unwrap();
    assert_eq!(
        volumen_for(doc, &HashSet::from([del.id.clone()])),
        b"Alpha.\n\n",
        "delete-only intermediate keeps the separator"
    );
}

#[test]
fn order_independence() {
    let p = check(
        b"One first.\n\nTwo second.\n\nThree third.\n",
        b"One CHANGED.\n\nTwo second.\n\nThree CHANGED.\n",
        "two disjoint edits",
    );
    let revs: Vec<_> = p
        .revisions
        .iter()
        .filter(|r| r.kind == RevKind::Edit)
        .collect();
    assert_eq!(revs.len(), 2, "two edit revisions");
    let doc = p.docs.get("a.md").unwrap();
    assert_eq!(
        volumen_for(doc, &HashSet::from([revs[0].id.clone()])),
        b"One CHANGED.\n\nTwo second.\n\nThree third.\n",
        "first revision alone"
    );
    assert_eq!(
        volumen_for(doc, &HashSet::from([revs[1].id.clone()])),
        b"One first.\n\nTwo second.\n\nThree CHANGED.\n",
        "second revision alone"
    );
    assert_eq!(
        volumen_for(
            doc,
            &HashSet::from([revs[0].id.clone(), revs[1].id.clone()])
        ),
        volumen_for(
            doc,
            &HashSet::from([revs[1].id.clone(), revs[0].id.clone()])
        ),
        "subset-based: order cannot matter"
    );
}

#[test]
fn structural_formatting_crlf_phantom_scenarios() {
    check(
        b"One. Two. Three.\n",
        b"One. Two.\n\nThree.\n",
        "pure split",
    );
    check(
        b"One. Two.\n\nThree.\n",
        b"One. Two. Three.\n",
        "pure merge",
    );
    check(
        b"A b.\r\n\r\nC d.\r\n",
        b"A b.\n\nC d.\n",
        "CRLF -> LF conversion",
    );
    check(
        b"Para one here. And more.\n\nPara two.\n",
        b"Para one\nhere. And more.\n\nPara two.\n",
        "rewrap",
    );
    check(
        "One.\n\n\u{A0}\n\nTwo.\n".as_bytes(),
        "One.\n\n\u{A0}\n\nTwo changed.\n".as_bytes(),
        "phantom grex in parent and tabula",
    );
    check(
        "One.\n\nTwo.\n".as_bytes(),
        "One.\n\n\u{A0}\n\nTwo.\n".as_bytes(),
        "phantom appears in tabula only",
    );
    check(
        b"\n\nLead.\n",
        b"Lead.\n\n\n",
        "leading/trailing blank changes",
    );
}

#[test]
fn poetry_scenario() {
    // Poetry: needs poetry mode on both sides.
    let p = detect(
        &side(&[("a.md", b"line one\nline two\n\nstanza two\n", "poetry")]),
        &side(&[(
            "a.md",
            b"line one\nline two changed\n\nstanza two\nnew line\n",
            "poetry",
        )]),
    )
    .unwrap();
    let doc = p.docs.get("a.md").unwrap();
    assert_eq!(
        volumen_for(doc, &HashSet::new()),
        b"line one\nline two\n\nstanza two\n",
        "poetry: empty subset = parent"
    );
    assert_eq!(
        volumen_for(doc, &ids(&p)),
        b"line one\nline two changed\n\nstanza two\nnew line\n",
        "poetry: full subset = tabula"
    );
}

#[test]
fn no_anchor_rewrite() {
    check(
        b"Aaa.\n\nBbb.\n",
        b"Xxx.\n\nYyy.\n\nZzz.\n",
        "no-anchor rewrite",
    );
}
