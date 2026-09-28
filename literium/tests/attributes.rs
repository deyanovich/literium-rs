// Port of literium-perl t/05-attributes.t — SPEC §1.1.

use literium::attributes::Attributes;

#[test]
fn parsing_and_first_match_wins() {
    let attrs = Attributes::parse(
        "# comment line\npoems/*.txt  poetry\n\nverse/\tpoetry\n*.txt\tprose\nepic.md poetry\n",
    )
    .expect("parses");

    assert_eq!(attrs.len(), 4, "comments and blanks skipped");

    // First-match-wins (SPEC §1.1 — inverts .gitattributes).
    assert_eq!(
        attrs.mode_for("poems/ode.txt"),
        "poetry",
        "specific rule wins"
    );
    assert_eq!(attrs.mode_for("notes.txt"), "prose", "general rule applies");
    assert_eq!(attrs.mode_for("epic.md"), "poetry", "exact name entry");

    // No matching entry -> prose (SPEC §1.1).
    assert_eq!(
        attrs.mode_for("README.rst"),
        "prose",
        "unmatched file is prose"
    );

    // Directory rule covers contained files (gitignore semantics).
    assert_eq!(
        attrs.mode_for("verse/canto/1.md"),
        "poetry",
        "dir-only rule covers files beneath it"
    );

    // Tab and space column separators are equivalent.
    assert_eq!(
        attrs.mode_for("anything.txt"),
        "prose",
        "tab-separated entry"
    );
}

#[test]
fn escaped_whitespace_in_globs() {
    let ws = Attributes::parse("my\\ poems/*  poetry\n").unwrap();
    assert_eq!(
        ws.mode_for("my poems/a.txt"),
        "poetry",
        "escaped space in glob"
    );
}

#[test]
fn rejections() {
    let e = Attributes::parse("!poems/* poetry\n").unwrap_err();
    assert!(
        e.0.contains("negation is not permitted"),
        "negation rejected (SPEC §1.1): {e}"
    );

    let e = Attributes::parse("*.txt verse\n").unwrap_err();
    assert!(e.0.contains("mode must be"), "unknown mode rejected: {e}");

    let e = Attributes::parse("loneglob\n").unwrap_err();
    assert!(e.0.contains("expected"), "single-column line rejected: {e}");
}

#[test]
fn empty_file_defaults_to_prose() {
    let empty = Attributes::parse("").unwrap();
    assert_eq!(
        empty.mode_for("x.txt"),
        "prose",
        "empty file defaults to prose"
    );
}

#[test]
fn control_files_are_always_prose() {
    // SPEC §1.1/§1.2 — the override wins over every entry,
    // including a catch-all.
    let all_poetry = Attributes::parse("* poetry\n").unwrap();
    assert_eq!(
        all_poetry.mode_for(".litattributes"),
        "prose",
        "attributes file is prose despite catch-all poetry"
    );
    assert_eq!(
        all_poetry.mode_for(".litignore"),
        "prose",
        "root .litignore is prose despite catch-all poetry"
    );
    assert_eq!(
        all_poetry.mode_for("part/.litignore"),
        "prose",
        "subdirectory .litignore is prose despite catch-all poetry"
    );
    assert_eq!(
        all_poetry.mode_for("verse.txt"),
        "poetry",
        "catch-all still applies to ordinary documents"
    );

    // An explicit entry naming a control file is equally overridden.
    let explicit = Attributes::parse(".litignore poetry\n").unwrap();
    assert_eq!(
        explicit.mode_for(".litignore"),
        "prose",
        "explicit control-file entry cannot re-mode it"
    );
}
