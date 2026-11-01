//! Text differences for page version comparison: a longest-common-subsequence diff over blocks
//! (one unit per top-level block of Markdown), then over words inside changed blocks.
//!
//! Work is bounded: common prefixes and suffixes are stripped in linear time, and the quadratic
//! LCS table is only built when it has at most [`MAX_CELLS`] cells. Above that the differing
//! middle is reported as one delete plus one insert — correct, just coarse.

use serde::{Deserialize, Serialize};

/// Largest LCS table (rows × columns) built for one diff.
pub const MAX_CELLS: usize = 1_000_000;
/// Longest text (in bytes) diffed word by word; longer changed blocks are replaced whole.
pub const MAX_WORD_DIFF_BYTES: usize = 64 * 1024;
/// Total LCS cells one [`diff_blocks`] may spend on word diffs; later changed blocks are
/// reported as a delete plus an insert.
pub const MAX_TOTAL_CELLS: usize = 20_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffKind {
    Equal,
    Insert,
    Delete,
}

impl DiffKind {
    pub fn name(self) -> &'static str {
        match self {
            DiffKind::Equal => "equal",
            DiffKind::Insert => "insert",
            DiffKind::Delete => "delete",
        }
    }
}

/// A run of text that is unchanged, inserted or deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub kind: DiffKind,
    pub text: String,
}

/// One block of a block-level diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockDiff {
    Equal(String),
    Insert(String),
    Delete(String),
    /// A block edited in place, with its word-level differences.
    Change {
        from: String,
        to: String,
        inline: Vec<Hunk>,
    },
}

/// The edit script turning `a` into `b`: `(kind, index)` pairs, where the index is into `a` for
/// equal/delete and into `b` for insert. Bounded by [`MAX_CELLS`].
pub fn diff_seq<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(DiffKind, usize)> {
    let prefix = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let (ra, rb) = (a.get(prefix..).unwrap_or_default(), b.get(prefix..).unwrap_or_default());
    let suffix = ra.iter().rev().zip(rb.iter().rev()).take_while(|(x, y)| x == y).count();
    let ma = ra.get(..ra.len().saturating_sub(suffix)).unwrap_or_default();
    let mb = rb.get(..rb.len().saturating_sub(suffix)).unwrap_or_default();

    let mut out: Vec<(DiffKind, usize)> = (0..prefix).map(|i| (DiffKind::Equal, i)).collect();
    middle(ma, mb, prefix, &mut out);
    let tail_a = prefix.saturating_add(ma.len());
    out.extend((0..suffix).map(|i| (DiffKind::Equal, tail_a.saturating_add(i))));
    out
}

/// LCS over the differing middle; `off` is the middle's offset into both inputs.
fn middle<T: PartialEq>(a: &[T], b: &[T], off: usize, out: &mut Vec<(DiffKind, usize)>) {
    let (n, m) = (a.len(), b.len());
    let fits = n.checked_add(1).and_then(|r| m.checked_add(1).and_then(|c| r.checked_mul(c))).is_some_and(|cells| cells <= MAX_CELLS);
    if !fits {
        out.extend((0..n).map(|i| (DiffKind::Delete, off.saturating_add(i))));
        out.extend((0..m).map(|j| (DiffKind::Insert, off.saturating_add(j))));
        return;
    }
    // table[i][j] = LCS length of a[i..] and b[j..], flattened row-major.
    let w = m + 1;
    let mut table = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let v = if a.get(i) == b.get(j) {
                table.get((i + 1) * w + j + 1).copied().unwrap_or(0).saturating_add(1)
            } else {
                let down = table.get((i + 1) * w + j).copied().unwrap_or(0);
                let right = table.get(i * w + j + 1).copied().unwrap_or(0);
                down.max(right)
            };
            if let Some(c) = table.get_mut(i * w + j) {
                *c = v;
            }
        }
    }
    let at = |i: usize, j: usize| table.get(i * w + j).copied().unwrap_or(0);
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a.get(i) == b.get(j) {
            out.push((DiffKind::Equal, off + i));
            i += 1;
            j += 1;
        } else if i < n && (j >= m || at(i + 1, j) >= at(i, j + 1)) {
            // Deletes come before inserts within a changed run.
            out.push((DiffKind::Delete, off + i));
            i += 1;
        } else {
            out.push((DiffKind::Insert, off + j));
            j += 1;
        }
    }
}

