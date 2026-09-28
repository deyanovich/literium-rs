//! Intermediate volumen construction (SPEC §8.6).
//!
//! The splice rule, subset-based (PROVISIONAL.md #5). The
//! intermediate volumen for any set of inscribed revisions is a
//! re-concatenation over the document's cell list, choosing tabula
//! bytes for cells whose revision is inscribed and parent bytes
//! otherwise. The result depends only on the *subset*, never the
//! inscription order; the empty subset yields the parent volumen
//! and the full subset is byte-identical to the tabula. A previous
//! splice product is never patched.

use std::collections::HashSet;

use crate::revision_detect::{CellType, DocCorrespondence, DocState};

/// The intermediate volumen for one document under a set of
/// inscribed revision ids.
pub fn volumen_for(doc: &DocCorrespondence, inscribed: &HashSet<String>) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for cell in &doc.cells {
        let on = cell.rev.as_ref().is_some_and(|id| inscribed.contains(id));
        match cell.cell_type {
            // Untouched anchors carry equal bytes on both sides;
            // touched anchors belong to a formatting revision.
            CellType::Anchor | CellType::Pair | CellType::Regroup => {
                if on {
                    push_bytes(&mut out, &doc.tabula, &cell.t);
                } else {
                    push_bytes(&mut out, &doc.parent, &cell.p);
                }
            }
            CellType::Del => {
                if !on {
                    push_bytes(&mut out, &doc.parent, &cell.p);
                }
            }
            CellType::Ins => {
                if on {
                    push_bytes(&mut out, &doc.tabula, &cell.t);
                }
            }
        }
    }
    out
}

fn push_bytes(out: &mut Vec<u8>, side: &DocState, indices: &[usize]) {
    for &i in indices {
        let (s, e) = side.ranges[i];
        out.extend_from_slice(&side.derivation.volumen[s..e]);
    }
}
