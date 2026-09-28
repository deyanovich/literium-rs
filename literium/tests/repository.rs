// Port of literium-perl t/09-repository.t — SPEC §1.3, §8.1, §9.2, §4.

use std::path::Path;

use literium::repository::Repository;
use literium::theca::ObjectStore;
use literium::version_object::VersionObject;

fn spew(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn repository_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = std::fs::canonicalize(tmp.path()).unwrap();
    spew(&dir.join("ch1.md"), b"First sentence. Second one.\n");

    // ── Creation (SPEC §1.3) ────────────────────────────────────
    let repo = Repository::create(&dir, "prose").unwrap();
    assert!(dir.join(".lit/theca").is_dir(), "theca created");
    assert!(
        dir.join(".lit/historia").is_file(),
        "empty historia created"
    );
    assert!(repo.historia().is_empty().unwrap(), "historia starts empty");
    assert_eq!(
        repo.segmentation_version(),
        "uax29-16.0.0",
        "segmentation recorded"
    );
    assert_eq!(
        std::fs::read(dir.join(".litattributes")).unwrap(),
        b"* prose\n",
        "attributes carry the confirmed mode"
    );
    assert!(
        repo.assert_segmentation().is_ok(),
        "segmentation guard passes"
    );

    let e = Repository::create(&dir, "prose").unwrap_err();
    assert!(e.0.contains("inside one"), "no nested creation");

    // ── Discovery (SPEC §1.3) ───────────────────────────────────
    std::fs::create_dir_all(dir.join("part1/deep")).unwrap();
    assert_eq!(
        Repository::discover(&dir.join("part1/deep")).as_deref(),
        Some(dir.as_path()),
        "discovery walks upward to the repo root"
    );
    let outside = tempfile::tempdir().unwrap();
    assert_eq!(
        Repository::discover(outside.path()),
        None,
        "no repository found outside"
    );

    // ── Scan: document rules (SPEC §1.3) ────────────────────────
    spew(&dir.join("part1/ch2.md"), b"Another chapter here.\n");
    spew(&dir.join("empty.md"), b"");
    spew(&dir.join("blank.md"), b" \n\t\n");
    spew(&dir.join("binary.bin"), b"\xFF\xFE\x00\x01");
    std::os::unix::fs::symlink(dir.join("ch1.md"), dir.join("link.md")).unwrap();
    std::os::unix::fs::symlink(dir.join("part1"), dir.join("partlink")).unwrap();

    let scan = repo.scan().unwrap();
    assert_eq!(
        scan.manifest.keys().collect::<Vec<_>>(),
        vec![".litattributes", "ch1.md", "part1/ch2.md"],
        "documents only: no empty/blank/binary/symlink; attributes versioned"
    );
    assert_eq!(
        scan.modes.get("ch1.md").map(String::as_str),
        Some("prose"),
        "mode resolved from attributes"
    );

    // .litignore excludes; subdir .litignore applies to its subtree.
    spew(&dir.join("notes.md"), b"Scratch notes.\n");
    spew(&dir.join(".litignore"), b"notes.md\n");
    spew(&dir.join("part1/.litignore"), b"ch2.md\n");
    let scan2 = repo.scan().unwrap();
    assert!(
        !scan2.manifest.contains_key("notes.md"),
        ".litignore excludes at root"
    );
    assert!(
        !scan2.manifest.contains_key("part1/ch2.md"),
        "subdir .litignore applies"
    );
    assert!(
        scan2.manifest.contains_key(".litignore"),
        ".litignore itself is a document"
    );
    std::fs::remove_file(dir.join("part1/.litignore")).unwrap();
    std::fs::remove_file(dir.join("notes.md")).unwrap();

    // Byte-identical files at two paths: one document, two entries.
    spew(&dir.join("copy.md"), b"First sentence. Second one.\n");
    let scan3 = repo.scan().unwrap();
    assert_eq!(
        scan3.manifest.get("copy.md"),
        scan3.manifest.get("ch1.md"),
        "byte-identical files share one document identity (SPEC §5)"
    );
    assert_eq!(
        scan3.docs.len(),
        scan3.manifest.len() - 1,
        "one derivation for the shared document"
    );
    std::fs::remove_file(dir.join("copy.md")).unwrap();

    // ── Snapshot storage + tabula (SPEC §8.1) ───────────────────
    let scan4 = repo.scan().unwrap();
    let (trios, manifest) = repo.store_scan(&scan4).unwrap();
    let vo = VersionObject::new(&repo.segmentation_version(), trios, manifest).unwrap();
    let ch1 = scan4.manifest.get("ch1.md").unwrap();
    assert!(repo.theca().has("volumen", ch1).unwrap(), "volumen stored");
    let d = scan4.docs.get(ch1).unwrap();
    assert!(
        repo.theca().has("spina", &d.spina_hash).unwrap(),
        "spina stored"
    );
    assert!(
        repo.theca().has("atom-spina", &d.atom_spina_hash).unwrap(),
        "atom-spina stored"
    );

    assert!(repo.read_tabula().unwrap().is_none(), "tabula starts empty");
    repo.write_tabula(&vo).unwrap();
    let tab = repo.read_tabula().unwrap().unwrap();
    assert_eq!(
        tab.manifest(),
        vo.manifest(),
        "tabula round-trips the snapshot"
    );
    repo.clear_tabula().unwrap();
    assert!(repo.read_tabula().unwrap().is_none(), "tabula cleared");

    // ── Version-state assembly (SPEC §1.1 historical modes) ─────
    let vhash = vo.store(&repo.theca()).unwrap();
    let states = repo.doc_states(&vo).unwrap();
    assert_eq!(
        states.get("ch1.md").unwrap().mode,
        "prose",
        "historical mode resolution"
    );
    assert_eq!(
        states.get("ch1.md").unwrap().volumen,
        b"First sentence. Second one.\n",
        "volumen bytes from theca"
    );

    // ── Experiments (SPEC §9.2) ─────────────────────────────────
    assert!(Repository::valid_experiment_name("peira-1"), "good name");
    assert!(
        !Repository::valid_experiment_name("historia"),
        "historia name reserved"
    );
    assert!(
        !Repository::valid_experiment_name(&"0".repeat(64)),
        "64-hex name forbidden (SPEC §9.2)"
    );
    assert!(
        Repository::valid_experiment_name(&"A".repeat(64)),
        "64 chars is fine when not all lowercase hex"
    );
    assert!(
        !Repository::valid_experiment_name("with space"),
        "no whitespace"
    );
    assert!(!Repository::valid_experiment_name("a/b"), "no slash");

    let exp = repo.create_experiment("peira-1").unwrap();
    assert!(exp.is_empty().unwrap(), "fresh experiment empty");
    assert_eq!(repo.experiments(), vec!["peira-1"], "experiment listed");
    let e = repo.create_experiment("peira-1").unwrap_err();
    assert!(e.0.contains("already exists"), "duplicate creation refused");
    exp.append(&vhash, "trial", None).unwrap();
    assert_eq!(
        repo.experiment("peira-1")
            .unwrap()
            .unwrap()
            .latest()
            .unwrap()
            .unwrap()
            .vhash,
        vhash,
        "experiment appends"
    );
    repo.delete_experiment("peira-1").unwrap();
    assert!(repo.experiments().is_empty(), "experiment deleted");
    assert!(
        repo.theca().has("version", &vhash).unwrap(),
        "version objects survive experiment deletion (SPEC §9.2)"
    );

    // ── Segmentation guard (SPEC §4) ────────────────────────────
    spew(&dir.join(".lit/segmentation"), b"uax29-15.1.0\n");
    let e = repo.assert_segmentation().unwrap_err();
    assert!(
        e.0.contains("segmentation-version mismatch"),
        "mismatch refuses"
    );
    spew(
        &dir.join(".lit/segmentation"),
        format!("{}\n", literium::SEGMENTATION_VERSION).as_bytes(),
    );
}
