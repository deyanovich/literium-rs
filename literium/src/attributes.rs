//! Mode attribution (SPEC §1.1).
//!
//! Parses the repo-root attributes file — one `<glob> <mode>`
//! mapping per non-blank, non-comment line — and resolves a
//! file's mode with first-match-wins semantics (SPEC §1.1
//! deliberately inverts `.gitattributes`' last-match-wins).
//! Errors on negated globs and unknown modes.

use crate::error::Result;
use crate::glob::{compile, match_file_or_ancestor, Rule};

#[derive(Debug)]
struct Entry {
    rule: Rule,
    mode: String,
    #[allow(dead_code)]
    glob: String,
}

#[derive(Default, Debug)]
pub struct Attributes {
    entries: Vec<Entry>,
}

impl Attributes {
    pub fn parse(content: &str) -> Result<Attributes> {
        let mut entries = Vec::new();
        for (i, raw) in content.split('\n').enumerate() {
            let lineno = i + 1;
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            // SPEC §1.1: comments and blank lines.
            let trimmed = line.trim_start_matches([' ', '\t']);
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Columns separated by runs of space or tab; the glob
            // may carry backslash-escaped whitespace (gitignore
            // rules).
            let (glob, mode) = match split_entry(trimmed) {
                Some(pair) => pair,
                None => {
                    err!("attributes line {lineno}: expected '<glob> <mode>'")
                }
            };

            // SPEC §1.1: `!` negation is not permitted — each line
            // is a mapping, not an exclusion.
            if glob.starts_with('!') {
                err!("attributes line {lineno}: '!' negation is not permitted");
            }
            if mode != "prose" && mode != "poetry" {
                err!("attributes line {lineno}: mode must be 'prose' or 'poetry'");
            }

            let rule = match compile(&glob) {
                Some(r) => r,
                None => err!("attributes line {lineno}: unusable glob '{glob}'"),
            };
            entries.push(Entry {
                rule,
                mode: mode.to_string(),
                glob,
            });
        }
        Ok(Attributes { entries })
    }

    /// SPEC §1.1: the first matching entry wins; a file matched by
    /// no entry is prose. A rule matching an ancestor directory
    /// covers the file (gitignore matching semantics).
    ///
    /// SPEC §1.1/§1.2: the control files — the repo-root
    /// attributes file and every `.litignore` — are prose
    /// unconditionally, before any entry is consulted; a catch-all
    /// like `* poetry` must not re-mode them (HISTORY §3 *Control
    /// files are always prose*).
    pub fn mode_for(&self, rel_path: &str) -> &str {
        if rel_path == crate::ATTRIBUTES_FILE
            || rel_path == crate::IGNORE_FILE
            || rel_path
                .rsplit('/')
                .next()
                .is_some_and(|b| b == crate::IGNORE_FILE && rel_path.contains('/'))
        {
            return "prose";
        }
        for e in &self.entries {
            if match_file_or_ancestor(&e.rule, rel_path) {
                return &e.mode;
            }
        }
        "prose"
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Splits one attributes line into (glob, mode). The glob token is
/// a run of backslash-escape pairs or non-whitespace,
/// non-backslash characters; then whitespace; then the mode token;
/// then optional trailing whitespace.
fn split_entry(line: &str) -> Option<(String, &str)> {
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
                None => return None, // lone trailing backslash
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
    if after.len() == rest.len() {
        return None; // no separating whitespace
    }
    let mode_end = after.find([' ', '\t']).unwrap_or(after.len());
    let (mode, tail) = after.split_at(mode_end);
    if mode.is_empty() || !tail.trim_matches([' ', '\t']).is_empty() {
        return None;
    }
    Some((glob, mode))
}
