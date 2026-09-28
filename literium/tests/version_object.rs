// Port of literium-perl t/07-version-object.t — SPEC §8, §6.7.

use std::collections::BTreeMap;

use literium::derive::hash_bytes;
use literium::theca::Theca;
use literium::version_object::VersionObject;

fn trios(pairs: &[(&str, &str, &str)]) -> BTreeMap<String, (String, String)> {
    pairs
        .iter()
        .map(|(v, s, a)| (v.to_string(), (s.to_string(), a.to_string())))
        .collect()
}

fn manifest(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(p, v)| (p.to_string(), v.to_string()))
        .collect()
}

struct Hashes {
    d1: String,
    d2: String,
    s1: String,
    s2: String,
    a1: String,
    a2: String,
}

fn hashes() -> Hashes {
    Hashes {
        d1: hash_bytes(b"doc one"),
        d2: hash_bytes(b"doc two"),
        s1: hash_bytes(b"spina one"),
        s2: hash_bytes(b"spina two"),
        a1: hash_bytes(b"aspina one"),
        a2: hash_bytes(b"aspina two"),
    }
}

fn sample(h: &Hashes) -> VersionObject {
    VersionObject::new(
        "uax29-16.0.0",
        trios(&[(&h.d1, &h.s1, &h.a1), (&h.d2, &h.s2, &h.a2)]),
        manifest(&[("ch1.md", &h.d1), ("notes/ch2.md", &h.d2)]),
    )
    .unwrap()
}

#[test]
fn hash_covers_preimage_only() {
    // SPEC §8.
    let h = hashes();
    let vo = sample(&h);

    let pre = vo.preimage();
    assert!(
        pre.starts_with("lit-version 1\nsegmentation uax29-16.0.0\ntrio "),
        "pre-image header and shape"
    );
    assert!(!pre.contains("ch1.md"), "manifest excluded from pre-image");
    assert_eq!(
        vo.vhash(),
        hash_bytes(pre.as_bytes()),
        "version hash = sha256(pre-image)"
    );

    let other_paths = VersionObject::new(
        "uax29-16.0.0",
        trios(&[(&h.d1, &h.s1, &h.a1), (&h.d2, &h.s2, &h.a2)]),
        manifest(&[("renamed.md", &h.d1), ("elsewhere.md", &h.d2)]),
    )
    .unwrap();
    assert_eq!(
        other_paths.vhash(),
        vo.vhash(),
        "same trios, different manifest -> same version hash"
    );

    // Trio order in the pre-image is by volumen hash.
    let (lo, hi) = if h.d1 < h.d2 {
        (&h.d1, &h.d2)
    } else {
        (&h.d2, &h.d1)
    };
    let lo_pos = pre.find(&format!("trio {lo} ")).unwrap();
    let hi_pos = pre.find(&format!("trio {hi} ")).unwrap();
    assert!(lo_pos < hi_pos, "trios sorted by volumen hash");
}

#[test]
fn serialization_round_trip() {
    let h = hashes();
    let vo = sample(&h);

    let parsed = VersionObject::parse(&vo.serialize()).unwrap();
    assert_eq!(parsed.vhash(), vo.vhash(), "parse preserves hash");
    assert_eq!(parsed.manifest(), vo.manifest(), "parse preserves manifest");
    assert_eq!(parsed.trios(), vo.trios(), "parse preserves trios");

    // Paths with spaces and percent-encoding round-trip.
    let odd = VersionObject::new(
        "uax29-16.0.0",
        trios(&[(&h.d1, &h.s1, &h.a1)]),
        manifest(&[("my draft %1\nweird.md", &h.d1)]),
    )
    .unwrap();
    let odd2 = VersionObject::parse(&odd.serialize()).unwrap();
    assert_eq!(odd2.manifest(), odd.manifest(), "odd path round-trips");

    // Zero-trio version (removal of last documents, SPEC §8.2).
    let empty = VersionObject::new("uax29-16.0.0", BTreeMap::new(), BTreeMap::new()).unwrap();
    let empty2 = VersionObject::parse(&empty.serialize()).unwrap();
    assert_eq!(
        empty2.vhash(),
        empty.vhash(),
        "zero-trio version round-trips"
    );
}

#[test]
fn validation() {
    let h = hashes();

    let e = VersionObject::new(
        "uax29-16.0.0",
        trios(&[(&h.d1, &h.s1, &h.a1)]),
        manifest(&[("x.md", &h.d2)]),
    )
    .unwrap_err();
    assert!(
        e.0.contains("unknown document"),
        "manifest must reference known trios"
    );

    let e = VersionObject::new(
        "uax29-16.0.0",
        trios(&[(&h.d1, &h.s1, &h.a1)]),
        BTreeMap::new(),
    )
    .unwrap_err();
    assert!(
        e.0.contains("no manifest path"),
        "every document needs a path"
    );
}

#[test]
fn theca_binding_and_manifest_rewrite() {
    // SPEC §6.7.
    let h = hashes();
    let vo = sample(&h);

    let dir = tempfile::tempdir().unwrap();
    let theca = Theca::new(dir.path().join("theca"));
    let vhash = vo.store(&theca).unwrap();
    let mut back = VersionObject::load(&theca, &vhash).unwrap().unwrap();
    assert_eq!(back.manifest(), vo.manifest(), "store/load round-trip");

    back.rewrite_manifest(&theca, manifest(&[("moved.md", &h.d1), ("kept.md", &h.d2)]))
        .unwrap();
    let again = VersionObject::load(&theca, &vhash).unwrap().unwrap();
    assert_eq!(
        again.document_at("moved.md"),
        Some(h.d1.as_str()),
        "manifest rewritten in place, hash unchanged"
    );
    assert_eq!(again.vhash(), vhash, "address stable across rewrite");

    let e = back.rewrite_manifest(&theca, BTreeMap::new()).unwrap_err();
    assert!(
        e.0.contains("does not cover"),
        "rewrite must keep covering the trios"
    );
}
