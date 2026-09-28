//! Display bonds (SPEC §7.4 *Bonds*; HISTORY §7 *Bonds: sentences
//! over atoms*): a `.desm` file joins atoms into molecules in
//! `lit show`, the apparatus, and the protocol; identity and the
//! `--unified` conformance form stay atomic.

use std::io::Cursor;
use std::path::Path;

use literium::cli::Cli;

fn lit(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut input = Cursor::new(Vec::new());
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut cli = Cli {
        cwd: dir.to_path_buf(),
        input: &mut input,
        out: &mut out,
        err: &mut err,
    };
    let rc = cli.run(&argv);
    (
        rc,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let (rc, out, err) = lit(dir, args);
    assert_eq!(rc, 0, "lit {args:?} failed: {err}");
    out
}

/// A repository with a root `.desm` (Mr.), a subdirectory `.desm`
/// (Dr.) that overrides it, and one document under each.
fn scratch() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join(".desm"), "Mr.\n").unwrap();
    std::fs::write(root.join("a.md"), "Mr. Smith arrived. He sat down.\n").unwrap();
    std::fs::create_dir(root.join("part")).unwrap();
    std::fs::write(root.join("part/.desm"), "Dr.\n").unwrap();
    std::fs::write(
        root.join("part/b.md"),
        "Mr. Smith arrived. Dr. Jones left.\n",
    )
    .unwrap();
    ok(root, &["add", "--mode", "prose", "--yes"]);
    ok(root, &["commit", "--all", "-m", "first"]);
    dir
}

#[test]
fn show_atoms_joins_molecules_under_the_nearest_desm() {
    let dir = scratch();
    let root = dir.path();

    // The root .desm bonds "Mr." to the atom after it.
    assert_eq!(
        ok(root, &["show", "historia", "a.md", "--atoms"]),
        "a1..a2 g1a1..g1a2  Mr. Smith arrived.\na3 g1a3  He sat down.\n"
    );
    // --no-desm: the atoms themselves.
    assert_eq!(
        ok(root, &["show", "historia", "a.md", "--atoms", "--no-desm"]),
        "a1 g1a1  Mr.\na2 g1a2  Smith arrived.\na3 g1a3  He sat down.\n"
    );
    // The subdirectory's .desm overrides the root's: Dr. bonds, Mr.
    // does not.
    assert_eq!(
        ok(root, &["show", "historia", "part/b.md", "--atoms"]),
        "a1 g1a1  Mr.\na2 g1a2  Smith arrived.\na3..a4 g1a3..g1a4  Dr. Jones left.\n"
    );
    // --desm names one file for every document.
    assert_eq!(
        ok(
            root,
            &[
                "show",
                "historia",
                "part/b.md",
                "--atoms",
                "--desm",
                ".desm"
            ]
        ),
        "a1..a2 g1a1..g1a2  Mr. Smith arrived.\na3 g1a3  Dr.\na4 g1a4  Jones left.\n"
    );
    // gN prints the grex's molecules; gNaM and aK still resolve atoms.
    assert_eq!(
        ok(root, &["show", "historia", "a.md", "g1"]),
        "Mr. Smith arrived.\nHe sat down.\n"
    );
    assert_eq!(
        ok(root, &["show", "historia", "a.md", "g1a2"]),
        "Smith arrived.\n"
    );
    assert_eq!(ok(root, &["show", "historia", "a.md", "a1"]), "Mr.\n");
    // Identity is untouched: the atom hash is the atom's.
    let h = ok(root, &["show", "historia", "a.md", "g1a1", "--hash"]);
    assert_eq!(h.trim().len(), 64);
    assert_eq!(
        h,
        ok(
            root,
            &["show", "historia", "a.md", "g1a1", "--hash", "--no-desm"]
        )
    );
}

#[test]
fn apparatus_shows_molecules_and_unified_stays_atomic() {
    let dir = scratch();
    let root = dir.path();
    std::fs::write(root.join("a.md"), "Mr. Smith departed. He sat down.\n").unwrap();
    ok(root, &["add"]);

    let apparatus = ok(root, &["diff"]);
    assert_eq!(
        apparatus,
        "a.md\n  g1a1..g1a2  substitution\n      - Mr. Smith arrived.\n      + Mr. Smith departed.\n"
    );
    let atomic = ok(root, &["diff", "--no-desm"]);
    assert_eq!(
        atomic,
        "a.md\n  g1a2  substitution\n      - Smith arrived.\n      + Smith departed.\n"
    );
    // --unified is the conformance grammar: never molecules.
    let unified = ok(root, &["diff", "--unified"]);
    assert!(unified.contains("- g1a2  Smith arrived.\n"), "{unified}");
    assert!(unified.contains("+ g1a2  Smith departed.\n"), "{unified}");
    assert!(!unified.contains(".."), "{unified}");
}

