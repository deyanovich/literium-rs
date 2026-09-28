//! In-process exercise of `lit serve` (PROTOCOL §3–§7): the
//! session lifecycle + the read methods, driven through
//! serve()'s BufRead/Write seam with no subprocess.

use std::io::Cursor;

use literium::cli::Cli;

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
        let he = bytes[i..].windows(4).position(|w| w == b"\r\n\r\n").unwrap() + i;
        let hdr = std::str::from_utf8(&bytes[i..he]).unwrap();
        let len: usize = hdr.split(':').nth(1).unwrap().trim().parse().unwrap();
        let body = &bytes[he + 4..he + 4 + len];
        out.push(serde_json::from_slice(body).unwrap());
        i = he + 4 + len;
    }
    out
}

/// A throwaway repo with two versions, driven through the CLI.
fn scratch_repo() -> tempdir_lite::TempDir {
    let dir = tempdir_lite::TempDir::new();
    std::fs::write(dir.path().join("work.md"), b"One sentence. A second one.\n\nA paragraph.\n").unwrap();
    run_cli(&dir, &["add", "--mode", "prose", "--yes"]);
    run_cli(&dir, &["commit", "--all", "-m", "first draft"]);
    std::fs::write(dir.path().join("work.md"), b"One sentence, revised. A second one.\n\nA paragraph.\n").unwrap();
    run_cli(&dir, &["add"]);
    run_cli(&dir, &["commit", "--all", "-m", "revise"]);
    // Leave a pending edit on the tabula so status/diff have work.
    std::fs::write(dir.path().join("work.md"), b"One sentence, revised. A second one, changed.\n\nA paragraph.\n").unwrap();
    run_cli(&dir, &["add"]);
    dir
}

fn run_cli(dir: &tempdir_lite::TempDir, args: &[&str]) {
    let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut input = Cursor::new(Vec::new());
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut cli = Cli {
        cwd: dir.path().to_path_buf(),
        input: &mut input,
        out: &mut out,
        err: &mut err,
    };
    let rc = cli.run(&argv);
    assert_eq!(rc, 0, "cli {args:?} failed: {}", String::from_utf8_lossy(&err));
}

#[test]
fn lifecycle_and_read_methods() {
    let dir = scratch_repo();
    let root = dir.path().to_string_lossy().to_string();
    let reqs = vec![
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"workspaceRoot":root}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"log","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"show","params":{"version":"historia"}}),
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"status","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":5,"method":"diff","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":6,"method":"push","params":{}}),
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
    assert_eq!(responses.len(), 6); // ids 1..6

    let init = &responses[0]["result"];
    assert_eq!(init["protocolVersion"], "0.1");
    assert_eq!(init["engine"]["segmentationVersion"], "uax29-16.0.0");
    assert_eq!(init["repository"]["found"], true);

    let log = &responses[1]["result"];
    assert_eq!(log["history"], "historia");
    let ins = log["inscriptiones"].as_array().unwrap();
    assert_eq!(ins.len(), 2);
    assert_eq!(ins[0]["message"], "first draft");
    assert_eq!(ins[1]["ordinal"], 2);
    assert_eq!(ins[1]["version"].as_str().unwrap().len(), 64);

    let show = &responses[2]["result"];
    assert_eq!(show["hash"].as_str().unwrap().len(), 64);
    let docs = show["documents"].as_array().unwrap();
    assert!(docs.iter().any(|d| d["path"] == "work.md"));
    assert!(docs.iter().all(|d| d["volumen"].as_str().unwrap().len() == 64));

    // status: a pending edit is on the tabula.
    let status = &responses[3]["result"];
    assert_eq!(status["tabula"], true);
    assert!(!status["pending"].as_array().unwrap().is_empty());
    assert_eq!(status["pending"][0]["path"], "work.md");

    // diff: the tabula vs the historia tip, as typed changes in the
    // copyedit categories (a substitution of the changed sentence).
    let diff = &responses[4]["result"];
    let ddocs = diff["documents"].as_array().unwrap();
    assert!(!ddocs.is_empty());
    let changes = ddocs[0]["changes"].as_array().unwrap();
    let kinds: Vec<&str> = changes.iter().map(|c| c["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"substitution"), "diff kinds: {kinds:?}");

    // a still-unimplemented method -> method_not_found.
    assert_eq!(responses[5]["error"]["code"], -32601);
}

/// A minimal tempdir helper (no dev-dep needed).
mod tempdir_lite {
    use std::path::{Path, PathBuf};
    pub struct TempDir(PathBuf);
    impl TempDir {
        pub fn new() -> TempDir {
            let base = std::env::temp_dir();
            let mut p;
            let mut n = std::process::id() as u64;
            loop {
                n = n.wrapping_mul(6364136223846793005).wrapping_add(1);
                p = base.join(format!("lit-serve-test-{n:x}"));
                if std::fs::create_dir(&p).is_ok() {
                    break;
                }
            }
            TempDir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn write_round_trip() {
    // add (creates the repo) + commit, then an edit, add, commit —
    // entirely through the protocol; verified via log.
    let dir = tempdir_lite::TempDir::new();
    std::fs::write(dir.path().join("story.md"), b"One. Two.\n\nThree para.\n").unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let drive = |reqs: Vec<serde_json::Value>| -> Vec<serde_json::Value> {
        let mut input = Vec::new();
        for r in &reqs {
            input.extend_from_slice(&frame(r));
        }
        let mut out = Vec::new();
        assert_eq!(literium::serve::serve(&mut Cursor::new(input), &mut out), 0);
        parse_all(&out)
    };

    let r = drive(vec![
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"workspaceRoot":root}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"add","params":{"mode":"prose"}}),
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"commit","params":{"message":"first"}}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(r[1]["result"]["created"], true);
    assert_eq!(r[2]["result"]["inscriptiones"][0]["message"], "first");
    assert_eq!(r[2]["result"]["inscriptiones"][0]["version"].as_str().unwrap().len(), 64);

    std::fs::write(dir.path().join("story.md"), b"One, changed. Two.\n\nThree para.\n").unwrap();
    let r = drive(vec![
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"workspaceRoot":root}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"add","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"commit","params":{"message":"second"}}),
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"log","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(r[1]["result"]["created"], false);
    let msgs: Vec<&str> = r[3]["result"]["inscriptiones"]
        .as_array().unwrap().iter().map(|i| i["message"].as_str().unwrap()).collect();
    assert_eq!(msgs, vec!["first", "second"]);

    // commit with no message -> invalid_params (never opens an editor).
    let r = drive(vec![
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"workspaceRoot":root}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"commit","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit"}),
    ]);
    assert_eq!(r[1]["error"]["code"], -32602);
}
