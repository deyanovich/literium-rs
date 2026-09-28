// Port of literium-perl t/01-extract.t — SPEC §3.

use literium_core::extract::{extract_grexes, is_blank_line, Extraction};

use Extraction::{Poetry, Prose};

#[test]
fn blank_line_definition() {
    // SPEC §3.1 rule 2.
    assert!(is_blank_line(""), "empty line is blank");
    assert!(is_blank_line("   "), "spaces-only line is blank");
    assert!(is_blank_line("\t \t"), "tabs and spaces line is blank");
    assert!(!is_blank_line("\u{A0}"), "lone NBSP line is NOT blank");
    assert!(
        !is_blank_line("\u{2009}"),
        "lone THIN SPACE line is NOT blank"
    );
    assert!(!is_blank_line("x"), "text line is not blank");
}

#[test]
fn line_end_normalization() {
    // SPEC §3.1 rule 1.
    assert_eq!(
        extract_grexes("a\r\nb\r\n\r\nc\r\n", Prose),
        vec!["a b", "c"],
        "CRLF normalized before line handling"
    );
    assert_eq!(
        extract_grexes("a\rb\r\rc", Prose),
        vec!["a b", "c"],
        "lone CR treated as LF"
    );
}

#[test]
fn grex_grouping() {
    // SPEC §3.1 rules 3–4.
    assert_eq!(
        extract_grexes("one\ntwo\n\nthree\n", Prose),
        vec!["one two", "three"],
        "blank line separates grexes"
    );
    assert_eq!(
        extract_grexes("one\n\n\n\n\ntwo\n", Prose),
        vec!["one", "two"],
        "blank-line run is one separator"
    );
    assert_eq!(
        extract_grexes("\n\n  \none\n\n \t\n", Prose),
        vec!["one"],
        "leading and trailing blank lines removed"
    );
    assert_eq!(
        extract_grexes("one\n \t \ntwo\n", Prose),
        vec!["one", "two"],
        "whitespace-bearing blank line still separates"
    );
}

#[test]
fn prose_collapse() {
    // SPEC §3.2 rule 6.
    assert_eq!(
        extract_grexes("a  b\t\tc\nd\n", Prose),
        vec!["a b c d"],
        "space/tab/LF runs collapse to single space"
    );
    // SPEC §3.1 rule 5 + note after rule 2: non-ASCII whitespace
    // survives, and a lone-NBSP line does not separate grexes.
    assert_eq!(
        extract_grexes("text\n\u{A0}\nmore\n", Prose),
        vec!["text \u{A0} more"],
        "NBSP-only line joins one grex, NBSP survives"
    );
    assert_eq!(
        extract_grexes("10\u{202F}000 km\n", Prose),
        vec!["10\u{202F}000 km"],
        "NNBSP survives extraction verbatim"
    );
}

#[test]
fn poetry_rules() {
    // SPEC §3.3.
    assert_eq!(
        extract_grexes("line one\nline two\n\nline three\n", Poetry),
        vec!["line one\nline two", "line three"],
        "poetry preserves LFs within a stanza"
    );
    assert_eq!(
        extract_grexes("  indented\t line \n", Poetry),
        vec![" indented line "],
        "poetry collapses space/tab runs within a line only"
    );
}

#[test]
fn empty_input_shapes() {
    assert!(
        extract_grexes("", Prose).is_empty(),
        "empty text: no grexes"
    );
    assert!(
        extract_grexes("\n \n ", Prose).is_empty(),
        "whitespace-only: no grexes"
    );
}
