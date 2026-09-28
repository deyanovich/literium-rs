//! Verbatim atom spans (SPEC §8.6).
//!
//! Each atom's exact byte range within its grex's verbatim range,
//! recovered through an offset map threaded through content
//! extraction: §3.1's CRLF folding and §3.2/§3.3's whitespace
//! collapse are deterministic, so every extracted character maps
//! to a source byte offset and segmentation boundaries map back
//! to volumen offsets. Spans partition the grex range (each span
//! owning its trailing whitespace — the grex-range design one
//! level down); a span additionally records the atom's *first
//! contributing byte*, which the combining merge's assembly rule
//! uses (SPEC §9.6). Derivable, never stored.

use crate::derive::Derivation;
use crate::error::Result;
use crate::extract::{is_blank_line, Extraction};
use crate::grex_ranges;
use crate::segment::{edge_strip, mode_spec};

/// One atom's verbatim span within the volumen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Partition start: equals `first` except for a grex's first
    /// atom, whose span also owns the range's leading bytes.
    pub start: usize,
    /// The atom's first contributing byte.
    pub first: usize,
    /// Exclusive end (the next atom's first contributing byte, or
    /// the grex range's end for the last atom).
    pub end: usize,
}

/// One extracted character with its source byte offset.
struct OChar {
    c: char,
    off: usize,
}

/// Extraction with offsets: mirrors `extract::extract_grexes`
/// exactly, but each extracted character carries the volumen byte
/// offset of its first contributing byte. Returns one entry per
/// RAW grex (matching `Derivation::atom_bearing`).
fn extract_with_offsets(text: &str, extraction: Extraction) -> Vec<Vec<OChar>> {
    // §3.1 rule 1 as a line recognizer over the ORIGINAL bytes:
    // \r\n, lone \r, and \n all terminate lines; content chars
    // keep their true offsets.
    struct Line {
        chars: Vec<OChar>,
        term_off: Option<usize>, // offset of the terminator's first byte
    }
    let mut lines: Vec<Line> = Vec::new();
    let mut cur: Vec<OChar> = Vec::new();
    let mut it = text.char_indices().peekable();
    while let Some((off, c)) = it.next() {
        match c {
            '\r' => {
                if matches!(it.peek(), Some((_, '\n'))) {
                    it.next();
                }
                lines.push(Line {
                    chars: std::mem::take(&mut cur),
                    term_off: Some(off),
                });
            }
            '\n' => lines.push(Line {
                chars: std::mem::take(&mut cur),
                term_off: Some(off),
            }),
            _ => cur.push(OChar { c, off }),
        }
    }
    lines.push(Line {
        chars: cur,
        term_off: None,
    });

    // Blank-line grouping (§3.1 rules 2-4), identical to
    // extract_grexes: maximal runs of non-blank lines.
    let mut grexes: Vec<Vec<&Line>> = Vec::new();
    let mut run: Vec<&Line> = Vec::new();
    for line in &lines {
        let content: String = line.chars.iter().map(|o| o.c).collect();
        if is_blank_line(&content) {
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
            Extraction::Poetry => {
                // §3.3: LFs preserved between lines; within a
                // line, space/tab runs collapse to one space
                // carrying the run's first offset.
                let mut out: Vec<OChar> = Vec::new();
                for (li, line) in g.iter().enumerate() {
                    if li > 0 {
                        let prev = g[li - 1];
                        out.push(OChar {
                            c: '\n',
                            off: prev.term_off.expect("interior line has terminator"),
                        });
                    }
                    let mut in_run = false;
                    for oc in &line.chars {
                        if oc.c == ' ' || oc.c == '\t' {
                            if !in_run {
                                out.push(OChar {
                                    c: ' ',
                                    off: oc.off,
                                });
                                in_run = true;
                            }
                        } else {
                            out.push(OChar {
                                c: oc.c,
                                off: oc.off,
                            });
                            in_run = false;
                        }
                    }
                }
                out
            }
            Extraction::Prose => {
                // §3.2: runs of [space|tab|LF] collapse to one
                // space carrying the run's first offset.
                let mut out: Vec<OChar> = Vec::new();
                let mut in_run = false;
                for (li, line) in g.iter().enumerate() {
                    if li > 0 {
                        // The joining LF is whitespace in the run.
                        let prev = g[li - 1];
                        let off = prev.term_off.expect("interior line has terminator");
                        if !in_run {
                            out.push(OChar { c: ' ', off });
                            in_run = true;
                        }
                    }
                    for oc in &line.chars {
                        if oc.c == ' ' || oc.c == '\t' {
                            if !in_run {
                                out.push(OChar {
                                    c: ' ',
                                    off: oc.off,
                                });
                                in_run = true;
                            }
                        } else {
                            out.push(OChar {
                                c: oc.c,
                                off: oc.off,
                            });
                            in_run = false;
                        }
                    }
                }
                out
            }
        })
        .collect()
}

