//! Shared LCS machinery (SPEC §8.8, §7.4, §9.6).
//!
//! One longest-common-subsequence implementation with the
//! normative match-eager, parent-advance-on-tie traceback — the
//! unique leftmost trace — used by revision detection (fasciculus
//! alignment), diff display (atom ops), and the combining merge
//! (both diff3 levels).

/// Matched index pairs `(i, j)` in the leftmost trace.
pub fn anchors<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0u32; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else if dp[i + 1][j] >= dp[i][j + 1] {
                dp[i + 1][j]
            } else {
                dp[i][j + 1]
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < m && j < n {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Equal,
    Del,
    Ins,
}

impl Op {
    pub fn sigil(self) -> char {
        match self {
            Op::Equal => '=',
            Op::Del => '-',
            Op::Ins => '+',
        }
    }
}

/// Edit script in the leftmost trace: `(op, index)` where the
/// index is into `a` for Equal/Del and into `b` for Ins.
pub fn ops<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(Op, usize)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    for (ai, bj) in anchors(a, b) {
        while i < ai {
            out.push((Op::Del, i));
            i += 1;
        }
        while j < bj {
            out.push((Op::Ins, j));
            j += 1;
        }
        out.push((Op::Equal, i));
        i += 1;
        j += 1;
    }
    while i < a.len() {
        out.push((Op::Del, i));
        i += 1;
    }
    while j < b.len() {
        out.push((Op::Ins, j));
        j += 1;
    }
    out
}
