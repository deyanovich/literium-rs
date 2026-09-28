// Conformance-oracle differential test (HISTORY §2 "Implementation
// language: Rust core, Perl oracle").
//
// Runs literium-perl's Literium::Derive over every fixture file in
// tests/fixtures/, in both modes, and asserts this crate derives
// byte-identical objects: volumen/spina/atom-spina hashes, the
// full sorted atom and fasciculus hash sets, and the content form.
//
// Skips (with a note) when the sibling literium-perl checkout or a
// perl with UCD 16.0.0 is unavailable — CI environments without
// the oracle still run the unit suites.

use std::path::{Path, PathBuf};
use std::process::Command;

use literium::derive::derive;

fn perl_lib() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .join("literium-perl/lib");
    dir.join("Literium/Derive.pm").exists().then_some(dir)
}

fn perl_oracle_available(lib: &Path) -> bool {
    Command::new("perl")
        .arg(format!("-I{}", lib.display()))
        .args(["-MUnicode::UCD", "-e"])
        .arg("exit(Unicode::UCD::UnicodeVersion() eq '16.0.0' ? 0 : 1)")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// One line per field, "NOT_A_DOCUMENT" for the undef case; object
// hash sets sorted and comma-joined; content form hex-encoded.
const ORACLE_PROG: &str = r#"
use strict; use warnings;
use Literium::Derive qw(derive);
my ($path, $mode) = @ARGV;
open my $fh, '<:raw', $path or die $!;
local $/; my $bytes = <$fh>;
my $d = derive(bytes => $bytes, mode => $mode);
if (!$d) { print "NOT_A_DOCUMENT\n"; exit 0 }
print "$d->{volumen_hash}\n$d->{spina_hash}\n$d->{atom_spina_hash}\n";
print join(',', sort keys %{$d->{atoms}}), "\n";
print join(',', sort keys %{$d->{fasciculi}}), "\n";
print unpack('H*', $d->{content_form}), "\n";
"#;

fn rust_summary(bytes: &[u8], mode: &str) -> String {
    match derive(bytes, mode) {
        None => "NOT_A_DOCUMENT\n".to_string(),
        Some(d) => {
            let mut atoms: Vec<&String> = d.atoms.keys().collect();
            atoms.sort();
            let mut fascs: Vec<&String> = d.fasciculi.keys().collect();
            fascs.sort();
            format!(
                "{}\n{}\n{}\n{}\n{}\n{}\n",
                d.volumen_hash,
                d.spina_hash,
                d.atom_spina_hash,
                atoms
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                fascs
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                hex::encode(&d.content_form),
            )
        }
    }
}

#[test]
fn edge_class_matches_perl_oracle() {
    let Some(lib) = perl_lib() else {
        eprintln!("skipped: sibling literium-perl checkout not found");
        return;
    };
    if !perl_oracle_available(&lib) {
        eprintln!("skipped: no perl with UCD 16.0.0 on PATH");
        return;
    }
    // Perl side: every code point matching the SPEC §4.3 property
    // class. Rust side: every code point edge_strip removes.
    let out = Command::new("perl")
        .arg("-e")
        .arg(
            r#"my $e = qr/[\p{Sentence_Break=Sp}\p{Sentence_Break=Sep}\p{Sentence_Break=CR}\p{Sentence_Break=LF}]/;
for my $cp (0..0x10FFFF) {
  next if $cp >= 0xD800 && $cp <= 0xDFFF;
  print "$cp\n" if chr($cp) =~ $e;
}"#,
        )
        .output()
        .expect("run perl");
    assert!(out.status.success());
    let perl_class: Vec<u32> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| l.parse().unwrap())
        .collect();
    // A code point is in the class iff a string of just that char
    // strips to empty.
    let rust_class: Vec<u32> = (0..=0x10FFFF)
        .filter(|cp| !(0xD800..=0xDFFF).contains(cp))
        .filter(|&cp| {
            let c = char::from_u32(cp).unwrap();
            literium::segment::edge_strip(&c.to_string()).is_empty()
        })
        .collect();
    assert_eq!(rust_class, perl_class, "SPEC §4.3 edge class diverges");
    eprintln!("edge class: {} code points identical", perl_class.len());
}

#[test]
fn derivation_matches_perl_oracle() {
    let Some(lib) = perl_lib() else {
        eprintln!("skipped: sibling literium-perl checkout not found");
        return;
    };
    if !perl_oracle_available(&lib) {
        eprintln!("skipped: no perl with UCD 16.0.0 on PATH");
        return;
    }

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&fixtures)
        .expect("fixtures dir")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty(), "no fixtures found");

    let mut checked = 0;
    for path in &entries {
        let bytes = std::fs::read(path).expect("read fixture");
        for mode in ["prose", "poetry"] {
            let out = Command::new("perl")
                .arg(format!("-I{}", lib.display()))
                .arg("-e")
                .arg(ORACLE_PROG)
                .arg(path)
                .arg(mode)
                .output()
                .expect("run perl oracle");
            assert!(
                out.status.success(),
                "oracle failed on {} ({mode}): {}",
                path.display(),
                String::from_utf8_lossy(&out.stderr)
            );
            let perl = String::from_utf8(out.stdout).expect("oracle output");
            let rust = rust_summary(&bytes, mode);
            assert_eq!(
                rust,
                perl,
                "derivation diverges from the Perl oracle on {} ({mode})",
                path.display()
            );
            checked += 1;
        }
    }
    eprintln!("oracle: {checked} (file, mode) derivations byte-identical");
}