#[test]
fn bonds_apply_to_prose_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join(".desm"), "Mr.\n").unwrap();
    std::fs::write(root.join(".litattributes"), "*.txt poetry\n").unwrap();
    std::fs::write(root.join("v.txt"), "Mr.\nSmith arrived.\n").unwrap();
    ok(root, &["add", "--mode", "prose", "--yes"]);
    ok(root, &["commit", "--all", "-m", "first"]);
    assert_eq!(
        ok(root, &["show", "historia", "v.txt", "--atoms"]),
        "a1 g1a1  Mr.\na2 g1a2  Smith arrived.\n"
    );
}

#[test]
fn a_malformed_desm_is_an_error_naming_it() {
    let dir = scratch();
    let root = dir.path();
    std::fs::write(root.join(".desm"), "/unclosed\n").unwrap();
    let (rc, _, err) = lit(root, &["show", "historia", "a.md", "--atoms"]);
    assert_eq!(rc, 1);
    assert!(err.contains(".desm: line 1:"), "{err}");
    // Atoms are still reachable.
    ok(root, &["show", "historia", "a.md", "--atoms", "--no-desm"]);
}

// ── lit serve ───────────────────────────────────────────────────

fn frame(v: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(v).unwrap();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

fn parse_all(bytes: &[u8]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let he = bytes[i..]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap()
            + i;
        let hdr = std::str::from_utf8(&bytes[i..he]).unwrap();
        let len: usize = hdr.split(':').nth(1).unwrap().trim().parse().unwrap();
        let body = &bytes[he + 4..he + 4 + len];
        out.push(serde_json::from_slice(body).unwrap());
        i = he + 4 + len;
    }
    out
}

#[test]
fn serve_show_and_diff_carry_molecules() {
    let dir = scratch();
    let root = dir.path();
    std::fs::write(root.join("a.md"), "Mr. Smith departed. He sat down.\n").unwrap();
    ok(root, &["add"]);

    let ws = root.to_string_lossy().to_string();
    let reqs = vec![
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"workspaceRoot":ws}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"show","params":{"version":"historia","path":"a.md","atoms":true}}),
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"diff","params":{"path":"a.md"}}),
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"diff","params":{"path":"a.md","bonds":false}}),
        serde_json::json!({"jsonrpc":"2.0","id":5,"method":"show","params":{"version":"historia","path":"part/b.md","atoms":true,"desm":".desm"}}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit"}),
    ];
    let mut input: Vec<u8> = Vec::new();
    for r in &reqs {
        input.extend_from_slice(&frame(r));
    }
    let mut reader = Cursor::new(input);
    let mut out: Vec<u8> = Vec::new();
    let rc = literium::serve::serve(&mut reader, &mut out);
    assert_eq!(rc, 0);
    let responses = parse_all(&out);
    let by_id = |id: u64| {
        responses
            .iter()
            .find(|r| r["id"] == serde_json::json!(id))
            .unwrap_or_else(|| panic!("no response {id}"))
    };

    assert_eq!(
        by_id(1)["result"]["capabilities"]["bonds"],
        serde_json::json!(true)
    );

    let show = &by_id(2)["result"];
    assert_eq!(show["mode"], "prose");
    assert_eq!(show["atoms"].as_array().unwrap().len(), 3);
    assert_eq!(show["atoms"][0]["coordinate"], "g1a1");
    assert_eq!(show["atoms"][0]["flat"], "a1");
    assert_eq!(show["atoms"][0]["text"], "Mr.");
    assert_eq!(show["bonds"], ".desm");
    let molecules = show["molecules"].as_array().unwrap();
    assert_eq!(molecules.len(), 2);
    assert_eq!(molecules[0]["coordinate"], "g1a1..g1a2");
    assert_eq!(molecules[0]["flat"], "a1..a2");
    assert_eq!(molecules[0]["text"], "Mr. Smith arrived.");
    assert_eq!(molecules[1]["coordinate"], "g1a3");

    let diff = &by_id(3)["result"]["documents"][0];
    assert_eq!(diff["bonds"], ".desm");
    assert_eq!(diff["changes"][0]["coord"], "g1a1..g1a2");
    assert_eq!(diff["changes"][0]["old"], "Mr. Smith arrived.");
    assert_eq!(diff["changes"][0]["new"], "Mr. Smith departed.");

    let atomic = &by_id(4)["result"]["documents"][0];
    assert!(atomic.get("bonds").is_none(), "{atomic}");
    assert_eq!(atomic["changes"][0]["coord"], "g1a2");

    // `desm` names one file, repository-relative.
    let b = &by_id(5)["result"];
    assert_eq!(b["molecules"][0]["coordinate"], "g1a1..g1a2");
    assert_eq!(b["bonds"], ".desm");
}
