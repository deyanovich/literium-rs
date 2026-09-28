// Port of literium-perl t/06-theca.t — SPEC §6.7.

use literium::derive::hash_bytes;
use literium::theca::{ObjectStore, Theca};

#[test]
fn content_addressed_put_get() {
    let dir = tempfile::tempdir().unwrap();
    let theca = Theca::new(dir.path().join("theca"));

    let bytes = b"First sentence.\n";
    let hash = theca.put("atom", bytes).unwrap();
    assert_eq!(hash, hash_bytes(bytes), "put returns sha256 of raw bytes");
    assert!(theca.has("atom", &hash).unwrap(), "object exists after put");
    assert_eq!(
        theca.get("atom", &hash).unwrap().as_deref(),
        Some(&bytes[..]),
        "round-trip bytes"
    );

    // Sharded path layout: theca/<type>/<aa>/<hash>.
    let aa = &hash[..2];
    assert!(
        dir.path().join("theca/atom").join(aa).join(&hash).is_file(),
        "sharded on-disk layout"
    );

    // Idempotent re-put.
    assert_eq!(theca.put("atom", bytes).unwrap(), hash, "re-put is a no-op");

    // Types are separate namespaces.
    let vhash = theca.put("volumen", bytes).unwrap();
    assert_eq!(vhash, hash, "same bytes, same hash across types");
    assert!(
        dir.path()
            .join("theca/volumen")
            .join(aa)
            .join(&hash)
            .is_file(),
        "stored under its own type"
    );

    assert_eq!(
        theca.get("atom", &hash_bytes(b"absent")).unwrap(),
        None,
        "missing object: None"
    );

    let e = theca.put("blob", b"x").unwrap_err();
    assert!(e.0.contains("unknown object type"), "unknown type rejected");

    let e = theca.put("version", b"x").unwrap_err();
    assert!(
        e.0.contains("put_at"),
        "version type requires the carve-out API"
    );
}

#[test]
fn version_carve_out() {
    // SPEC §6.7.
    let dir = tempfile::tempdir().unwrap();
    let theca = Theca::new(dir.path().join("theca"));

    let vh = format!("ab{}", "0".repeat(62));
    theca.put_at("version", &vh, b"payload-1\n", false).unwrap();
    assert_eq!(
        theca.get("version", &vh).unwrap().as_deref(),
        Some(&b"payload-1\n"[..]),
        "keyed write"
    );
    theca.put_at("version", &vh, b"payload-2\n", false).unwrap();
    assert_eq!(
        theca.get("version", &vh).unwrap().as_deref(),
        Some(&b"payload-1\n"[..]),
        "put_at without overwrite keeps existing bytes"
    );
    theca.put_at("version", &vh, b"payload-2\n", true).unwrap();
    assert_eq!(
        theca.get("version", &vh).unwrap().as_deref(),
        Some(&b"payload-2\n"[..]),
        "overwrite rewrites in place (manifest region mutation)"
    );
}

#[test]
fn listing() {
    let dir = tempfile::tempdir().unwrap();
    let theca = Theca::new(dir.path().join("theca"));
    let hash = theca.put("atom", b"First sentence.\n").unwrap();

    assert_eq!(
        theca.list("atom").unwrap(),
        vec![hash],
        "list finds stored objects"
    );
    assert!(
        theca.list("spina").unwrap().is_empty(),
        "empty type lists nothing"
    );
}