/// Splits text into words, whitespace runs and single punctuation characters, so that the
/// tokens concatenate back to the input exactly.
pub fn tokens(s: &str) -> Vec<&str> {
    #[derive(PartialEq, Clone, Copy)]
    enum C {
        Word,
        Space,
        Punct,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            C::Word
        } else if c.is_whitespace() {
            C::Space
        } else {
            C::Punct
        }
    };
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<C> = None;
    for (i, c) in s.char_indices() {
        let k = class(c);
        if let Some(p) = prev
            && (p != k || k == C::Punct)
        {
            if let Some(t) = s.get(start..i) {
                out.push(t);
            }
            start = i;
        }
        prev = Some(k);
    }
    if let Some(t) = s.get(start..)
        && !t.is_empty()
    {
        out.push(t);
    }
    out
}

/// Word-level differences between two texts, adjacent runs of the same kind merged.
pub fn diff_words(a: &str, b: &str) -> Vec<Hunk> {
    let mut budget = usize::MAX;
    diff_words_within(a, b, &mut budget)
}

/// [`diff_words`] charged against `budget` (LCS cells); replaces whole when it would overspend.
fn diff_words_within(a: &str, b: &str, budget: &mut usize) -> Vec<Hunk> {
    let whole = || {
        let mut out = Vec::new();
        push(&mut out, DiffKind::Delete, a);
        push(&mut out, DiffKind::Insert, b);
        out
    };
    if a.len() > MAX_WORD_DIFF_BYTES || b.len() > MAX_WORD_DIFF_BYTES {
        return whole();
    }
    let (ta, tb) = (tokens(a), tokens(b));
    // diff_seq never builds more than MAX_CELLS cells, so that bounds the charge.
    let cost = ta.len().saturating_add(1).saturating_mul(tb.len().saturating_add(1)).min(MAX_CELLS);
    if cost > *budget {
        return whole();
    }
    *budget -= cost;
    let mut out = Vec::new();
    for (k, i) in diff_seq(&ta, &tb) {
        let t = if k == DiffKind::Insert { tb.get(i) } else { ta.get(i) };
        push(&mut out, k, t.copied().unwrap_or_default());
    }
    out
}

fn push(out: &mut Vec<Hunk>, kind: DiffKind, text: &str) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(h) if h.kind == kind => h.text.push_str(text),
        _ => out.push(Hunk { kind, text: text.to_string() }),
    }
}

/// Block-level differences. Inside each run of deletes and inserts between unchanged blocks,
/// deleted and inserted blocks are paired in order; a pair that shares at least a third of its
/// words becomes a [`BlockDiff::Change`] with word-level hunks, otherwise a delete and an insert.
pub fn diff_blocks(a: &[String], b: &[String]) -> Vec<BlockDiff> {
    let script = diff_seq(a, b);
    let mut out = Vec::new();
    let mut budget = MAX_TOTAL_CELLS;
    let (mut dels, mut ins): (Vec<&String>, Vec<&String>) = (Vec::new(), Vec::new());
    for (k, i) in script {
        match k {
            DiffKind::Equal => {
                flush(&mut dels, &mut ins, &mut out, &mut budget);
                out.push(BlockDiff::Equal(a.get(i).cloned().unwrap_or_default()));
            }
            DiffKind::Delete => {
                if let Some(s) = a.get(i) {
                    dels.push(s);
                }
            }
            DiffKind::Insert => {
                if let Some(s) = b.get(i) {
                    ins.push(s);
                }
            }
        }
    }
    flush(&mut dels, &mut ins, &mut out, &mut budget);
    out
}

