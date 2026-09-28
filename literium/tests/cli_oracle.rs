// CLI-level conformance-oracle differential test (HISTORY §2
// "Implementation language: Rust core, Perl oracle").
//
// Drives byte-identical command sequences through this crate's
// `lit` binary and literium-perl's `bin/lit`, each in a fresh
// working tree, then asserts:
//
//   - identical exit statuses per step;
//   - identical stdout/stderr per step (timestamps and the two
//     absolute roots masked);
//   - byte-identical `.lit/` trees at the end: every theca object
//     (hash AND bytes), the tabula, format and segmentation
//     records, and every history file with the timestamp column
//     masked.
//
// Skips with a note when the sibling literium-perl checkout or a
// perl with UCD 16.0.0 is unavailable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

enum Step {
    /// Write a file (creating parent dirs).
    File(&'static str, &'static [u8]),
    /// Remove a file.
    Rm(&'static str),
    /// Run `lit <args>` from the tree root, expecting this exit
    /// status.
    Cmd(&'static [&'static str], i32),
    /// Run `lit <args>` from a subdirectory of the tree.
    CmdAt(&'static str, &'static [&'static str], i32),
    /// Run `lit <args>` with the given stdin.
    CmdIn(&'static [&'static str], &'static str, i32),
}

use Step::{Cmd, CmdAt, CmdIn, File, Rm};

fn perl_lit() -> Option<(PathBuf, PathBuf)> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .join("literium-perl");
    let lib = base.join("lib");
    let bin = base.join("bin/lit");
    (lib.join("Literium/CLI.pm").exists() && bin.exists()).then_some((lib, bin))
}

fn perl_available(lib: &Path) -> bool {
    Command::new("perl")
        .arg(format!("-I{}", lib.display()))
        .args(["-MUnicode::UCD", "-e"])
        .arg("exit(Unicode::UCD::UnicodeVersion() eq '16.0.0' ? 0 : 1)")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const TS_MASK: &str = "<TS>";

fn mask(text: &str, root: &Path) -> String {
    let ts = regex::Regex::new(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6}Z").unwrap();
    let text = ts.replace_all(text, TS_MASK);
    text.replace(&root.display().to_string(), "<ROOT>")
}

/// Runs one scenario in a fresh tree with the given command
/// runner; returns the masked per-step transcripts and the root.
fn run_scenario(
    steps: &[Step],
    run: &dyn Fn(&Path, &[&str], &str) -> (i32, String, String),
) -> (Vec<String>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let mut transcript = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        match step {
            File(path, bytes) => {
                let p = root.join(path);
                if let Some(dir) = p.parent() {
                    std::fs::create_dir_all(dir).unwrap();
                }
                std::fs::write(&p, bytes).unwrap();
            }
            Rm(path) => {
                std::fs::remove_file(root.join(path)).unwrap();
            }
            Cmd(args, expect) | CmdAt(_, args, expect) | CmdIn(args, _, expect) => {
                let cwd = match step {
                    CmdAt(sub, _, _) => root.join(sub),
                    _ => root.clone(),
                };
                let stdin = match step {
                    CmdIn(_, input, _) => *input,
                    _ => "",
                };
                let (status, out, err) = run(&cwd, args, stdin);
                assert_eq!(
                    status,
                    *expect,
                    "step {i} `lit {}`: expected exit {expect}, got {status}\nstdout:\n{out}\nstderr:\n{err}",
                    args.join(" ")
                );
                transcript.push(format!(
                    "== lit {} -> {status}\n{}--\n{}",
                    args.join(" "),
                    mask(&out, &root),
                    mask(&err, &root)
                ));
            }
        }
    }
    (transcript, tmp)
}

/// Collects `.lit/` as relpath → bytes, with history timestamps
/// masked.
fn collect_lit(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut map = BTreeMap::new();
    let lit = root.join(".lit");
    let mut queue = vec![lit.clone()];
    while let Some(dir) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                queue.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(&lit)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let mut bytes = std::fs::read(&path).unwrap();
            if rel == "historia" || rel.starts_with("experimenta/") {
                let text = String::from_utf8(bytes).expect("history is UTF-8");
                bytes = text
                    .split('\n')
                    .map(|line| {
                        let mut parts: Vec<&str> = line.splitn(3, '\t').collect();
                        if parts.len() == 3 {
                            parts[1] = TS_MASK;
                        }
                        parts.join("\t")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into_bytes();
            }
            map.insert(rel, bytes);
        }
    }
    map
}

fn compare_scenario(name: &str, steps: &[Step]) {
    let Some((lib, bin)) = perl_lit() else {
        eprintln!("skipped: sibling literium-perl checkout not found");
        return;
    };
    if !perl_available(&lib) {
        eprintln!("skipped: no perl with UCD 16.0.0 on PATH");
        return;
    }

    let rust_bin = env!("CARGO_BIN_EXE_lit");
    let run_rust = move |root: &Path, args: &[&str], stdin: &str| -> (i32, String, String) {
        use std::io::Write;
        let mut child = Command::new(rust_bin)
            .args(args)
            .current_dir(root)
            .env_remove("EDITOR")
            .env_remove("VISUAL")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run rust lit");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(stdin.as_bytes())
            .ok();
        let out = child.wait_with_output().expect("run rust lit");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let run_perl = move |root: &Path, args: &[&str], stdin: &str| -> (i32, String, String) {
        use std::io::Write;
        let mut child = Command::new("perl")
            .arg(format!("-I{}", lib.display()))
            .arg(&bin)
            .args(args)
            .current_dir(root)
            .env_remove("EDITOR")
            .env_remove("VISUAL")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run perl lit");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(stdin.as_bytes())
            .ok();
        let out = child.wait_with_output().expect("run perl lit");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    let (rust_transcript, rust_tmp) = run_scenario(steps, &run_rust);
    let (perl_transcript, perl_tmp) = run_scenario(steps, &run_perl);

    for (i, (r, p)) in rust_transcript
        .iter()
        .zip(perl_transcript.iter())
        .enumerate()
    {
        assert_eq!(r, p, "{name}: transcript diverges at command {i}");
    }

    let rust_lit = collect_lit(&std::fs::canonicalize(rust_tmp.path()).unwrap());
    let perl_lit_tree = collect_lit(&std::fs::canonicalize(perl_tmp.path()).unwrap());
    let rust_keys: Vec<&String> = rust_lit.keys().collect();
    let perl_keys: Vec<&String> = perl_lit_tree.keys().collect();
    assert_eq!(rust_keys, perl_keys, "{name}: .lit file sets differ");
    for (rel, bytes) in &rust_lit {
        assert_eq!(
            bytes,
            perl_lit_tree.get(rel).unwrap(),
            "{name}: .lit/{rel} differs between implementations"
        );
    }
    eprintln!(
        "{name}: {} commands + {} .lit files byte-identical",
        rust_transcript.len(),
        rust_lit.len()
    );
}

#[test]
fn prose_workflow() {
    compare_scenario(
        "prose workflow",
        &[
            Cmd(&["conscribe", "--mode", "prose"], 1), // no documents yet
            File(
                "ch1.md",
                b"One sentence. Two sentences.\n\nSecond paragraph.\n",
            ),
            File("notes/ch2.md", b"Another chapter starts. It continues.\n"),
            Cmd(&["conscribe", "--mode", "prose"], 0),
            Cmd(&["status"], 0),
            Cmd(&["inscribe", "--all", "-m", "first version"], 0),
            Cmd(&["conscribe"], 1), // no pending changes
            File(
                "ch1.md",
                b"One sentence EDITED. Two sentences.\n\nSecond paragraph.\n\nThird para new.\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["status"], 0),
            Cmd(&["inscribe", "-1", "-m", "edit first sentence"], 0),
            Cmd(&["status"], 0),
            Cmd(&["inscribe", "--all", "-m", "the rest"], 0),
            Cmd(&["show"], 0),
            Cmd(&["fsck"], 0),
            Cmd(&["diff", "--unified"], 0),
        ],
    );
}

#[test]
fn structural_formatting_and_tabula() {
    compare_scenario(
        "structural/formatting/tabula",
        &[
            File("a.md", b"One. Two. Three.\n"),
            Cmd(&["conscribe", "--mode", "prose"], 0),
            Cmd(&["inscribe", "--all", "-m", "base"], 0),
            // Pure split (structural) + formatting elsewhere.
            File("a.md", b"One. Two.\n\nThree.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["status"], 0),
            Cmd(&["tabula"], 0),
            Cmd(&["inscribe", "-m", "split the paragraph"], 0),
            // Formatting-only change.
            File("a.md", b"One.  Two.\n\nThree.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["status"], 0),
            Cmd(&["tabula", "discard"], 0),
            Cmd(&["tabula"], 0),
            Cmd(&["status"], 0),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "--all", "-m", "double space"], 0),
            Cmd(&["fsck"], 0),
        ],
    );
}

#[test]
fn experiments_fons_exscribe() {
    compare_scenario(
        "experiments/fons/exscribe",
        &[
            File("a.md", b"Base text here.\n"),
            Cmd(&["conscribe", "--mode", "prose"], 0),
            Cmd(&["inscribe", "--all", "-m", "base"], 0),
            Cmd(&["experiment", "create", "peira-1"], 0),
            Cmd(&["experiment", "list"], 0),
            File("a.md", b"Experimental text here.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "peira-1", "--all", "-m", "trial"], 0),
            Cmd(&["show", "peira-1"], 0),
            // Adopt the experiment's tip into historia.
            Cmd(&["inscribe", "--fons", "peira-1", "-m", "adopt trial"], 0),
            Cmd(&["show"], 0),
            // Restore the base version into the tree.
            Cmd(&["exscribe", "historia", "--force"], 0),
            Cmd(&["diff", "--unified"], 0),
            // Hex-shaped experiment name resolves as a name.
            Cmd(&["experiment", "create", "deadbeef"], 0),
            Cmd(
                &["inscribe", "deadbeef", "--fons", "historia", "-m", "adopt"],
                0,
            ),
            Cmd(&["show", "deadbeef"], 0),
            Cmd(&["experiment", "delete", "deadbeef"], 0),
            Cmd(&["fsck"], 0),
        ],
    );
}

#[test]
fn rename_removal_zero_trio() {
    compare_scenario(
        "rename/removal/zero-trio",
        &[
            File("old.md", b"Same text here.\n"),
            File("other.md", b"Other text.\n"),
            Cmd(&["conscribe", "--mode", "prose"], 0),
            Cmd(&["inscribe", "--all", "-m", "base"], 0),
            // Pure rename: same bytes, new path.
            Rm("old.md"),
            File("new.md", b"Same text here.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["status"], 0),
            Cmd(&["inscribe", "--all", "-m", "moved"], 0),
            Cmd(&["show"], 0),
            // Removal.
            Rm("other.md"),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "--all", "-m", "dropped other"], 0),
            // Deleting the last documents: zero-trio version.
            Rm("new.md"),
            Rm(".litattributes"),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "--all", "-m", "everything removed"], 0),
            Cmd(&["conscribe"], 1), // no pending changes
            Cmd(&["fsck"], 0),
        ],
    );
}

#[test]
fn poetry_and_ignore() {
    compare_scenario(
        "poetry/ignore",
        &[
            File("poem.txt", b"first line\nsecond line\n\nnew stanza\n"),
            File("drafts/scratch.txt", b"Ignore me. Please.\n"),
            File(".litignore", b"drafts/\n"),
            Cmd(&["conscribe", "--mode", "poetry"], 0),
            Cmd(&["status"], 0),
            Cmd(&["inscribe", "--all", "-m", "first poem"], 0),
            // Line edit + new stanza line.
            File(
                "poem.txt",
                b"first line\nsecond line changed\n\nnew stanza\nanother line\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["status"], 0),
            Cmd(&["inscribe", "-1", "-m", "second line"], 0),
            Cmd(&["inscribe", "--all", "-m", "extend stanza"], 0),
            Cmd(&["diff", "--unified"], 0),
            Cmd(&["fsck"], 0),
        ],
    );
}

#[test]
fn elevation() {
    compare_scenario(
        "elevation",
        &[
            // Nested repo: create and inscribe inside novella/.
            File("novella/ch1.md", b"Nested chapter. It exists.\n"),
            File("novella/.litignore", b"*.tmp\nsub/anchored.md\n"),
            File("novella/verse.txt", b"a line of verse\nanother one\n"),
            CmdAt("novella", &["conscribe", "--mode", "prose"], 0),
            CmdAt("novella", &["inscribe", "--all", "-m", "nested base"], 0),
            // Elevate from the parent (one descendant, --yes).
            File("top.md", b"Top-level text. More of it.\n"),
            Cmd(&["conscribe", "--mode", "prose", "--yes"], 0),
            Cmd(&["status"], 0),
            Cmd(&["inscribe", "--all", "-m", "elevated tree"], 0),
            Cmd(&["show"], 0),
            Cmd(&["fsck"], 0),
        ],
    );
}

#[test]
fn diff_and_read_surface() {
    compare_scenario(
        "diff/read surface",
        &[
            File(
                "ch1.md",
                b"Alpha one. Alpha two.\n\nBeta one. Beta two. Beta three.\n\nGamma one.\n",
            ),
            Cmd(&["conscribe", "--mode", "prose"], 0),
            Cmd(&["inscribe", "--all", "-m", "base"], 0),
            // Move across grexes (Alpha two -> Gamma), an exact
            // split (Beta two -> two sentences), a modify, and a
            // formatting touch elsewhere.
            File(
                "ch1.md",
                b"Alpha one.\n\nBeta one. Beta. Two. Beta three CHANGED.\n\nGamma one. Alpha two.\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["status"], 0),
            Cmd(&["diff", "--unified"], 0),
            Cmd(&["inscribe", "--all", "-m", "rework"], 0),
            // Structural regroup: pure split of a paragraph.
            File(
                "ch1.md",
                b"Alpha one.\n\nBeta one. Beta. Two.\n\nBeta three CHANGED.\n\nGamma one. Alpha two.\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["diff", "--unified"], 0),
            Cmd(&["inscribe", "--all", "-m", "split beta"], 0),
            // Formatting-only rewrap.
            File(
                "ch1.md",
                b"Alpha one.\n\nBeta one. Beta.\nTwo.\n\nBeta three CHANGED.\n\nGamma one. Alpha two.\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["diff", "--unified"], 0),
            Cmd(&["tabula", "discard"], 0),
            // Read surface.
            Cmd(&["show", "historia"], 0),
            Cmd(&["show", "historia", "ch1.md", "--atoms"], 0),
            Cmd(&["show", "historia", "ch1.md", "g2a2"], 0),
            Cmd(&["show", "historia", "ch1.md", "a5"], 0),
            Cmd(&["show", "--hash", "historia", "ch1.md", "g1a1"], 0),
            Cmd(&["show", "--hash", "historia", "ch1.md", "g2"], 0),
            Cmd(&["show", "historia", "ch1.md", "g9"], 1),
            Cmd(&["show", "historia", "ch1.md", "g1a9"], 1),
            Cmd(&["show", "historia", "ch1.md", "a99"], 1),
            Cmd(&["show", "historia", "ch1.md", "x7"], 1),
            // Poetry merge-annotation check: two lines merged into
            // one (join with LF fails; identical-content join with
            // space fails too in poetry — instead verify a prose
            // atom merge).
            File("m.md", b"First half. Second half.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "--all", "-m", "add m"], 0),
            File("m.md", b"First half. Second half. Wait here.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["diff", "--unified"], 0),
            Cmd(&["tabula", "discard"], 0),
            File("m.md", b"First half.  Second half.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["diff", "--unified"], 0),
            Cmd(&["inscribe", "--all", "-m", "spaced"], 0),
            // Poetry line split and merge: exact single-space-join
            // detection (SPEC \u{a7}7.4).
            File(".litattributes", b"p.txt poetry\n* prose\n"),
            File("p.txt", b"a long verse line here\nsecond line\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "--all", "-m", "poem"], 0),
            File("p.txt", b"a long verse\nline here\nsecond line\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["diff", "--unified"], 0),
            Cmd(&["inscribe", "--all", "-m", "line split"], 0),
            File("p.txt", b"a long verse line here second line\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["diff", "--unified"], 0),
            Cmd(&["tabula", "discard"], 0),
            Cmd(&["fsck"], 0),
        ],
    );
}

#[test]
fn contexe_and_conflicts() {
    compare_scenario(
        "contexe/conflicts",
        &[
            File(
                "ch1.md",
                b"Alpha one. Alpha two.\n\nBeta one. Beta two.\n\nGamma one.\n",
            ),
            Cmd(&["conscribe", "--mode", "prose"], 0),
            Cmd(&["inscribe", "--all", "-m", "base"], 0),
            // Seed the experiment with the base (the common
            // version the auto-base discovers).
            Cmd(&["experiment", "create", "editor"], 0),
            Cmd(&["inscribe", "editor", "--fons", "historia", "-m", "seed"], 0),
            // Editor's pass: change Beta, add notes.md.
            File(
                "ch1.md",
                b"Alpha one. Alpha two.\n\nBeta one EDITED. Beta two.\n\nGamma one.\n",
            ),
            File("notes.md", b"Notes from the editor.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "editor", "--all", "-m", "editor pass"], 0),
            // Author continues from the base on historia.
            Cmd(&["exscribe", "historia", "--force"], 0),
            File(
                "ch1.md",
                b"Alpha one. Alpha two.\n\nBeta one. Beta two.\n\nGamma one CHANGED. Gamma extra.\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "--all", "-m", "author pass"], 0),
            // Clean combining merge: both sides compose, no
            // conflict; notes.md arrives.
            Cmd(&["contexe", "editor"], 0),
            Cmd(&["conflicts"], 0),
            Cmd(&["conscribe"], 0),
            Cmd(&["status"], 0),
            Cmd(&["inscribe", "--all", "-m", "merged"], 0),
            Cmd(&["fsck"], 0),
            // A real conflict: both sides rewrite the same atom.
            Cmd(&["experiment", "create", "rival"], 0),
            Cmd(&["inscribe", "rival", "--fons", "historia", "-m", "seed"], 0),
            File(
                "ch1.md",
                b"Alpha one. Alpha two.\n\nBeta one RIVAL. Beta two.\n\nGamma one CHANGED. Gamma extra.\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "rival", "--all", "-m", "rival beta"], 0),
            Cmd(&["exscribe", "historia", "--force"], 0),
            File(
                "ch1.md",
                b"Alpha one. Alpha two.\n\nBeta one MINE. Beta two.\n\nGamma one CHANGED. Gamma extra.\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "--all", "-m", "my beta"], 0),
            Cmd(&["contexe", "rival"], 0),
            Cmd(&["conflicts"], 0),
            Cmd(&["status"], 0), // ledger warning + empty tabula
            Cmd(&["fsck"], 0),
            // Resolve by ordinary editing; inscribe prompts (y).
            File(
                "ch1.md",
                b"Alpha one. Alpha two.\n\nBeta one RESOLVED. Beta two.\n\nGamma one CHANGED. Gamma extra.\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["status"], 0),
            CmdIn(&["inscribe", "--all", "-m", "resolved"], "y\n", 0),
            Cmd(&["conflicts"], 0),
            // Declining the prompt refuses.
            Cmd(&["contexe", "rival", "--force"], 0),
            Cmd(&["conscribe"], 0),
            CmdIn(&["inscribe", "--all", "-m", "again"], "n\n", 1),
            Cmd(&["conflicts", "clear"], 0),
            Cmd(&["conflicts"], 0),
            CmdIn(&["inscribe", "--all", "-m", "again"], "", 0),
            // --base override + modify/delete conflict.
            Cmd(&["experiment", "create", "cutter"], 0),
            Cmd(&["inscribe", "cutter", "--fons", "historia", "-m", "seed"], 0),
            Rm("notes.md"),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "cutter", "--all", "-m", "cut notes"], 0),
            Cmd(&["exscribe", "historia", "--force"], 0),
            File("notes.md", b"Notes from the editor, expanded now.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "--all", "-m", "expand notes"], 0),
            Cmd(&["contexe", "cutter"], 0),
            Cmd(&["conflicts"], 0),
            Cmd(&["conflicts", "clear"], 0),
            Cmd(&["fsck"], 0),
        ],
    );
}

#[test]
fn stemma_provenance() {
    compare_scenario(
        "stemma",
        &[
            File("a.md", b"One base sentence. Another one.\n"),
            Cmd(&["conscribe", "--mode", "prose"], 0),
            Cmd(&["inscribe", "--all", "-m", "base"], 0),
            Cmd(&["experiment", "create", "peira-1"], 0),
            Cmd(
                &["inscribe", "peira-1", "--fons", "historia", "-m", "seed"],
                0,
            ),
            File("a.md", b"One base sentence. Another one. A third joins.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "peira-1", "--all", "-m", "third sentence"], 0),
            Cmd(&["inscribe", "--fons", "peira-1", "-m", "adopt third"], 0),
            // Version life-paths: default (historia tip), the base
            // version (origin in historia, seeded into peira-1),
            // and the adopted version (origin in peira-1).
            Cmd(&["stemma"], 0),
            Cmd(&["stemma", "historia"], 0),
            // Atom provenance: the new sentence (first in the
            // experiment) and a base sentence (present everywhere).
            Cmd(&["stemma", "historia", "a.md", "g1a3"], 0),
            Cmd(&["stemma", "historia", "a.md", "a1"], 0),
            // Errors.
            Cmd(&["stemma", "historia", "a.md", "g9a1"], 1),
            Cmd(&["stemma", "historia", "a.md", "g1"], 1),
            Cmd(&["stemma", "historia", "a.md"], 1),
            // Deleting the experiment shifts the apparent origin
            // (accepted §9.2 behavior, now observable).
            Cmd(&["experiment", "delete", "peira-1"], 0),
            Cmd(&["stemma"], 0),
            Cmd(&["stemma", "historia", "a.md", "g1a3"], 0),
            Cmd(&["fsck"], 0),
        ],
    );
}

#[test]
fn gc_collection() {
    compare_scenario(
        "gc",
        &[
            File("a.md", b"Kept sentence one. Kept two.\n"),
            Cmd(&["conscribe", "--mode", "prose"], 0),
            Cmd(&["inscribe", "--all", "-m", "base"], 0),
            // Tabula churn: two conscribed snapshots never
            // inscribed (the larger garbage source in daily use).
            File("a.md", b"Kept sentence one. Kept two. Churn A unique.\n"),
            Cmd(&["conscribe"], 0),
            File("a.md", b"Kept sentence one. Kept two. Churn B unique.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["tabula", "discard"], 0),
            File("a.md", b"Kept sentence one. Kept two.\n"),
            // Orphaned experiment: a unique draft sharing one atom
            // with the surviving text.
            Cmd(&["experiment", "create", "doomed"], 0),
            Cmd(
                &["inscribe", "doomed", "--fons", "historia", "-m", "seed"],
                0,
            ),
            File(
                "a.md",
                b"Kept sentence one. Doomed unique draft sentence.\n",
            ),
            Cmd(&["conscribe"], 0),
            Cmd(&["inscribe", "doomed", "--all", "-m", "draft"], 0),
            Cmd(&["exscribe", "historia", "--force"], 0),
            Cmd(&["experiment", "delete", "doomed"], 0),
            // Preview, collect, idempotence.
            Cmd(&["gc", "--dry-run"], 0),
            Cmd(&["gc"], 0),
            Cmd(&["gc"], 0),
            // Everything reachable is untouched.
            Cmd(&["fsck"], 0),
            Cmd(&["stemma"], 0),
            Cmd(&["show", "historia", "a.md", "--atoms"], 0),
            // A live tabula is a gc root: conscribe then gc must
            // not eat the pending snapshot.
            File("a.md", b"Kept sentence one. Kept two. Pending three.\n"),
            Cmd(&["conscribe"], 0),
            Cmd(&["gc"], 0),
            Cmd(&["status"], 0),
            Cmd(&["inscribe", "--all", "-m", "pending survives"], 0),
            Cmd(&["fsck"], 0),
            Cmd(&["gc", "--dry-run"], 0),
        ],
    );
}

#[test]
fn git_peer_aliases() {
    // The git-English register is byte-identical to the Latin
    // handlers across both engines (HISTORY §15): a whole
    // workflow driven by git verbs must produce the same
    // transcripts and .lit trees as its conscribe/inscribe form.
    compare_scenario(
        "git peer aliases",
        &[
            File("ch.md", b"One sentence. Two sentences.\n\nSecond paragraph.\n"),
            Cmd(&["add", "--mode", "prose"], 0), // = conscribe
            Cmd(&["status"], 0),
            Cmd(&["commit", "--all", "-m", "first"], 0), // = inscribe
            Cmd(&["log", "historia"], 0),                // = show
            Cmd(&["branch", "create", "editor-pass"], 0), // = experiment
            Cmd(&["branch", "list"], 0),
            File("ch.md", b"One sentence EDITED. Two sentences.\n\nSecond paragraph.\n"),
            Cmd(&["add"], 0),
            Cmd(&["commit", "--all", "-m", "edit"], 0),
            Cmd(&["log"], 0),
            Cmd(&["fsck"], 0),
            // A deliberately-absent git verb: the guidance hint is
            // identical on both engines.
            Cmd(&["rebase"], 1),
            Cmd(&["revert"], 1),
        ],
    );
}
