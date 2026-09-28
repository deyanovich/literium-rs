//! Verbatim grex byte ranges (SPEC §8.6).
//!
//! Partitions a volumen's raw bytes into one half-open range
//! `[s, e)` per spina entry. Line ends and blank lines are
//! recognized per SPEC §3.1 rules 1–2 *for delimiting only*; the
//! bytes themselves are never altered. A grex's range includes its
//! trailing separator run; the document's leading blank lines
//! belong to the first range, everything after the last grex to
//! the last range.
//!
//! Phantom-grex fold (PROVISIONAL.md #5): a raw grex deriving zero
//! atoms (e.g. a lone-NBSP line, which is not blank per §3.1 rule
//! 2 but whose every atom strips empty per §4.3) has no spina
//! entry; its bytes fold into the preceding atom-bearing range, or
//! into the first range when no atom-bearing grex precedes it.

use crate::error::Result;

/// `atom_bearing` has one flag per raw grex (Derive's
/// `atom_bearing`); the result has one `(start, end)` per set
/// flag, partitioning `[0, len)`.
pub fn extract(volumen: &[u8], atom_bearing: &[bool]) -> Result<Vec<(usize, usize)>> {
    let len = volumen.len();

    // Scan raw bytes into lines: content range + terminator.
    // Terminators recognized: \r\n, lone \r, lone \n (§3.1 rule 1
    // as a recognizer).
    let mut line_start: Vec<usize> = Vec::new();
    let mut line_blank: Vec<bool> = Vec::new();
    let mut pos = 0;
    while pos < len {
        let start = pos;
        let nl = memchr(volumen, b'\n', pos);
        let cr = memchr(volumen, b'\r', pos);
        let (content_end, next) = match (cr, nl) {
            (Some(c), n) if n.is_none() || c < n.unwrap() => {
                let next = if c + 1 < len && volumen[c + 1] == b'\n' {
                    c + 2
                } else {
                    c + 1
                };
                (c, next)
            }
            (_, Some(n)) => (n, n + 1),
            _ => (len, len),
        };
        let content = &volumen[start..content_end];
        line_start.push(start);
        line_blank.push(content.iter().all(|&b| b == b' ' || b == b'\t'));
        pos = next;
    }

    // Raw grexes: maximal runs of non-blank lines; each raw range
    // runs from its first line's first byte to the next raw grex's
    // first byte (or EOF), so it owns its trailing separator run.
    let mut raw_start: Vec<usize> = Vec::new(); // line index of first line
    let mut in_grex = false;
    for i in 0..line_start.len() {
        if !line_blank[i] {
            if !in_grex {
                raw_start.push(i);
            }
            in_grex = true;
        } else {
            in_grex = false;
        }
    }
    if raw_start.len() != atom_bearing.len() {
        err!(
            "GrexRanges: raw grex count does not match atom_bearing ({} vs {})",
            raw_start.len(),
            atom_bearing.len()
        );
    }

    let mut raw_ranges: Vec<(usize, usize)> = Vec::new();
    for g in 0..raw_start.len() {
        let s = if g == 0 { 0 } else { line_start[raw_start[g]] };
        let e = if g == raw_start.len() - 1 {
            len
        } else {
            line_start[raw_start[g + 1]]
        };
        raw_ranges.push((s, e));
    }
    // (Leading blank lines already belong to the first range via
    // the g == 0 start-at-0 rule above.)

    // Phantom fold.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut pending_prefix: Option<usize> = None;
    for (g, &(s, e)) in raw_ranges.iter().enumerate() {
        let s = pending_prefix.unwrap_or(s);
        if atom_bearing[g] {
            ranges.push((s, e));
            pending_prefix = None;
        } else if let Some(last) = ranges.last_mut() {
            last.1 = e; // fold into preceding kept range
            pending_prefix = None;
        } else if pending_prefix.is_none() {
            pending_prefix = Some(s); // fold into the first kept range
        }
    }
    if ranges.is_empty() {
        err!("GrexRanges: document has no atom-bearing grex");
    }

    // Partition invariants.
    if ranges[0].0 != 0 {
        err!("GrexRanges: ranges do not start at 0");
    }
    if ranges.last().unwrap().1 != len {
        err!("GrexRanges: ranges do not end at EOF");
    }
    for i in 1..ranges.len() {
        if ranges[i].0 != ranges[i - 1].1 {
            err!("GrexRanges: ranges do not partition the volumen");
        }
    }
    Ok(ranges)
}

fn memchr(haystack: &[u8], needle: u8, from: usize) -> Option<usize> {
    haystack[from..]
        .iter()
        .position(|&b| b == needle)
        .map(|i| i + from)
}
