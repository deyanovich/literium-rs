// Port of literium-perl t/10-grexranges.t — SPEC §8.6.

use literium::derive::derive;
use literium::grex_ranges::extract;

fn ranges_of(bytes: &[u8], mode: &str) -> (Vec<(usize, usize)>, Vec<u8>) {
    let d = derive(bytes, mode).expect("not a document");
    (extract(bytes, &d.atom_bearing).unwrap(), bytes.to_vec())
}

fn range_bytes(ranges: &[(usize, usize)], bytes: &[u8]) -> Vec<Vec<u8>> {
    ranges.iter().map(|&(s, e)| bytes[s..e].to_vec()).collect()
}

fn assert_ranges(bytes: &[u8], mode: &str, expected: &[&[u8]], label: &str) {
    let (r, b) = ranges_of(bytes, mode);
    assert_eq!(
        range_bytes(&r, &b),
        expected.iter().map(|x| x.to_vec()).collect::<Vec<_>>(),
        "{label}"
    );
}

#[test]
fn basic_partition() {
    // Each range owns its trailing separator run.
    assert_ranges(
        b"Alpha one.\n\nBeta two.\n\n\nGamma three.\n",
        "prose",
        &[b"Alpha one.\n\n", b"Beta two.\n\n\n", b"Gamma three.\n"],
        "each range owns its trailing separator run",
    );
    assert_ranges(
        b"\n\n  \nAlpha.\n\nBeta.\n",
        "prose",
        &[b"\n\n  \nAlpha.\n\n", b"Beta.\n"],
        "leading blank lines belong to the first range",
    );
    assert_ranges(
        b"Alpha.\n\nBeta.\n\n \n",
        "prose",
        &[b"Alpha.\n\n", b"Beta.\n\n \n"],
        "trailing blank lines belong to the last range",
    );
    assert_ranges(
        b"No newline at all",
        "prose",
        &[b"No newline at all"],
        "no trailing LF",
    );
}

#[test]
fn crlf_and_lone_cr_delimit_only() {
    assert_ranges(
        b"Alpha.\r\n\r\nBeta.\r\n",
        "prose",
        &[b"Alpha.\r\n\r\n", b"Beta.\r\n"],
        "CRLF delimits; bytes preserved verbatim",
    );
    assert_ranges(
        b"Alpha.\r\rBeta.\r",
        "prose",
        &[b"Alpha.\r\r", b"Beta.\r"],
        "lone CR delimits",
    );
}

#[test]
fn phantom_grex_fold() {
    // Isolated NBSP line between blank lines: raw grex with zero
    // atoms; folds into the preceding atom-bearing range.
    let nbsp = "One.\n\n\u{A0}\n\nTwo.\n".as_bytes();
    let d = derive(nbsp, "prose").unwrap();
    assert_eq!(d.atom_bearing, vec![true, false, true], "phantom flagged");
    let pr = extract(nbsp, &d.atom_bearing).unwrap();
    assert_eq!(
        range_bytes(&pr, nbsp),
        vec!["One.\n\n\u{A0}\n\n".as_bytes().to_vec(), b"Two.\n".to_vec()],
        "phantom folds into the preceding range"
    );

    // Phantom at document head folds into the first range.
    let lead = "\u{A0}\n\nOne.\n".as_bytes();
    let dl = derive(lead, "prose").unwrap();
    assert_eq!(
        dl.atom_bearing,
        vec![false, true],
        "leading phantom flagged"
    );
    let lr = extract(lead, &dl.atom_bearing).unwrap();
    assert_eq!(
        range_bytes(&lr, lead),
        vec!["\u{A0}\n\nOne.\n".as_bytes().to_vec()],
        "leading phantom folds into the first range"
    );
}

#[test]
fn poetry_mode() {
    assert_ranges(
        b"  line one\n  line two\n\n  stanza two\n",
        "poetry",
        &[b"  line one\n  line two\n\n", b"  stanza two\n"],
        "poetry stanza ranges, indentation preserved in bytes",
    );
}

#[test]
fn errors() {
    let e = extract(b"One.\n", &[true, true]).unwrap_err();
    assert!(e.0.contains("does not match"), "flag count mismatch errors");
}
