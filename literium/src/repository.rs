//! Repository discovery and creation (SPEC §1.3); the
//! working-tree scan (documents, ignore rules, symlinks); the
//! tabula (SPEC §8.1); and the segmentation-version guard (SPEC
//! §4). On-disk layout per PROVISIONAL.md #1:
//!
//! ```text
//! .lit/format         repository format marker ("1\n")
//! .lit/segmentation   segmentation-version record
//! .lit/historia       the historia file
//! .lit/experimenta/   experiment history files
//! .lit/tabula         the tabula (absent = empty)
//! .lit/theca/         the object store (SPEC §6.7)
//! ```

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::attributes::Attributes;
use crate::derive::{derive, Derivation};
use crate::error::Result;
use crate::history::History;
use crate::ignore::Ignore;
use crate::theca::{ObjectStore, Theca};
use crate::version_object::VersionObject;
use crate::{ATTRIBUTES_FILE, IGNORE_FILE};

#[derive(Debug)]
pub struct Scan {
    /// path → volumen hash
    pub manifest: BTreeMap<String, String>,
    /// volumen hash → derivation
    pub docs: HashMap<String, Derivation>,
    /// path → mode
    pub modes: BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct Repository {
    root: PathBuf,
}

impl Repository {
    // ── Discovery (SPEC §1.3) ───────────────────────────────────

    /// Absolute repo root, or `None` when no ancestor (inclusive)
    /// carries a `.lit/` directory.
    pub fn discover(dir: &Path) -> Option<PathBuf> {
        let mut abs = std::fs::canonicalize(dir).ok()?;
        loop {
            if abs.join(".lit").is_dir() {
                return Some(abs);
            }
            if !abs.pop() {
                return None;
            }
        }
    }

    /// Repo roots among PROPER descendants (SPEC §1.4 elevation
    /// dispatch). Symlinks are never followed.
    pub fn find_descendant_lits(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut queue = vec![dir.to_path_buf()];
        while !queue.is_empty() {
            let d = queue.remove(0);
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            names.sort();
            for path in names {
                if path
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(true)
                {
                    continue;
                }
                if !path.is_dir() {
                    continue;
                }
                if path.file_name().is_some_and(|n| n == ".lit") {
                    if d != dir {
                        found.push(d.clone()); // proper descendants only
                    }
                    continue;
                }
                queue.push(path);
            }
        }
        found
    }

    // ── Construction ────────────────────────────────────────────

    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.join(".lit").is_dir() {
            err!("not a literium repository: {}", root.display());
        }
        let repo = Repository { root };
        let format = repo.slurp_lit("format").unwrap_or_default();
        let format = String::from_utf8_lossy(&format);
        let format = format.trim_end_matches('\n');
        if format != crate::FORMAT_VERSION {
            err!("unsupported repository format '{format}'");
        }
        Ok(repo)
    }

    /// SPEC §1.3: creation is the first conscribe's business; the
    /// caller has already verified the tree contains documents.
    /// Writes `.lit/` and the empty historia together, and the
    /// attributes file before any version is recorded. Refused
    /// when the interpreter's tables do not match the SPEC §4 pin.
    pub fn create(root: &Path, mode: &str) -> Result<Self> {
        if mode != "prose" && mode != "poetry" {
            err!("mode must be 'prose' or 'poetry'");
        }
        if root.join(".lit").is_dir() {
            err!("refusing to create a repository inside one (SPEC §1.3)");
        }

        let here = crate::interpreter_segmentation_version();
        if here != crate::SEGMENTATION_VERSION {
            err!(
                "cannot create repository: this tooling segments with {here}, not {} (SPEC §4)",
                crate::SEGMENTATION_VERSION
            );
        }

        // SPEC §1.3: the attributes file is written before any
        // version is recorded. An existing one is the writer's.
        let attr_path = root.join(ATTRIBUTES_FILE);
        if !attr_path.exists() {
            std::fs::write(&attr_path, format!("* {mode}\n"))?;
        }

        std::fs::create_dir_all(root.join(".lit/theca"))?;
        std::fs::create_dir_all(root.join(".lit/experimenta"))?;
        std::fs::write(
            root.join(".lit/format"),
            format!("{}\n", crate::FORMAT_VERSION),
        )?;
        std::fs::write(
            root.join(".lit/segmentation"),
            format!("{}\n", crate::SEGMENTATION_VERSION),
        )?;
        std::fs::write(root.join(".lit/historia"), "")?; // empty historia (SPEC §9.1)

        Repository::new(root)
    }

