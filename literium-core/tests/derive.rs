// Port of literium-perl t/03-derive.t — SPEC §5–§6.

use literium_core::derive::{derive, hash_bytes, reconstruct};

#[test]
fn document_test() {
    // SPEC §1.3.
    assert!(derive(b"", "prose").is_none(), "empty file: not a document");
    assert!(
        derive(b"  \n\t\n", "prose").is_none(),
        "whitespace-only: not a document"
    );
    assert!(
        derive(b"\xFF\xFE broken", "prose").is_none(),
        "invalid UTF-8: not a document"
    );
    assert!(
        derive("\u{A0}\n".as_bytes(), "prose").is_none(),
        "NBSP-only file: grex with zero atoms, not a document"
    );
}

#[test]
fn basic_derivation() {
    let bytes = "One. Two!\n\nThree?\n".as_bytes();
    let d = derive(bytes, "prose").expect("document derives");

    assert_eq!(d.volumen, bytes, "volumen is verbatim bytes");
    assert_eq!(
        d.volumen_hash,
        hash_bytes(bytes),
        "document identity = sha256(volumen)"
    );

    assert_eq!(
        d.grexes,
        vec![vec!["One.", "Two!"], vec!["Three?"]],
        "grex/atom structure"
    );

    // Atom identity (SPEC §5): sha256 of content-form bytes.
    let h_one = hash_bytes(b"One.");
    assert_eq!(
        d.atoms.get(&h_one).map(Vec::as_slice),
        Some(&b"One."[..]),
        "atom stored under content hash, holds content form bytes"
    );

    // Fasciculus grammar (SPEC §6.3): hash per line, trailing LF.
    let h_two = hash_bytes(b"Two!");
    let fasc1 = format!("{h_one}\n{h_two}\n");
    let fh1 = hash_bytes(fasc1.as_bytes());
    assert_eq!(d.fasc_hashes[0], fh1, "fasciculus hash of first grex");
    assert_eq!(
        d.fasciculi.get(&fh1).map(Vec::as_slice),
        Some(fasc1.as_bytes()),
        "fasciculus bytes"
    );

    // Spina grammar (SPEC §6.4).
    let h_three = hash_bytes(b"Three?");
    let fasc2 = format!("{h_three}\n");
    let fh2 = hash_bytes(fasc2.as_bytes());
    assert_eq!(
        d.spina,
        format!("{fh1}\n{fh2}\n").into_bytes(),
        "spina bytes"
    );
    assert_eq!(d.spina_hash, hash_bytes(&d.spina), "spina hash");

    // Atom-spina (SPEC §6.6): flat atom hashes in document order,
    // equal to concatenating each fasciculus's list in spina order.
    assert_eq!(
        d.atom_spina,
        format!("{h_one}\n{h_two}\n{h_three}\n").into_bytes(),
        "atom-spina bytes"
    );
    assert_eq!(
        d.atom_spina,
        [fasc1.as_bytes(), fasc2.as_bytes()].concat(),
        "atom-spina = concatenated fasciculi"
    );

    // Content form by construction (SPEC §3.4).
    assert_eq!(
        d.content_form, b"One. Two!\n\nThree?\n",
        "content form join"
    );

    // Reconstruction invariant (SPEC §6.5).
    assert_eq!(
        reconstruct(&d.spina, &d.fasciculi, &d.atoms, "prose"),
        d.content_form,
        "reconstruction equals content form (prose)"
    );
}

#[test]
fn zero_width_boundary_content_form() {
    // SPEC §3.4's non-byte-faithful example: One!Two.
    let z = derive(b"One!Two.\n", "prose").unwrap();
    assert_eq!(
        z.content_form, b"One! Two.\n",
        "content form not byte-faithful at zero-width boundaries"
    );
    assert_eq!(
        reconstruct(&z.spina, &z.fasciculi, &z.atoms, "prose"),
        z.content_form,
        "invariant holds regardless"
    );
}

#[test]
fn deduplication() {
    // SPEC §6.2, §6.3.
    let dup = derive(b"Same. Same.\n\nSame. Same.\n\nOther.\n", "prose").unwrap();
    assert_eq!(dup.atoms.len(), 2, "identical atoms dedupe repository-wide");
    assert_eq!(
        dup.fasciculi.len(),
        2,
        "identical grexes share one fasciculus object"
    );
    assert_eq!(dup.fasc_hashes.len(), 3, "spina still lists three entries");
    assert_eq!(
        dup.fasc_hashes[0], dup.fasc_hashes[1],
        "repeated paragraph -> same fasciculus hash"
    );
}

#[test]
fn poetry_derivation() {
    let p = derive(
        "  First line.\n\tSecond, indented\n\nNew stanza\n".as_bytes(),
        "poetry",
    )
    .unwrap();
    assert_eq!(
        p.grexes,
        vec![vec!["First line.", "Second, indented"], vec!["New stanza"]],
        "poetry atoms, indentation stripped in content form"
    );
    assert_eq!(
        p.content_form, b"First line.\nSecond, indented\n\nNew stanza\n",
        "poetry content form joins atoms with LF"
    );
    assert_eq!(
        reconstruct(&p.spina, &p.fasciculi, &p.atoms, "poetry"),
        p.content_form,
        "reconstruction equals content form (poetry)"
    );
}

#[test]
fn mode_is_load_bearing_for_identity() {
    // SPEC §1.1, §5.
    let same_bytes = "Alpha beta. Gamma delta.\nSecond line here.\n".as_bytes();
    let as_prose = derive(same_bytes, "prose").unwrap();
    let as_poetry = derive(same_bytes, "poetry").unwrap();
    assert_eq!(
        as_prose.volumen_hash, as_poetry.volumen_hash,
        "same bytes, same document identity"
    );
    assert_ne!(
        as_prose.atom_spina_hash, as_poetry.atom_spina_hash,
        "same bytes, different mode -> different atom-spina"
    );
}

#[test]
fn no_unicode_normalization() {
    // SPEC §0.
    let nfc = derive("caf\u{E9}.\n".as_bytes(), "prose").unwrap();
    let nfd = derive("cafe\u{301}.\n".as_bytes(), "prose").unwrap();
    assert_ne!(
        nfc.atom_spina_hash, nfd.atom_spina_hash,
        "NFC and NFD are different atoms by design"
    );
}

#[test]
fn atom_bearing_flags() {
    // Phantom-grex input for SPEC §8.6. The NBSP line inside the
    // first raw grex is not blank, so it joins that grex.
    let ph = derive("Text one.\n\u{A0}\n\nText two.\n".as_bytes(), "prose").unwrap();
    assert_eq!(
        ph.atom_bearing,
        vec![true, true],
        "NBSP joins grex, both atom-bearing"
    );

    let ph2 = derive("Text one.\n\n\u{A0}\n\nText two.\n".as_bytes(), "prose").unwrap();
    assert_eq!(
        ph2.atom_bearing,
        vec![true, false, true],
        "isolated NBSP-only raw grex derives zero atoms (phantom)"
    );
    assert_eq!(
        ph2.grexes.len(),
        2,
        "phantom grex contributes no fasciculus"
    );
}
