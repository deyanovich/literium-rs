// Port of literium-perl t/02-segment.t — SPEC §4.

use literium_core::extract::Extraction;
use literium_core::segment::{atoms_of_grex, edge_strip, mode_spec, register_mode, ModeSpec};

fn atoms(grex: &str, mode: &str) -> Vec<String> {
    let spec = mode_spec(mode).expect("known mode");
    atoms_of_grex(grex, &spec)
        .into_iter()
        .map(str::to_string)
        .collect()
}

#[test]
fn atom_edge_strip() {
    // SPEC §4.3 — property class: Sentence_Break in {Sp, Sep, CR, LF}.
    assert_eq!(edge_strip("  hello  "), "hello", "ASCII space stripped");
    assert_eq!(edge_strip("\thello\t"), "hello", "TAB stripped (Sp)");
    assert_eq!(
        edge_strip("\u{A0}hello\u{A0}"),
        "hello",
        "NBSP stripped (Sp)"
    );
    assert_eq!(edge_strip("\u{2009}hi\u{2009}"), "hi", "THIN SPACE (Sp)");
    assert_eq!(edge_strip("\u{202F}hi\u{202F}"), "hi", "NNBSP (Sp)");
    assert_eq!(edge_strip("\u{C}hi\u{B}"), "hi", "FF and VT stripped (Sp)");
    assert_eq!(edge_strip("\u{85}hi\u{2028}"), "hi", "NEL and LS (Sep)");
    assert_eq!(edge_strip("\u{2029}hi\u{2029}"), "hi", "PS stripped (Sep)");
    assert_eq!(edge_strip("\rhi\n"), "hi", "CR and LF stripped");
    assert_eq!(
        edge_strip("a\u{A0}b"),
        "a\u{A0}b",
        "interior NBSP untouched"
    );
    assert_eq!(
        edge_strip("\u{A0} \u{2028}"),
        "",
        "all-strippable segment strips empty"
    );
}

#[test]
fn prose_segmentation() {
    // SPEC §4.1, strict UAX-29.
    assert_eq!(
        atoms("One. Two! Three?", "prose"),
        vec!["One.", "Two!", "Three?"],
        "basic sentence split"
    );
    // SPEC §3.4's example: a zero-width UAX-29 boundary after STerm.
    assert_eq!(
        atoms("One!Two.", "prose"),
        vec!["One!", "Two."],
        "zero-width boundary after STerm splits"
    );
    // SPEC §4.1: UAX-29 mis-segmentations are accepted, not
    // corrected. "Mr." before uppercase "Smith" DOES break (SB11).
    assert_eq!(
        atoms("Mr. Smith left. He returned.", "prose"),
        vec!["Mr.", "Smith left.", "He returned."],
        "known UAX-29 Mr. limitation preserved, not worked around"
    );
    // SB8: after ATerm, skippable classes then lowercase suppress
    // the break — "Fig. 4 above" holds together without any
    // abbreviation list.
    assert_eq!(
        atoms("See Fig. 4 above. Next.", "prose"),
        vec!["See Fig. 4 above.", "Next."],
        "UAX-29 SB8: no break when lowercase continuation follows"
    );
    assert_eq!(
        atoms("No terminator here", "prose"),
        vec!["No terminator here"],
        "unterminated text is one sentence"
    );
}

#[test]
fn poetry_segmentation() {
    // SPEC §4.2.
    assert_eq!(
        atoms("first line\nsecond line", "poetry"),
        vec!["first line", "second line"],
        "poetry splits at LF"
    );
    // Indentation is stripped from content form only (SPEC §4.2).
    assert_eq!(
        atoms(" \u{A0}indented\nplain", "poetry"),
        vec!["indented", "plain"],
        "leading indentation (incl. NBSP) stripped"
    );
    // A line of only strippable characters is not an atom (§4.3).
    assert_eq!(
        atoms("one\n\u{A0}\ntwo", "poetry"),
        vec!["one", "two"],
        "all-strippable line contributes no atom"
    );
    assert_eq!(
        atoms("He said. She said. All on one line", "poetry"),
        vec!["He said. She said. All on one line"],
        "poetry mode never splits at sentence boundaries"
    );
}

fn word_boundary(grex: &str) -> Vec<&str> {
    grex.split(' ').collect()
}

#[test]
fn mode_registry() {
    // SPEC §4 extension point.
    assert!(mode_spec("prose").is_some(), "prose registered");
    assert!(mode_spec("poetry").is_some(), "poetry registered");
    assert!(mode_spec("code").is_none(), "unknown mode is None");

    register_mode(
        "test-words",
        ModeSpec {
            atom_boundary: word_boundary,
            atom_join: " ",
            extraction: Extraction::Prose,
        },
    )
    .expect("fresh name registers");
    assert_eq!(
        atoms("alpha beta", "test-words"),
        vec!["alpha", "beta"],
        "registered custom mode segments"
    );
    assert!(
        register_mode(
            "prose",
            ModeSpec {
                atom_boundary: word_boundary,
                atom_join: " ",
                extraction: Extraction::Prose,
            }
        )
        .is_err(),
        "re-registering a mode errors"
    );
}