    fn slurp_lit(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root.join(".lit").join(rel)).ok()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn theca(&self) -> Theca {
        Theca::new(self.root.join(".lit/theca"))
    }

    // ── Segmentation guard (SPEC §4) ────────────────────────────

    pub fn segmentation_version(&self) -> String {
        let rec = self.slurp_lit("segmentation").unwrap_or_default();
        String::from_utf8_lossy(&rec)
            .trim_end_matches('\n')
            .to_string()
    }

    /// Hash-producing operations MUST refuse on a
    /// segmentation-version mismatch rather than silently
    /// re-segment (SPEC §4).
    pub fn assert_segmentation(&self) -> Result<()> {
        let repo = self.segmentation_version();
        let here = crate::interpreter_segmentation_version();
        if repo != here {
            err!(
                "segmentation-version mismatch: repository records '{repo}', this tooling segments with '{here}'; refusing hash-producing operations (SPEC §4)"
            );
        }
        Ok(())
    }

    // ── Histories (SPEC §9) ─────────────────────────────────────

    pub fn historia(&self) -> History {
        History::new(self.root.join(".lit/historia"), "historia")
    }

    pub fn valid_experiment_name(name: &str) -> bool {
        if name.is_empty() || name == "historia" {
            return false; // SPEC §9.2
        }
        if name.len() == 64
            && name
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return false; // SPEC §9.2 / §8.3
        }
        if name.contains('/')
            || name.starts_with('.')
            || name.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return false;
        }
        true
    }

    pub fn experiment(&self, name: &str) -> Result<Option<History>> {
        if !Self::valid_experiment_name(name) {
            err!("invalid experiment name '{name}'");
        }
        let file = self.root.join(".lit/experimenta").join(name);
        if !file.exists() {
            return Ok(None);
        }
        Ok(Some(History::new(file, name)))
    }

    pub fn experiments(&self) -> Vec<String> {
        let dir = self.root.join(".lit/experimenta");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        names.sort();
        names
    }

    pub fn create_experiment(&self, name: &str) -> Result<History> {
        if !Self::valid_experiment_name(name) {
            err!("invalid experiment name '{name}'");
        }
        let file = self.root.join(".lit/experimenta").join(name);
        if file.exists() {
            err!("experiment '{name}' already exists");
        }
        std::fs::write(&file, "")?;
        Ok(History::new(file, name))
    }

    /// SPEC §9.2: deleting an experiment removes its
    /// inscriptiones; the version objects it named stay in the
    /// theca.
    pub fn delete_experiment(&self, name: &str) -> Result<()> {
        let Some(exp) = self.experiment(name)? else {
            err!("no experiment '{name}'");
        };
        std::fs::remove_file(exp.file()).map_err(|e| {
            crate::error::Error::new(format!("cannot delete experiment '{name}': {e}"))
        })
    }

    /// `History` for `historia` or an existing experiment; `None`
    /// when the name is invalid or absent.
    pub fn history(&self, name: &str) -> Option<History> {
        if name == "historia" {
            return Some(self.historia());
        }
        if !Self::valid_experiment_name(name) {
            return None;
        }
        self.experiment(name).ok().flatten()
    }

    /// Scan result for a directory that is NOT (yet) a repository:
    /// the pre-creation document check of SPEC §1.3. Honors any
    /// `.litignore` / attributes files already present in the tree.
    pub fn probe(dir: &Path) -> Result<Scan> {
        let probe = Repository {
            root: dir.to_path_buf(),
        };
        probe.scan()
    }

    // ── Working-tree scan (SPEC §1.3) ───────────────────────────

    /// `Attributes` from the working tree's attributes file (SPEC
    /// §1.1); empty mapping when absent.
    pub fn attributes(&self) -> Result<Attributes> {
        let path = self.root.join(ATTRIBUTES_FILE);
        let mut content = String::new();
        if path.is_file() && !is_symlink(&path) {
            let bytes = std::fs::read(&path).map_err(|e| {
                crate::error::Error::new(format!("cannot read {}: {e}", path.display()))
            })?;
            content = String::from_utf8(bytes)
                .map_err(|_| crate::error::Error::new("attributes file is not UTF-8"))?;
        }
        Attributes::parse(&content)
    }

    /// Documents only: valid UTF-8, non-empty content form;
    /// symlinks never followed; `.lit/` implicitly ignored;
    /// `.litignore` applied.
    pub fn scan(&self) -> Result<Scan> {
        let root = &self.root;
        let mut ignore = Ignore::new();
        let attrs = self.attributes()?;

        let mut manifest = BTreeMap::new();
        let mut docs = HashMap::new();
        let mut modes = BTreeMap::new();
        let mut dirs: Vec<String> = vec![String::new()]; // repo-relative, "" = root
        while !dirs.is_empty() {
            let rel_dir = dirs.remove(0);
            let abs_dir = if rel_dir.is_empty() {
                root.clone()
            } else {
                root.join(&rel_dir)
            };

            // A directory's .litignore applies to its subtree (§1.2).
            let ign_path = abs_dir.join(IGNORE_FILE);
            if ign_path.is_file() && !is_symlink(&ign_path) {
                let bytes = std::fs::read(&ign_path)?;
                let content = String::from_utf8(bytes)
                    .map_err(|_| crate::error::Error::new(".litignore is not UTF-8"))?;
                ignore.add_file(&rel_dir, &content);
            }

            let entries = std::fs::read_dir(&abs_dir).map_err(|e| {
                crate::error::Error::new(format!("cannot scan {}: {e}", abs_dir.display()))
            })?;
            let mut names: Vec<std::ffi::OsString> =
                entries.flatten().map(|e| e.file_name()).collect();
            names.sort();

            for name in names {
                if name == ".lit" {
                    continue; // always implicitly ignored (§1.2)
                }
                let abs = abs_dir.join(&name);
                if is_symlink(&abs) {
                    continue; // symlinks are not documents (§1.3)
                }
                let Some(name) = name.to_str() else {
                    continue; // undecodable filename: skip
                };
                let rel = if rel_dir.is_empty() {
                    name.to_string()
                } else {
                    format!("{rel_dir}/{name}")
                };

                if abs.is_dir() {
                    if ignore.ignored(&rel, true) {
                        continue;
                    }
                    dirs.push(rel);
                    continue;
                }
                if !abs.is_file() {
                    continue;
                }
                if ignore.ignored(&rel, false) {
                    continue;
                }

                let bytes = std::fs::read(&abs).map_err(|e| {
                    crate::error::Error::new(format!("cannot read {}: {e}", abs.display()))
                })?;
                let mode = attrs.mode_for(&rel).to_string();
                let Some(d) = derive(&bytes, &mode) else {
                    continue;
                };
                manifest.insert(rel.clone(), d.volumen_hash.clone());
                docs.entry(d.volumen_hash.clone()).or_insert(d);
                modes.insert(rel, mode);
            }
        }
        Ok(Scan {
            manifest,
            docs,
            modes,
        })
    }

    // ── Snapshot storage ────────────────────────────────────────

    /// Writes every derived object of every scanned document into
    /// the theca and returns the version-object fields for the
    /// snapshot: (trios, manifest).
    pub fn store_scan(
        &self,
        scan: &Scan,
    ) -> Result<(BTreeMap<String, (String, String)>, BTreeMap<String, String>)> {
        let theca = self.theca();
        let mut trios = BTreeMap::new();
        for (vh, d) in &scan.docs {
            theca.put("volumen", &d.volumen)?;
            for bytes in d.atoms.values() {
                theca.put("atom", bytes)?;
            }
            for bytes in d.fasciculi.values() {
                theca.put("fasciculus", bytes)?;
            }
            theca.put("spina", &d.spina)?;
            theca.put("atom-spina", &d.atom_spina)?;
            trios.insert(
                vh.clone(),
                (d.spina_hash.clone(), d.atom_spina_hash.clone()),
            );
        }
        Ok((trios, scan.manifest.clone()))
    }

    // ── Tabula (SPEC §8.1) ──────────────────────────────────────

    pub fn tabula_path(&self) -> PathBuf {
        self.root.join(".lit/tabula")
    }

    /// VersionObject-shaped record, or `None` when the tabula is
    /// empty. (The record is unhashed; the tabula is a scratchpad,
    /// not a stored version.)
    pub fn read_tabula(&self) -> Result<Option<VersionObject>> {
        let path = self.tabula_path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| crate::error::Error::new(format!("cannot read tabula: {e}")))?;
        Ok(Some(VersionObject::parse(&bytes)?))
    }

    /// SPEC §8.1: single slot, replace on write; the previous
    /// state is never persisted.
    pub fn write_tabula(&self, vo: &VersionObject) -> Result<()> {
        std::fs::write(self.tabula_path(), vo.serialize())?;
        Ok(())
    }

    // ── Contextio ledger (SPEC §9.6, §6.9) ──────────────────────

    pub fn contextio_path(&self) -> PathBuf {
        self.root.join(".lit/contextio")
    }

    pub fn read_contextio(&self) -> Result<Option<crate::merge::Ledger>> {
        let path = self.contextio_path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| crate::error::Error::new(format!("cannot read contextio: {e}")))?;
        Ok(Some(crate::merge::Ledger::parse(&bytes)?))
    }

    pub fn write_contextio(&self, ledger: &crate::merge::Ledger) -> Result<()> {
        std::fs::write(self.contextio_path(), ledger.serialize())?;
        Ok(())
    }

    pub fn clear_contextio(&self) -> Result<()> {
        let path = self.contextio_path();
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn clear_tabula(&self) -> Result<()> {
        let path = self.tabula_path();
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    // ── Version-state assembly ──────────────────────────────────

    /// `VersionObject` for a stored version (None when absent).
    pub fn version(&self, vhash: &str) -> Result<Option<VersionObject>> {
        VersionObject::load(&self.theca(), vhash)
    }

    /// `Attributes` resolved from the attributes document AS IT
    /// WAS in that version (SPEC §1.1).
    pub fn attributes_for_version(&self, vo: &VersionObject) -> Result<Attributes> {
        let mut content = String::new();
        if let Some(doc) = vo.document_at(ATTRIBUTES_FILE) {
            let Some(bytes) = self.theca().get("volumen", doc)? else {
                err!("attributes volumen {doc} missing from theca");
            };
            content = String::from_utf8(bytes)
                .map_err(|_| crate::error::Error::new("attributes volumen is not UTF-8"))?;
        }
        Attributes::parse(&content)
    }

    /// `path → derivation` with modes resolved from the version's
    /// own attributes document (SPEC §1.1). The derivation is
    /// recomputed from the volumen — deterministic, and fsck
    /// separately checks it equals the stored objects.
    pub fn doc_states(&self, vo: &VersionObject) -> Result<HashMap<String, Derivation>> {
        let attrs = self.attributes_for_version(vo)?;
        let theca = self.theca();
        let mut states = HashMap::new();
        for path in vo.paths() {
            let vh = vo.document_at(path).expect("path from manifest");
            let Some(bytes) = theca.get("volumen", vh)? else {
                err!("volumen {vh} missing from theca");
            };
            let mode = attrs.mode_for(path);
            let Some(d) = derive(&bytes, mode) else {
                err!("stored volumen {vh} does not derive as a document");
            };
            states.insert(path.clone(), d);
        }
        Ok(states)
    }
}

fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}
