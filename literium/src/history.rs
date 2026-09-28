//! Histories and inscriptiones (SPEC §8.4, §9).
//!
//! One instance wraps one history file — the historia or an
//! experiment. A history is an append-only event log; walking it
//! yields the version sequence, and it is the sole source of
//! version ordering (version objects carry no parent references,
//! SPEC §8). Order is entry order, never timestamps. Per-entry
//! encoding is provisional (PROVISIONAL.md #4):
//!
//! ```text
//! <version-hash>\t<ISO-8601 UTC, microseconds>\t<escaped message>
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct Inscriptio {
    pub vhash: String,
    pub timestamp: String,
    pub message: String,
}

#[derive(Debug)]
pub struct History {
    file: PathBuf,
    name: String,
}

impl History {
    pub fn new(file: impl Into<PathBuf>, name: &str) -> Self {
        History {
            file: file.into(),
            name: name.to_string(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Appends one inscriptio (SPEC §8.4). Re-inscription of a
    /// hash already present is ordinary (SPEC §9.5).
    pub fn append(&self, vhash: &str, message: &str, timestamp: Option<&str>) -> Result<()> {
        if vhash.len() != 64
            || !vhash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            err!("History: malformed version hash '{vhash}'");
        }
        let ts = match timestamp {
            Some(t) => t.to_string(),
            None => now_timestamp(),
        };
        let line = format!("{vhash}\t{ts}\t{}\n", escape(message));
        let mut fh = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
            .map_err(|e| {
                crate::error::Error::new(format!(
                    "History: cannot append to {}: {e}",
                    self.file.display()
                ))
            })?;
        fh.write_all(line.as_bytes())?;
        Ok(())
    }

    /// All inscriptiones in entry order.
    pub fn entries(&self) -> Result<Vec<Inscriptio>> {
        if !self.file.exists() {
            return Ok(Vec::new());
        }
        let bytes = std::fs::read(&self.file).map_err(|e| {
            crate::error::Error::new(format!("History: cannot read {}: {e}", self.file.display()))
        })?;
        let content = String::from_utf8(bytes)
            .map_err(|_| crate::error::Error::new("History: file is not UTF-8"))?;

        // Perl's split drops trailing empty fields; interior empty
        // lines are malformed entries.
        let mut lines: Vec<&str> = content.split('\n').collect();
        while lines.last() == Some(&"") {
            lines.pop();
        }
        let mut entries = Vec::new();
        for (i, line) in lines.into_iter().enumerate() {
            let mut parts = line.splitn(3, '\t');
            let vhash = parts.next().unwrap_or("");
            let ts = parts.next();
            let msg = parts.next();
            let (Some(ts), Some(msg)) = (ts, msg) else {
                err!(
                    "History: malformed entry at {} line {}",
                    self.file.display(),
                    i + 1
                );
            };
            if vhash.len() != 64
                || !vhash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                err!(
                    "History: malformed entry at {} line {}",
                    self.file.display(),
                    i + 1
                );
            }
            entries.push(Inscriptio {
                vhash: vhash.to_string(),
                timestamp: ts.to_string(),
                message: unescape(msg),
            });
        }
        Ok(entries)
    }

    /// The last inscriptio, or `None` on an empty history.
    pub fn latest(&self) -> Result<Option<Inscriptio>> {
        Ok(self.entries()?.pop())
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.entries()?.is_empty())
    }
}

/// ISO-8601 UTC with microseconds; lexicographic order equals
/// chronological order (PROVISIONAL.md #4).
pub fn now_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970");
    let (secs, micros) = (now.as_secs() as i64, now.subsec_micros());
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.{micros:06}Z")
}

/// Days-since-epoch → (year, month, day), proleptic Gregorian
/// (Howard Hinnant's civil_from_days).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn escape(msg: &str) -> String {
    msg.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn unescape(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut cs = msg.chars();
    while let Some(c) = cs.next() {
        if c == '\\' {
            match cs.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    // Perl's unescape leaves unknown escapes as-is.
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
