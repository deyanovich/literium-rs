// Verbatim atom spans (SPEC §8.6): partition invariants and
// first-contributing-byte correctness over the fixture corpus and
// adversarial constructions.

use literium::derive::derive;
use literium::grex_ranges;
use literium::spans::{assembly_bytes, atom_spans};

fn check(bytes: &[u8], mode: &str) {
    let Some(d) = derive(bytes, mode) else {
        return; // not a document — nothing to span
    };
    let ranges = grex_ranges::extract(bytes, &d.atom_bearing).unwrap();
    let spans = atom_spans(&d).unwrap();
    assert_eq!(spans.len(), d.grexes.len(), "one span list per grex");
    for (k, (grex_spans, grex_atoms)) in spans.iter().zip(d.grexes.iter()).enumerate() {
        assert_eq!(
            grex_spans.len(),
            grex_atoms.len(),
            "grex {k}: one span per atom"
        );
        let (rs, re) = ranges[k];
        assert_eq!(
            grex_spans[0].start, rs,
            "grex {k}: first span at range start"
        );
        assert_eq!(
            grex_spans.last().unwrap().end,
            re,
            "grex {k}: last span at range end"
        );
        for w in grex_spans.windows(2) {
            assert_eq!(w[0].end, w[1].start, "grex {k}: spans partition");
        }
        for (i, (span, atom)) in grex_spans.iter().zip(grex_atoms.iter()).enumerate() {
            assert!(span.start <= span.first && span.first < span.end);
            if i > 0 {
                assert_eq!(
                    span.start, span.first,
                    "non-first spans start at first byte"
                );
            }
            // The first contributing byte begins the atom's first
            // character.
            let first_char = atom.chars().next().unwrap();
            let mut buf = [0u8; 4];
            let enc = first_char.encode_utf8(&mut buf).as_bytes();
            assert!(
                bytes[span.first..].starts_with(enc),
                "grex {k} atom {i}: span.first points at the atom's first char"
            );
            // Assembly bytes are non-empty and start the same way.
            let ab = assembly_bytes(bytes, span);
            assert!(ab.starts_with(enc), "grex {k} atom {i}: assembly bytes");
        }
    }
}

#[test]
fn fixtures_both_modes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().extension().is_none_or(|x| x != "txt") {
            continue;
        }
        let bytes = std::fs::read(e.path()).unwrap();
        check(&bytes, "prose");
        check(&bytes, "poetry");
        n += 1;
    }
    assert!(n > 5, "fixtures present");
}

#[test]
fn adversarial_constructions() {
    // Leading blanks + phantom grex + CRLF + NBSP seams.
    check(b"\n\n  \nAlpha one. Alpha two.\n\nBeta.\n", "prose");
    check("One.\n\n\u{A0}\n\nTwo.\n".as_bytes(), "prose");
    check(b"A b.\r\n\r\nC d.\r\n", "prose");
    check("One.\u{A0}Two after nbsp.\n".as_bytes(), "prose");
    check(b"  indented line\n\tanother\n\nstanza two\n", "poetry");
    check("l1\n\u{A0}\nl3\n".as_bytes(), "poetry");
    check(b"No newline at all", "prose");
    check(b"wrapped\nsentence here. Next one.\n", "prose");
}

#[test]
fn assembly_reproduces_single_side_grex() {
    // Joining every atom's assembly bytes with the mode joiner
    // reproduces a normalized form whose derivation has the same
    // atom sequence (the §9.6 merged-grex property).
    let bytes = b"Alpha one.  Alpha\ntwo. Alpha three.\n\nBeta.\n";
    let d = derive(bytes, "prose").unwrap();
    let spans = atom_spans(&d).unwrap();
    let joined: Vec<u8> = spans[0]
        .iter()
        .map(|s| assembly_bytes(bytes, s))
        .collect::<Vec<_>>()
        .join(&b" "[..]);
    let mut doc = joined.clone();
    doc.push(b'\n');
    let d2 = derive(&doc, "prose").unwrap();
    assert_eq!(
        d2.grexes[0], d.grexes[0],
        "assembled grex preserves the atom sequence"
    );
    // Atom-interior formatting (the mid-sentence wrap) survives.
    assert!(
        joined.windows(6).any(|w| w == b"Alpha\n"),
        "intra-atom newline preserved verbatim"
    );
}