fn flush(dels: &mut Vec<&String>, ins: &mut Vec<&String>, out: &mut Vec<BlockDiff>, budget: &mut usize) {
    let pairs = dels.len().min(ins.len());
    for idx in 0..pairs {
        let (Some(d), Some(n)) = (dels.get(idx), ins.get(idx)) else { continue };
        let inline = diff_words_within(d, n, budget);
        if similar(&inline) {
            out.push(BlockDiff::Change { from: (*d).clone(), to: (*n).clone(), inline });
        } else {
            out.push(BlockDiff::Delete((*d).clone()));
            out.push(BlockDiff::Insert((*n).clone()));
        }
    }
    out.extend(dels.iter().skip(pairs).map(|d| BlockDiff::Delete((*d).clone())));
    out.extend(ins.iter().skip(pairs).map(|n| BlockDiff::Insert((*n).clone())));
    dels.clear();
    ins.clear();
}

/// Whether at least a third of the (non-space) characters survive unchanged.
fn similar(h: &[Hunk]) -> bool {
    let count = |k: Option<DiffKind>| -> usize {
        h.iter().filter(|x| k.is_none_or(|k| x.kind == k)).map(|x| x.text.chars().filter(|c| !c.is_whitespace()).count()).sum()
    };
    let (eq, all) = (count(Some(DiffKind::Equal)), count(None));
    all > 0 && eq.saturating_mul(3) >= all.saturating_sub(eq)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn seq_lcs() {
        let a = ['a', 'b', 'c', 'd'];
        let b = ['a', 'x', 'c', 'd', 'e'];
        let script = diff_seq(&a, &b);
        let kinds: Vec<&str> = script.iter().map(|(k, _)| k.name()).collect();
        assert_eq!(kinds, ["equal", "delete", "insert", "equal", "equal", "insert"]);
        assert!(diff_seq::<char>(&[], &[]).is_empty());
    }

    #[test]
    fn tokens_round_trip() {
        let t = "Hello, wörld  — 12_x!";
        assert_eq!(tokens(t).concat(), t);
        assert_eq!(tokens("a b"), ["a", " ", "b"]);
        assert!(tokens("").is_empty());
    }

    #[test]
    fn words() {
        let h = diff_words("the quick fox", "the slow fox");
        assert_eq!(
            h,
            vec![
                Hunk { kind: DiffKind::Equal, text: "the ".into() },
                Hunk { kind: DiffKind::Delete, text: "quick".into() },
                Hunk { kind: DiffKind::Insert, text: "slow".into() },
                Hunk { kind: DiffKind::Equal, text: " fox".into() },
            ]
        );
    }

    #[test]
    fn blocks_pair_changes() {
        let d = diff_blocks(&s(&["# T", "one two three", "gone"]), &s(&["# T", "one 2 three", "totally new"]));
        assert_eq!(d.first(), Some(&BlockDiff::Equal("# T".into())));
        assert!(matches!(d.get(1), Some(BlockDiff::Change { .. })));
        assert_eq!(d.get(2), Some(&BlockDiff::Delete("gone".into())));
        assert_eq!(d.get(3), Some(&BlockDiff::Insert("totally new".into())));
    }

    #[test]
    fn huge_inputs_are_bounded() {
        let a: Vec<String> = (0..5000).map(|i| format!("a{i}")).collect();
        let b: Vec<String> = (0..5000).map(|i| format!("b{i}")).collect();
        let d = diff_blocks(&a, &b);
        assert_eq!(d.len(), 10_000);
        let big = "word ".repeat(100_000);
        let h = diff_words(&big, "x");
        assert_eq!(h.len(), 2);
    }
}
