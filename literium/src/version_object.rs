//! Version objects (SPEC §8).
//!
//! A version object carries one identity trio per document plus
//! the manifest binding working-tree paths to document identities.
//! The version hash covers the trios and identity-bearing metadata
//! only; two version objects with equal hash may differ in their
//! manifests (SPEC §8). No parent-version reference exists —
//! history order lives in histories (SPEC §9). Serialization is
//! provisional (PROVISIONAL.md #4):
//!
//! ```text
//! lit-version 1
//! segmentation <id>
//! trio <volumen> <spina> <atom-spina>       (sorted by volumen)
//! manifest
//! <volumen-hash> <encoded path>             (sorted by path)
//! ```

use std::collections::BTreeMap;

use crate::derive::hash_bytes;
use crate::error::Result;
use crate::theca::ObjectStore;

fn is_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Debug, Clone)]
pub struct VersionObject {
    segmentation: String,
    /// volumen hash → (spina hash, atom-spina hash); BTreeMap
    /// keeps the serialization's sorted-by-volumen order.
    trios: BTreeMap<String, (String, String)>,
    /// path → volumen hash; BTreeMap keeps sorted-by-path order.
    manifest: BTreeMap<String, String>,
}

impl VersionObject {
    pub fn new(
        segmentation: &str,
        trios: BTreeMap<String, (String, String)>,
        manifest: BTreeMap<String, String>,
    ) -> Result<Self> {
        let vo = VersionObject {
            segmentation: segmentation.to_string(),
            trios,
            manifest,
        };
        vo.validate()?;
        Ok(vo)
    }

    fn validate(&self) -> Result<()> {
        for (v, (s, a)) in &self.trios {
            if !is_hash(v) || !is_hash(s) || !is_hash(a) {
                err!("VersionObject: malformed trio for '{v}'");
            }
        }
        let mut covered: BTreeMap<&str, bool> = BTreeMap::new();
        for (path, v) in &self.manifest {
            if !self.trios.contains_key(v) {
                err!("VersionObject: manifest path '{path}' names unknown document");
            }
            covered.insert(v, true);
        }
        // Every document must sit at some path; SPEC §8 allows
        // zero trios only for the removal of a repository's last
        // documents.
        for v in self.trios.keys() {
            if !covered.contains_key(v.as_str()) {
                err!("VersionObject: document {v} has no manifest path");
            }
        }
        Ok(())
    }

    pub fn segmentation(&self) -> &str {
        &self.segmentation
    }

    pub fn trios(&self) -> &BTreeMap<String, (String, String)> {
        &self.trios
    }

    pub fn manifest(&self) -> &BTreeMap<String, String> {
        &self.manifest
    }

    /// Document identities (volumen hashes), sorted.
    pub fn documents(&self) -> impl Iterator<Item = &String> {
        self.trios.keys()
    }

    /// Manifest paths, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.manifest.keys()
    }

    /// `[volumen, spina, atom-spina]` for one document.
    pub fn trio_for(&self, volumen: &str) -> Option<[&str; 3]> {
        self.trios
            .get_key_value(volumen)
            .map(|(k, (s, a))| [k.as_str(), s.as_str(), a.as_str()])
    }

    pub fn document_at(&self, path: &str) -> Option<&str> {
        self.manifest.get(path).map(String::as_str)
    }

    // ── Serialization ───────────────────────────────────────────

    /// The hashed pre-image (SPEC §8): identity trios +
    /// identity-bearing metadata, manifest excluded.
    pub fn preimage(&self) -> String {
        let mut out = String::from("lit-version 1\n");
        out.push_str(&format!("segmentation {}\n", self.segmentation));
        for (v, (s, a)) in &self.trios {
            out.push_str(&format!("trio {v} {s} {a}\n"));
        }
        out
    }

    pub fn vhash(&self) -> String {
        hash_bytes(self.preimage().as_bytes())
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = self.preimage();
        out.push_str("manifest\n");
        for (path, v) in &self.manifest {
            out.push_str(&format!("{v} {}\n", encode_path(path)));
        }
        out.into_bytes()
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| crate::error::Error::new("VersionObject: not UTF-8"))?;
        let mut lines: Vec<&str> = text.split('\n').collect();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        let mut it = lines.into_iter();

        let head = it.next().unwrap_or("");
        if head != "lit-version 1" {
            err!("VersionObject: bad header '{head}'");
        }
        let seg_line = it.next().unwrap_or("");
        let Some(seg) = seg_line.strip_prefix("segmentation ") else {
            err!("VersionObject: missing segmentation line");
        };

        let mut trios = BTreeMap::new();
        let mut manifest = BTreeMap::new();
        let mut in_manifest = false;
        for line in it {
            if !in_manifest {
                if line == "manifest" {
                    in_manifest = true;
                    continue;
                }
                let parts: Vec<&str> = line.split(' ').collect();
                if parts.len() != 4
                    || parts[0] != "trio"
                    || !is_hash(parts[1])
                    || !is_hash(parts[2])
                    || !is_hash(parts[3])
                {
                    err!("VersionObject: bad trio line '{line}'");
                }
                trios.insert(
                    parts[1].to_string(),
                    (parts[2].to_string(), parts[3].to_string()),
                );
                continue;
            }
            let (v, p) = match line.split_once(' ') {
                Some((v, p)) if is_hash(v) => (v, p),
                _ => err!("VersionObject: bad manifest line '{line}'"),
            };
            manifest.insert(decode_path(p)?, v.to_string());
        }
        if !in_manifest {
            err!("VersionObject: missing manifest separator");
        }
        VersionObject::new(seg, trios, manifest)
    }

    // ── Theca binding (SPEC §6.7 carve-out) ─────────────────────

    pub fn store(&self, theca: &dyn ObjectStore) -> Result<String> {
        let vhash = self.vhash();
        theca.put_at("version", &vhash, &self.serialize(), false)?;
        Ok(vhash)
    }

    pub fn load(theca: &dyn ObjectStore, vhash: &str) -> Result<Option<Self>> {
        let Some(bytes) = theca.get("version", vhash)? else {
            return Ok(None);
        };
        let vo = VersionObject::parse(&bytes)?;
        if vo.vhash() != vhash {
            err!("VersionObject: stored object {vhash} fails its pre-image hash");
        }
        Ok(Some(vo))
    }

    /// The manifest region's sanctioned in-place mutation (SPEC
    /// §8: elevation, project merge, rename re-inscription). The
    /// version hash is unchanged by construction.
    pub fn rewrite_manifest(
        &mut self,
        theca: &dyn ObjectStore,
        new_manifest: BTreeMap<String, String>,
    ) -> Result<()> {
        let old = std::mem::replace(&mut self.manifest, new_manifest);
        if self.validate().is_err() {
            self.manifest = old;
            err!("rewrite_manifest: manifest does not cover the trios");
        }
        theca.put_at("version", &self.vhash(), &self.serialize(), true)
    }
}

/// Paths are UTF-8 with `%`, LF, CR percent-encoded
/// (PROVISIONAL.md #4).
fn encode_path(path: &str) -> String {
    path.replace('%', "%25")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

fn decode_path(b: &str) -> Result<String> {
    Ok(b.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%25", "%"))
}