/// Computes each atom's verbatim span for every atom-bearing grex
/// of a derivation, aligned with `Derivation::grexes`.
pub fn atom_spans(d: &Derivation) -> Result<Vec<Vec<Span>>> {
    let spec = mode_spec(&d.mode).unwrap_or_else(|| panic!("unknown mode '{}'", d.mode));
    let text = std::str::from_utf8(&d.volumen)
        .map_err(|_| crate::error::Error::new("spans: volumen is not UTF-8"))?;
    let ranges = grex_ranges::extract(&d.volumen, &d.atom_bearing)?;

    let raw = extract_with_offsets(text, spec.extraction);
    if raw.len() != d.atom_bearing.len() {
        err!(
            "spans: raw grex count {} does not match atom_bearing {}",
            raw.len(),
            d.atom_bearing.len()
        );
    }

    let mut out: Vec<Vec<Span>> = Vec::new();
    let mut kept = 0usize; // index into ranges / d.grexes
    for (gi, ochars) in raw.iter().enumerate() {
        if !d.atom_bearing[gi] {
            continue;
        }
        let (range_start, range_end) = ranges[kept];

        // Map each atom's first post-strip char back to its
        // source offset. The walk depends on the extraction kind:
        // prose boundary functions are exhaustive splitters
        // (segments concatenate to the text), poetry drops the LF
        // separators — walked explicitly.
        let grex_text: String = ochars.iter().map(|o| o.c).collect();
        let mut firsts: Vec<usize> = Vec::new();
        match spec.extraction {
            Extraction::Prose => {
                let mut char_base = 0usize; // char index of segment start
                for seg in (spec.atom_boundary)(&grex_text) {
                    let seg_chars = seg.chars().count();
                    if !edge_strip(seg).is_empty() {
                        // First post-strip char's index in the segment.
                        let lead = seg_chars - seg.trim_start_matches(is_edge_char).chars().count();
                        firsts.push(ochars[char_base + lead].off);
                    }
                    char_base += seg_chars;
                }
                if char_base != ochars.len() {
                    err!("spans: boundary function is not an exhaustive splitter");
                }
            }
            Extraction::Poetry => {
                // Non-empty lines are atoms (§4.2); the '\n'
                // separators occupy one char each.
                let mut pos = 0usize; // char index of line start
                for line in grex_text.split('\n') {
                    let line_chars = line.chars().count();
                    if !line.is_empty() && !edge_strip(line).is_empty() {
                        let lead =
                            line_chars - line.trim_start_matches(is_edge_char).chars().count();
                        firsts.push(ochars[pos + lead].off);
                    }
                    pos += line_chars + 1; // the joining LF
                }
            }
        }

        if firsts.len() != d.grexes[kept].len() {
            err!(
                "spans: atom count mismatch in grex {} ({} vs {})",
                kept + 1,
                firsts.len(),
                d.grexes[kept].len()
            );
        }

        let mut spans: Vec<Span> = Vec::new();
        for (i, &first) in firsts.iter().enumerate() {
            let start = if i == 0 { range_start } else { first };
            let end = if i + 1 < firsts.len() {
                firsts[i + 1]
            } else {
                range_end
            };
            spans.push(Span { start, first, end });
        }

        // Partition invariants (SPEC §8.6).
        if let (Some(f), Some(l)) = (spans.first(), spans.last()) {
            if f.start != range_start || l.end != range_end {
                err!("spans: grex {} spans do not cover its range", kept + 1);
            }
        }
        for w in spans.windows(2) {
            if w[0].end != w[1].start {
                err!("spans: grex {} spans do not partition", kept + 1);
            }
        }

        out.push(spans);
        kept += 1;
    }
    Ok(out)
}

fn is_edge_char(c: char) -> bool {
    edge_strip(&c.to_string()).is_empty()
}

/// The assembly bytes for one atom (SPEC §9.6): from the first
/// contributing byte to the span end, right-trimmed of the
/// trailing run of §4.3 edge-class characters (Sp/Sep/CR/LF —
/// so a source NBSP between sentences cannot double the joiner).
pub fn assembly_bytes<'v>(volumen: &'v [u8], span: &Span) -> &'v [u8] {
    let raw = std::str::from_utf8(&volumen[span.first..span.end])
        .expect("span offsets sit on char boundaries");
    raw.trim_end_matches(is_edge_char).as_bytes()
}
