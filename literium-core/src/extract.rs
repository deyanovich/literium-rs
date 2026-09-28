//! Content extraction (SPEC §3).
//!
//! Operates on a decoded string (the volumen's bytes decoded as
//! UTF-8) and produces the ordered list of grex texts. Extraction
//! is applied only when computing hashes and diffs; the on-disk
//! bytes are never touched.

/// Which SPEC §3 within-grex whitespace rules apply (§3.2 prose,
/// §3.3 poetry). Part of a mode spec (SPEC §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extraction {
    Prose,
    Poetry,
}

/// SPEC §3.1 rule 2: a blank line consists only of U+0020 SPACE
/// and U+0009 TAB, or is empty. A line of non-ASCII whitespace
/// (e.g. a lone NBSP) is NOT blank.
pub fn is_blank_line(line: &str) -> bool {
    line.chars().all(|c| c == ' ' || c == '\t')
}

/// Returns one entry per raw grex (maximal run of non-blank
/// lines), in document order, with the mode's within-grex
/// whitespace rules applied. Grexes that will later derive zero
/// atoms are still returned (the caller decides what to do with
/// them); blank-line runs and leading/trailing blank lines are
/// consumed here (SPEC §3.1 rules 3–4 — separators normalize to a
/// conceptual `\n\n`, which the list representation makes
/// implicit).
pub fn extract_grexes(text: &str, extraction: Extraction) -> Vec<String> {
    // SPEC §3.1 rule 1: \r\n -> \n, then any remaining \r -> \n.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");

    // SPEC §3.1 rule 2: lines are runs between LFs (or start/end
    // and an LF). str::split keeps the final empty field when the
    // text ends in LF; it is blank and falls out of grouping.
    let mut grexes: Vec<Vec<&str>> = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        if is_blank_line(line) {
            if !run.is_empty() {
                grexes.push(std::mem::take(&mut run));
            }
        } else {
            run.push(line);
        }
    }
    if !run.is_empty() {
        grexes.push(run);
    }

    grexes
        .into_iter()
        .map(|g| match extraction {
            // SPEC §3.3: LFs between non-blank lines are preserved
            // (rule 6); within a line, runs of space/tab collapse
            // to one U+0020 (rule 7).
            Extraction::Poetry => g
                .iter()
                .map(|l| collapse(l, false))
                .collect::<Vec<_>>()
                .join("\n"),
            // SPEC §3.2 rule 6: runs of [SPACE|TAB|LF] collapse to
            // a single U+0020; a prose grex is a single line.
            Extraction::Prose => collapse(&g.join("\n"), true),
        })
        .collect()
}

/// Collapse runs of U+0020/U+0009 (and U+000A when `with_lf`) to a
/// single U+0020. Non-ASCII whitespace passes through verbatim
/// (SPEC §3.1 rule 5).
fn collapse(s: &str, with_lf: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_run = false;
    for c in s.chars() {
        if c == ' ' || c == '\t' || (with_lf && c == '\n') {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            out.push(c);
            in_run = false;
        }
    }
    out
}
