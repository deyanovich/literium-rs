//! The local content-addressed object store (SPEC §6.7).
//!
//! Stores the six object types at `<type>/<aa>/<hash>` with the
//! two-character sharding prefix of SPEC §6.7. `version` objects
//! use [`ObjectStore::put_at`] — their address is the version hash
//! over the hashed pre-image, and their manifest region is the
//! theca's sole mutable-in-place region. Object framing is
//! PROVISIONAL.md #3: none — a stored file's content is exactly
//! the object's raw bytes.
//!
//! [`ObjectStore`] is the storage trait boundary where syntheca
//! cellae attach at the litorium/v2 remoting seam (HISTORY §2
//! *Implementation language*); [`Theca`] is the SPEC §6.7 local
//! implementation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::derive::hash_bytes;
use crate::error::Result;

pub const TYPES: [&str; 6] = [
    "atom",
    "fasciculus",
    "volumen",
    "spina",
    "atom-spina",
    "version",
];

fn check_type(type_: &str) -> Result<()> {
    if !TYPES.contains(&type_) {
        err!("theca: unknown object type '{type_}'");
    }
    Ok(())
}

fn check_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        err!("theca: malformed hash '{hash}'");
    }
    Ok(())
}

/// The storage seam. v1 has one implementation ([`Theca`]);
/// syntheca-cella-backed stores attach here at v2.
pub trait ObjectStore {
    /// Content-addressed write; a no-op when the object already
    /// exists. Not for `version` objects (SPEC §6.7 carve-out).
    fn put(&self, type_: &str, bytes: &[u8]) -> Result<String>;

    /// Keyed write for the version-object carve-out (SPEC §6.7):
    /// the address is the version hash (digest of the hashed
    /// pre-image), not of the full bytes. `overwrite` serves the
    /// manifest region's mutate-in-place operations (elevation,
    /// rename re-inscription).
    fn put_at(&self, type_: &str, hash: &str, bytes: &[u8], overwrite: bool) -> Result<()>;

    fn get(&self, type_: &str, hash: &str) -> Result<Option<Vec<u8>>>;
    fn has(&self, type_: &str, hash: &str) -> Result<bool>;

    /// All stored hashes of one type, unordered.
    fn list(&self, type_: &str) -> Result<Vec<String>>;
}

pub struct Theca {
    root: PathBuf,
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Theca {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Theca { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, type_: &str, hash: &str) -> Result<PathBuf> {
        check_hash(hash)?;
        Ok(self.root.join(type_).join(&hash[..2]).join(hash))
    }

    fn write(&self, type_: &str, hash: &str, bytes: &[u8]) -> Result<()> {
        let path = self.path(type_, hash)?;
        let dir = path.parent().expect("object path has a parent");
        if !dir.is_dir() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Removes one stored object (the gc sweep's primitive,
    /// SPEC §6.10). Not part of [`ObjectStore`]: deletion is a
    /// local-storage concern, not part of the v2 cella seam.
    pub fn delete(&self, type_: &str, hash: &str) -> Result<()> {
        check_type(type_)?;
        let path = self.path(type_, hash)?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}

impl ObjectStore for Theca {
    fn put(&self, type_: &str, bytes: &[u8]) -> Result<String> {
        check_type(type_)?;
        if type_ == "version" {
            err!("theca: version objects use put_at (SPEC §6.7 carve-out)");
        }
        let hash = hash_bytes(bytes);
        if !self.has(type_, &hash)? {
            self.write(type_, &hash, bytes)?;
        }
        Ok(hash)
    }

    fn put_at(&self, type_: &str, hash: &str, bytes: &[u8], overwrite: bool) -> Result<()> {
        check_type(type_)?;
        if self.has(type_, hash)? && !overwrite {
            return Ok(());
        }
        self.write(type_, hash, bytes)
    }

    fn get(&self, type_: &str, hash: &str) -> Result<Option<Vec<u8>>> {
        check_type(type_)?;
        let path = self.path(type_, hash)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(_) => Ok(None),
        }
    }

    fn has(&self, type_: &str, hash: &str) -> Result<bool> {
        check_type(type_)?;
        Ok(self.path(type_, hash)?.is_file())
    }

    fn list(&self, type_: &str) -> Result<Vec<String>> {
        check_type(type_)?;
        let dir = self.root.join(type_);
        let mut hashes = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(hashes);
        };
        for aa in entries.flatten() {
            let name = aa.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.len() != 2
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                continue;
            }
            let Ok(sub) = std::fs::read_dir(aa.path()) else {
                continue;
            };
            for f in sub.flatten() {
                if let Some(h) = f.file_name().to_str() {
                    if check_hash(h).is_ok() {
                        hashes.push(h.to_string());
                    }
                }
            }
        }
        Ok(hashes)
    }
}
