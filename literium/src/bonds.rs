//! Display bonds (SPEC §7.4 *Bonds*; HISTORY §7 *Bonds: sentences
//! over atoms*).
//!
//! A `.desm` file (the syndesmos format) names the places where a
//! sentence runs on across a break UAX-29 made — after `Mr.`, across
//! an initial. Literium never lets it near segmentation: atoms,
//! hashes, and coordinates are what the untailored rules cut (SPEC
//! §4). The bonds act one layer up, at display time only: consecutive
//! prose atoms that a bond joins are shown as one **molecule** — one
//! line in `lit show --atoms`, one reading in the apparatus — with a
//! coordinate *range* (`g2a1..g2a2`) in place of the single atom's.
//!
//! Resolution: the `.desm` nearest to a document, walking up from
//! its directory to the repository root, applies to it; a
//! subdirectory's file overrides, and accumulates only by saying so
//! (`#!include ../.desm`). The working tree's files apply, even when
//! the version shown is historical — the reader chooses how to read.
//! `--desm <file>` names one file for every document; `--no-desm`
//! shows atoms.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use syndesmos::Syndesmos;

use crate::error::{Error, Result};
use crate::BONDS_FILE;

/// How a command chooses its bonds.
#[derive(Debug, Clone)]
pub enum Choice {
    /// The nearest `.desm` up the tree from each document.
    Discover,
    /// One file for every document.
    File(PathBuf),
    /// Atoms only.
    None,
}

/// A command's bond resolver: the choice plus a per-file cache.
pub struct Bonds {
    root: PathBuf,
    choice: Choice,
    cache: HashMap<PathBuf, Rc<Syndesmos>>,
}

impl Bonds {
    pub fn new(root: &Path, choice: Choice) -> Bonds {
        Bonds {
            root: root.to_path_buf(),
            choice,
            cache: HashMap::new(),
        }
    }

    /// `Discover` for the repository root.
    pub fn discover(root: &Path) -> Bonds {
        Bonds::new(root, Choice::Discover)
    }

    /// Atoms only.
    pub fn none(root: &Path) -> Bonds {
        Bonds::new(root, Choice::None)
    }

    /// The `.desm` file that applies to a repository-relative
    /// document path: the nearest one from the document's directory
    /// up to the root, or `None`. Directories that do not exist in
    /// the working tree (a document deleted since) are skipped.
    pub fn file_for(root: &Path, path: &str) -> Option<PathBuf> {
        let mut dir: Vec<&str> = path.split('/').collect();
        dir.pop(); // the file name
        loop {
            let mut candidate = root.to_path_buf();
            for d in &dir {
                candidate.push(d);
            }
            candidate.push(BONDS_FILE);
            if candidate.is_file() {
                return Some(candidate);
            }
            if dir.pop().is_none() {
                return None;
            }
        }
    }

    /// The bonds for a document, or `None` when no file applies (or
    /// the choice is `None`). A malformed file is an error naming it.
    pub fn for_path(&mut self, path: &str) -> Result<Option<Rc<Syndesmos>>> {
        let file = match &self.choice {
            Choice::None => return Ok(None),
            Choice::File(f) => f.clone(),
            Choice::Discover => match Bonds::file_for(&self.root, path) {
                Some(f) => f,
                None => return Ok(None),
            },
        };
        if let Some(s) = self.cache.get(&file) {
            return Ok(Some(Rc::clone(s)));
        }
        let loaded = Syndesmos::load(&file)
            .map_err(|e| Error::new(format!("{}: {e}", display_path(&self.root, &file))))?;
        let rc = Rc::new(loaded);
        self.cache.insert(file, Rc::clone(&rc));
        Ok(Some(rc))
    }

    /// The file that applies to a document, repository-relative, for
    /// reporting.
    pub fn file_name_for(&self, path: &str) -> Option<String> {
        match &self.choice {
            Choice::None => None,
            Choice::File(f) => Some(display_path(&self.root, f)),
            Choice::Discover => {
                Bonds::file_for(&self.root, path).map(|f| display_path(&self.root, &f))
            }
        }
    }
}

/// A path relative to the root when it lies under it, else as given.
fn display_path(root: &Path, file: &Path) -> String {
    match file.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => file.to_string_lossy().into_owned(),
    }
}

/// Parses and removes `--desm <file>` / `--no-desm` from an argument
/// list, returning the choice. Both given: the last wins.
pub fn choice_from_args(argv: &mut Vec<String>, cwd: &Path) -> Result<Choice> {
    let mut choice = Choice::Discover;
    let mut rest: Vec<String> = Vec::new();
    let mut it = std::mem::take(argv).into_iter();
    while let Some(item) = it.next() {
        if item == "--no-desm" {
            choice = Choice::None;
        } else if item == "--desm" {
            let Some(f) = it.next() else {
                return Err(Error::new("option --desm needs a value"));
            };
            choice = Choice::File(cwd.join(f));
        } else if let Some(f) = item.strip_prefix("--desm=") {
            choice = Choice::File(cwd.join(f));
        } else {
            rest.push(item);
        }
    }
    *argv = rest;
    Ok(choice)
}

/// The molecules of a grex: `(first, last)` atom index pairs,
/// inclusive, covering the atoms in order. Without bonds every atom
/// is its own molecule. The bonds are applied to the grex's
/// content-form join (atoms joined with one space — SPEC §3.4), with
/// each atom's start as a boundary: a boundary the bonds undo joins
/// the atom to its predecessor.
pub fn molecules(atoms: &[&str], bonds: Option<&Syndesmos>) -> Vec<(usize, usize)> {
    let n = atoms.len();
    if n == 0 {
        return Vec::new();
    }
    let bonds = match bonds {
        Some(b) if !b.is_empty() && n > 1 => b,
        _ => return (0..n).map(|i| (i, i)).collect(),
    };
    let mut text = String::new();
    let mut starts: Vec<usize> = Vec::with_capacity(n);
    for (i, a) in atoms.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        starts.push(text.len());
        text.push_str(a);
    }
    let standing = bonds.apply(&text, &starts[1..]);
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut first = 0usize;
    for (i, &s) in starts.iter().enumerate().skip(1) {
        if standing.contains(&s) {
            out.push((first, i - 1));
            first = i;
        }
    }
    out.push((first, n - 1));
    out
}

