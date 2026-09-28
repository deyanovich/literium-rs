//! The exemplar exchange (SPEC §9.7): collaboration as
//! correspondence. An exemplar is a gzip-compressed tar archive
//! of a version's documents plus a three-line `.exemplar`
//! metadata file — a format marker, the **basis** (the version
//! the exemplar was cut from; the diff target on return), and
//! the segmentation identifier. A convenience form only: plain
//! `tar` is the equivalent in both directions, and no part of
//! an archive enters any hash pre-image.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use crate::error::Error;

pub const METADATA_NAME: &str = ".exemplar";

pub struct Metadata {
    pub basis: String,
    pub segmentatio: String,
}

impl Metadata {
    pub fn serialize(&self) -> String {
        format!(
            "exemplar 1\nbasis {}\nsegmentatio {}\n",
            self.basis, self.segmentatio
        )
    }

    /// Parse the `.exemplar` file: `<key> SP <value>` lines.
    /// Unknown keys are ignored (a future format may add some);
    /// the marker, a well-formed basis, and the segmentation
    /// identifier are required.
    pub fn parse(text: &str) -> Result<Metadata, Error> {
        let mut marker = false;
        let mut basis: Option<String> = None;
        let mut segmentatio: Option<String> = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once(' ') else {
                continue;
            };
            match key {
                "exemplar" => {
                    if value != "1" {
                        err!("unsupported exemplar format '{value}'");
                    }
                    marker = true;
                }
                "basis" => basis = Some(value.to_string()),
                "segmentatio" => segmentatio = Some(value.to_string()),
                _ => {}
            }
        }
        if !marker {
            err!("not an exemplar archive (no format marker)");
        }
        let basis = basis.unwrap_or_default();
        if basis.len() != 64 || !basis.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            err!("exemplar basis is not a version hash");
        }
        let segmentatio = segmentatio.unwrap_or_default();
        if segmentatio.is_empty() {
            err!("exemplar carries no segmentation identifier");
        }
        Ok(Metadata { basis, segmentatio })
    }
}

/// Entry-path discipline for reading foreign archives: strip a
/// leading `./`, reject absolute paths and `..` components (a
/// mailed archive is untrusted bytes).
fn entry_path(raw: &str) -> Result<Option<String>, Error> {
    let path = raw.strip_prefix("./").unwrap_or(raw);
    if path.is_empty() || path.ends_with('/') {
        return Ok(None); // directory entries carry no bytes
    }
    if path.starts_with('/') || path.split('/').any(|seg| seg == ".." || seg.is_empty()) {
        err!("unsafe path in archive: {raw}");
    }
    Ok(Some(path.to_string()))
}

/// Write an archive: metadata first, then the documents in the
/// given order, all with epoch timestamps (the archive is a
/// convenience form; reproducibility is a courtesy).
pub fn write_archive(
    out: &Path,
    metadata: &Metadata,
    docs: &[(String, Vec<u8>)],
) -> Result<(), Error> {
    let file = std::fs::File::create(out)
        .map_err(|e| Error::new(format!("cannot create {}: {e}", out.display())))?;
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);

    let mut append = |path: &str, bytes: &[u8]| -> Result<(), Error> {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_cksum();
        tar.append_data(&mut header, path, bytes)
            .map_err(|e| Error::new(format!("archive write ({path}): {e}")))
    };

    append(METADATA_NAME, metadata.serialize().as_bytes())?;
    for (path, bytes) in docs {
        append(path, bytes)?;
    }
    tar.into_inner()
        .and_then(|gz| gz.finish())
        .map_err(|e| Error::new(format!("archive finish: {e}")))?;
    Ok(())
}

/// Read an archive: the metadata plus the document entries
/// (`.exemplar` itself is consumed here and never returned as a
/// document). Duplicate paths keep the last occurrence, matching
/// tar extraction semantics.
pub fn read_archive(file: &Path) -> Result<(Metadata, Vec<(String, Vec<u8>)>), Error> {
    let handle = std::fs::File::open(file)
        .map_err(|e| Error::new(format!("cannot open {}: {e}", file.display())))?;
    let gz = flate2::read::GzDecoder::new(handle);
    let mut tar = tar::Archive::new(gz);

    let mut metadata: Option<Metadata> = None;
    let mut docs: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for entry in tar
        .entries()
        .map_err(|e| Error::new(format!("not a tar.gz archive: {e}")))?
    {
        let mut entry = entry.map_err(|e| Error::new(format!("archive read: {e}")))?;
        let raw = entry
            .path()
            .map_err(|e| Error::new(format!("archive entry path: {e}")))?
            .to_string_lossy()
            .into_owned();
        let Some(path) = entry_path(&raw)? else {
            continue;
        };
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| Error::new(format!("archive read ({path}): {e}")))?;
        if path == METADATA_NAME {
            metadata = Some(Metadata::parse(&String::from_utf8_lossy(&bytes))?);
            continue;
        }
        if !seen.insert(path.clone()) {
            docs.retain(|(p, _)| p != &path);
        }
        docs.push((path, bytes));
    }
    let Some(metadata) = metadata else {
        err!("not an exemplar archive (no {METADATA_NAME} entry)");
    };
    Ok((metadata, docs))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASIS: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn metadata() -> Metadata {
        Metadata {
            basis: BASIS.into(),
            segmentatio: "uax29-v1".into(),
        }
    }

    #[test]
    fn metadata_roundtrip_and_rejects() {
        let text = metadata().serialize();
        assert_eq!(text, format!("exemplar 1\nbasis {BASIS}\nsegmentatio uax29-v1\n"));
        let back = Metadata::parse(&text).unwrap();
        assert_eq!(back.basis, BASIS);
        assert_eq!(back.segmentatio, "uax29-v1");

        assert!(Metadata::parse("basis abc\n").is_err()); // no marker
        assert!(Metadata::parse("exemplar 2\n").is_err()); // bad format
        assert!(Metadata::parse(&format!("exemplar 1\nbasis {}\n", "z".repeat(64))).is_err());
        assert!(Metadata::parse(&format!("exemplar 1\nbasis {BASIS}\n")).is_err()); // no segmentatio
        // Unknown keys ignored.
        let text = format!("exemplar 1\nfutur um\nbasis {BASIS}\nsegmentatio s\n");
        assert!(Metadata::parse(&text).is_ok());
    }

    #[test]
    fn archive_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lit-exemplar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("x.tgz");
        let docs = vec![
            ("chapter-one.md".to_string(), b"A sentence.\n".to_vec()),
            ("notes/plan.md".to_string(), b"Another.\n".to_vec()),
        ];
        write_archive(&out, &metadata(), &docs).unwrap();
        let (meta, back) = read_archive(&out).unwrap();
        assert_eq!(meta.basis, BASIS);
        assert_eq!(back, docs);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entry_paths_are_disciplined() {
        assert_eq!(entry_path("./a.md").unwrap().as_deref(), Some("a.md"));
        assert_eq!(entry_path("dir/").unwrap(), None);
        assert!(entry_path("/etc/passwd").is_err());
        assert!(entry_path("a/../b").is_err());
    }
}
