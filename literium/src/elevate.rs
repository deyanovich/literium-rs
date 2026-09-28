//! Project elevation (SPEC §1.4).
//!
//! A conscribe from an ancestor directory (outside any repository)
//! whose subtree contains exactly one descendant repository moves
//! that project's root up to the invocation directory. Manifest
//! rewrites are in place; version hashes are unchanged (SPEC §8).
//! One-way. The multi-descendant case (project merge) is not
//! implemented in this version.

use std::path::Path;

use crate::cli::Cli;
use crate::error::{Error, Result};
use crate::repository::Repository;
use crate::theca::ObjectStore;
use crate::version_object::VersionObject;
use crate::{ATTRIBUTES_FILE, IGNORE_FILE};

/// Returns the elevated repository (rooted at the CLI's cwd), or
/// `None` when the user declines.
pub fn elevate(cli: &mut Cli, descendant: &Path, yes: bool) -> Result<Option<Repository>> {
    let abs_new = std::fs::canonicalize(&cli.cwd)
        .map_err(|_| Error::new(format!("bad root {}", cli.cwd.display())))?;
    let abs_old = std::fs::canonicalize(descendant)
        .map_err(|_| Error::new(format!("bad descendant {}", descendant.display())))?;
    let Ok(prefix) = abs_old.strip_prefix(&abs_new) else {
        err!("descendant is not under the new root");
    };
    let Some(prefix) = prefix.to_str().map(str::to_string) else {
        err!("descendant path is not UTF-8");
    };

    // Fold preview: the descendant's control files, prefixed.
    let attr_folded = folded_lines(&abs_old, ATTRIBUTES_FILE, &prefix, Kind::Attributes)?;
    let ign_folded = folded_lines(&abs_old, IGNORE_FILE, &prefix, Kind::Ignore)?;

    // SPEC §1.4: implementations MUST NOT proceed without explicit
    // confirmation; the folded control files are presented at the
    // prompt.
    cli.note(&format!(
        "found one descendant repository: {}\n",
        descendant.display()
    ));
    if !attr_folded.is_empty() {
        cli.note(&format!(
            "attributes entries to fold into {ATTRIBUTES_FILE}:\n"
        ));
        for line in &attr_folded {
            cli.note(&format!("  {line}\n"));
        }
    }
    if !ign_folded.is_empty() {
        cli.note(&format!("ignore entries to fold into {IGNORE_FILE}:\n"));
        for line in &ign_folded {
            cli.note(&format!("  {line}\n"));
        }
    }
    let new_root_label = cli.cwd.display().to_string();
    if !cli.confirm(&format!("elevate its root to {new_root_label}?"), yes)? {
        cli.note("elevation declined; nothing changed\n");
        return Ok(None);
    }

    // Move .lit/ up.
    std::fs::rename(abs_old.join(".lit"), abs_new.join(".lit")).map_err(|e| {
        Error::new(format!(
            "cannot move .lit from {} to {}: {e}",
            abs_old.display(),
            abs_new.display()
        ))
    })?;
    let repo = Repository::new(&abs_new)?;

    // Rewrite every version object's manifest in place (hashes
    // unchanged, SPEC §1.4/§8); same for the tabula record.
    let theca = repo.theca();
    for vhash in theca.list("version")? {
        let Some(mut vo) = VersionObject::load(&theca, &vhash)? else {
            continue;
        };
        let m = vo
            .manifest()
            .iter()
            .map(|(p, v)| (format!("{prefix}/{p}"), v.clone()))
            .collect();
        vo.rewrite_manifest(&theca, m)?;
    }
    if let Some(tab) = repo.read_tabula()? {
        let m = tab
            .manifest()
            .iter()
            .map(|(p, v)| (format!("{prefix}/{p}"), v.clone()))
            .collect();
        repo.write_tabula(&VersionObject::new(
            tab.segmentation(),
            tab.trios().clone(),
            m,
        )?)?;
    }

    // Fold control files. The descendant's files stay in place as
    // ordinary documents (SPEC §1.4); only their *effect* is
    // root-anchored, hence the prefixed copies at the new root.
    append_lines(&abs_new.join(ATTRIBUTES_FILE), &attr_folded)?;
    append_lines(&abs_new.join(IGNORE_FILE), &ign_folded)?;

    cli.note(&format!(
        "elevated: repository root is now {new_root_label}\n"
    ));
    Ok(Some(repo))
}

#[derive(PartialEq)]
enum Kind {
    Attributes,
    Ignore,
}

/// Prefixes one control file's entries with the descendant's
/// root-relative directory. An unanchored glob (no slash) matches
/// at any depth, so it prefixes as `<dir>/**/<glob>`; anchored
/// globs prefix directly.
fn folded_lines(dir: &Path, file: &str, prefix: &str, kind: Kind) -> Result<Vec<String>> {
    let path = dir.join(file);
    let is_symlink = path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if !path.is_file() || is_symlink {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| Error::new(format!("cannot read {}: {e}", path.display())))?;
    let content = String::from_utf8(bytes)
        .map_err(|_| Error::new(format!("{} is not UTF-8", path.display())))?;

    let mut out = Vec::new();
    for raw in content.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (glob, rest): (String, Option<String>) = if kind == Kind::Attributes {
            match split_attr(trimmed) {
                Some((g, r)) => (g, Some(r)),
                None => continue,
            }
        } else {
            (line.to_string(), None)
        };
        let (neg, glob) = match glob.strip_prefix('!') {
            Some(g) => ("!", g.to_string()),
            None => ("", glob),
        };
        let prefixed = if glob.starts_with('/') {
            format!("{prefix}{glob}")
        } else if glob
            .char_indices()
            .any(|(i, c)| c == '/' && i + 1 < glob.len())
        {
            format!("{prefix}/{glob}")
        } else {
            format!("{prefix}/**/{glob}")
        };
        out.push(match rest {
            Some(r) => format!("{neg}{prefixed}\t{r}"),
            None => format!("{neg}{prefixed}"),
        });
    }
    Ok(out)
}

/// The attributes-line split for folding: glob token (with
/// backslash escapes) then the rest of the line.
fn split_attr(line: &str) -> Option<(String, String)> {
    let mut glob = String::new();
    let mut rest = line;
    loop {
        let mut cs = rest.chars();
        match cs.next() {
            Some('\\') => match cs.next() {
                Some(c) => {
                    glob.push('\\');
                    glob.push(c);
                    rest = cs.as_str();
                }
                None => return None,
            },
            Some(c) if c != ' ' && c != '\t' => {
                glob.push(c);
                rest = cs.as_str();
            }
            _ => break,
        }
    }
    if glob.is_empty() {
        return None;
    }
    let after = rest.trim_start_matches([' ', '\t']);
    if after.len() == rest.len() || after.is_empty() {
        return None;
    }
    Some((glob, after.to_string()))
}

fn append_lines(path: &Path, lines: &[String]) -> Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    use std::io::Write;
    let mut fh = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| Error::new(format!("cannot append to {}: {e}", path.display())))?;
    fh.write_all(format!("{}\n", lines.join("\n")).as_bytes())?;
    Ok(())
}