/// A molecule's text: its atoms' content forms joined with one
/// space (a one-atom molecule is the atom's content form).
pub fn molecule_text(atoms: &[&str], first: usize, last: usize) -> String {
    atoms[first..=last].join(" ")
}

/// The display coordinate of a molecule: `gNaM` for one atom,
/// `gNaM..gNaP` for a run (zero-based inputs, 1-based output).
pub fn coord_range(g0: usize, first0: usize, last0: usize) -> String {
    if first0 == last0 {
        format!("g{}a{}", g0 + 1, first0 + 1)
    } else {
        format!("g{}a{}..g{}a{}", g0 + 1, first0 + 1, g0 + 1, last0 + 1)
    }
}

/// The flat coordinate of a molecule: `aK` or `aK..aL` (1-based
/// inputs).
pub fn flat_range(first: usize, last: usize) -> String {
    if first == last {
        format!("a{first}")
    } else {
        format!("a{first}..a{last}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desm(text: &str) -> Syndesmos {
        Syndesmos::parse(text).unwrap()
    }

    #[test]
    fn no_bonds_is_one_atom_per_molecule() {
        let atoms = ["One.", "Two.", "Three."];
        assert_eq!(molecules(&atoms, None), vec![(0, 0), (1, 1), (2, 2)]);
        let empty = Syndesmos::empty();
        assert_eq!(
            molecules(&atoms, Some(&empty)),
            vec![(0, 0), (1, 1), (2, 2)]
        );
    }

    #[test]
    fn abbreviation_joins_two_atoms() {
        // UAX-29 cuts "Mr. Smith arrived." into "Mr." + "Smith arrived.".
        let atoms = ["Mr.", "Smith arrived.", "He sat down."];
        let b = desm("Mr.\n");
        assert_eq!(molecules(&atoms, Some(&b)), vec![(0, 1), (2, 2)]);
        assert_eq!(molecule_text(&atoms, 0, 1), "Mr. Smith arrived.");
    }

    #[test]
    fn pattern_straddling_the_join_bonds() {
        let atoms = ["J.", "R.", "Tolkien wrote.", "So did others."];
        let b = desm("/[A-Z]\\. [A-Z]/\n");
        assert_eq!(molecules(&atoms, Some(&b)), vec![(0, 2), (3, 3)]);
    }

    #[test]
    fn whole_word_check_holds_across_the_join() {
        let atoms = ["Undr.", "Smith arrived."];
        let b = desm("Dr.\n");
        assert_eq!(molecules(&atoms, Some(&b)), vec![(0, 0), (1, 1)]);
    }

    #[test]
    fn coordinates() {
        assert_eq!(coord_range(1, 0, 0), "g2a1");
        assert_eq!(coord_range(1, 0, 2), "g2a1..g2a3");
        assert_eq!(flat_range(4, 4), "a4");
        assert_eq!(flat_range(4, 6), "a4..a6");
    }

    #[test]
    fn nearest_file_wins_and_missing_dirs_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(BONDS_FILE), "Mr.\n").unwrap();
        std::fs::create_dir_all(root.join("part/ch")).unwrap();
        std::fs::write(root.join("part").join(BONDS_FILE), "Dr.\n").unwrap();
        assert_eq!(Bonds::file_for(root, "a.md"), Some(root.join(BONDS_FILE)));
        assert_eq!(
            Bonds::file_for(root, "part/ch/b.md"),
            Some(root.join("part").join(BONDS_FILE))
        );
        // A directory that no longer exists falls through to its
        // nearest existing ancestor's file.
        assert_eq!(
            Bonds::file_for(root, "gone/deep/c.md"),
            Some(root.join(BONDS_FILE))
        );
        let mut bonds = Bonds::discover(root);
        let s = bonds.for_path("part/ch/b.md").unwrap().unwrap();
        assert_eq!(s.abbreviations(), ["Dr."]);
        assert_eq!(
            bonds.file_name_for("part/ch/b.md").as_deref(),
            Some("part/.desm")
        );
        assert!(Bonds::none(root).for_path("a.md").unwrap().is_none());
    }

    #[test]
    fn a_bad_file_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(BONDS_FILE), "/unclosed\n").unwrap();
        let err = Bonds::discover(root).for_path("a.md").unwrap_err();
        assert!(err.0.starts_with(".desm: line 1:"), "{}", err.0);
    }

    #[test]
    fn args_are_parsed_and_removed() {
        let cwd = Path::new("/w");
        let mut argv: Vec<String> = ["historia", "--no-desm", "a.md"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(matches!(
            choice_from_args(&mut argv, cwd).unwrap(),
            Choice::None
        ));
        assert_eq!(argv, ["historia", "a.md"]);
        let mut argv: Vec<String> = ["--desm", "x.desm", "v1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        match choice_from_args(&mut argv, cwd).unwrap() {
            Choice::File(f) => assert_eq!(f, PathBuf::from("/w/x.desm")),
            other => panic!("{other:?}"),
        }
        assert_eq!(argv, ["v1"]);
        let mut argv: Vec<String> = vec!["--desm".to_string()];
        assert!(choice_from_args(&mut argv, cwd).is_err());
    }
}
