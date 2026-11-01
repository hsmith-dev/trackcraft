//! Issue rank: LexoRank-style base-36 strings that order issues by plain byte comparison.
//!
//! A rank is a base-36 fraction written with the digits `0-9a-z` (`"i"` is 0.5, `"i8"` is just
//! after it). Ranks never end in `0`, so the string order and the numeric order agree and a
//! key strictly between any two neighbours always exists. Moving an issue rewrites only the
//! moved issues' ranks; when keys grow past [`MAX_RANK_LEN`] the project is rebalanced (every
//! issue gets a fresh, evenly spaced short rank in the same order). Rank is not history-worthy.

/// Longest rank kept before the project is rebalanced.
pub const MAX_RANK_LEN: usize = 32;
/// Longest rank accepted as input to [`rank_between`] (anything longer is malformed).
const MAX_INPUT_LEN: usize = 256;
/// Appends and prepends move by a fixed step at this many digits (so a long run of
/// "create at the bottom" stays short instead of halving the remaining gap each time).
const STEP_WIDTH: usize = 6;
const STEP: u64 = 36 * 36;
const BASE: u64 = 36;

fn digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'z' => Some(c - b'a' + 10),
        _ => None,
    }
}

fn char_of(d: u8) -> char {
    match d {
        0..=9 => char::from(b'0' + d),
        10..=35 => char::from(b'a' + (d - 10)),
        _ => 'z',
    }
}

fn well_formed(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|c| digit(c).is_some()) && !s.ends_with('0')
}

/// Whether `s` is a usable rank: `0-9a-z`, 1..=[`MAX_RANK_LEN`] characters, not ending in `0`.
pub fn is_valid_rank(s: &str) -> bool {
    well_formed(s, MAX_RANK_LEN)
}

fn digits(s: &str) -> Vec<u8> {
    s.bytes().filter_map(digit).collect()
}

fn text(d: &[u8]) -> String {
    let end = d.iter().rposition(|&x| x != 0).map_or(0, |p| p.saturating_add(1));
    d.iter().take(end).map(|&x| char_of(x)).collect()
}

/// The value of the first `w` digits (missing digits are 0).
fn prefix_value(d: &[u8], w: usize) -> u64 {
    (0..w).fold(0u64, |acc, i| acc.saturating_mul(BASE).saturating_add(u64::from(d.get(i).copied().unwrap_or(0))))
}

fn from_value(mut v: u64, w: usize) -> Vec<u8> {
    let mut out = vec![0u8; w];
    for slot in out.iter_mut().rev() {
        *slot = u8::try_from(v % BASE).unwrap_or(0);
        v /= BASE;
    }
    out
}

/// A key strictly between `a` (empty = 0) and `b` (`None` = 1). Requires `a < b`.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Option<String> {
    let mut out = Vec::new();
    let mut upper_open = b.is_none();
    let limit = a.len().saturating_add(b.map_or(0, <[u8]>::len)).saturating_add(4);
    for i in 0..limit {
        let da = u32::from(a.get(i).copied().unwrap_or(0));
        let db = if upper_open { 36 } else { u32::from(b.and_then(|b| b.get(i)).copied().unwrap_or(0)) };
        if da == db {
            out.push(u8::try_from(da).ok()?);
            continue;
        }
        if db < da {
            return None; // a > b: not a valid interval
        }
        if db - da > 1 {
            out.push(u8::try_from((da + db) / 2).ok()?);
            return Some(text(&out));
        }
        out.push(u8::try_from(da).ok()?);
        upper_open = true;
    }
    None
}

fn after(l: &str) -> Option<String> {
    let d = digits(l);
    let v = prefix_value(&d, STEP_WIDTH);
    let max = BASE.pow(u32::try_from(STEP_WIDTH).unwrap_or(6));
    match v.checked_add(STEP) {
        Some(n) if n < max => Some(text(&from_value(n, STEP_WIDTH))),
        _ => midpoint(&d, None),
    }
}

fn before(h: &str) -> Option<String> {
    let d = digits(h);
    let v = prefix_value(&d, STEP_WIDTH);
    if v > STEP { Some(text(&from_value(v - STEP, STEP_WIDTH))) } else { midpoint(&[], Some(&d)) }
}

/// A rank strictly between `lo` and `hi` (`None` = open end). `None` when an input is malformed
/// or `lo >= hi`. The result may be longer than [`MAX_RANK_LEN`]; callers rebalance then.
pub fn rank_between(lo: Option<&str>, hi: Option<&str>) -> Option<String> {
    if lo.is_some_and(|l| !well_formed(l, MAX_INPUT_LEN)) || hi.is_some_and(|h| !well_formed(h, MAX_INPUT_LEN)) {
        return None;
    }
    match (lo, hi) {
        (None, None) => Some("i".into()),
        (Some(l), None) => after(l),
        (None, Some(h)) => before(h),
        (Some(l), Some(h)) if l < h => midpoint(&digits(l), Some(&digits(h))),
        _ => None,
    }
}

/// `n` increasing ranks strictly between `lo` and `hi`, or `None` when they don't fit in
/// [`MAX_RANK_LEN`] (the caller rebalances).
pub fn ranks_between(lo: Option<&str>, hi: Option<&str>, n: usize) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(n);
    match (lo, hi) {
        (_, None) => {
            let mut prev = lo.map(str::to_string);
            for _ in 0..n {
                let r = rank_between(prev.as_deref(), None)?;
                out.push(r.clone());
                prev = Some(r);
            }
        }
        (None, Some(_)) => {
            let mut next = hi.map(str::to_string);
            for _ in 0..n {
                let r = rank_between(None, next.as_deref())?;
                out.push(r.clone());
                next = Some(r);
            }
            out.reverse();
        }
        (Some(_), Some(_)) => fill(lo, hi, n, &mut out)?,
    }
    out.iter().all(|r| is_valid_rank(r)).then_some(out)
}

/// Bisection: the middle item takes the midpoint, each half recurses (depth log2(n)).
fn fill(lo: Option<&str>, hi: Option<&str>, n: usize, out: &mut Vec<String>) -> Option<()> {
    if n == 0 {
        return Some(());
    }
    let mid = rank_between(lo, hi)?;
    if mid.len() > MAX_RANK_LEN {
        return None;
    }
    let left = n / 2;
    fill(lo, Some(&mid), left, out)?;
    out.push(mid.clone());
    fill(Some(&mid), hi, n.saturating_sub(left).saturating_sub(1), out)
}

/// `n` fresh ranks, evenly spaced through the middle third of the space (room left at both
/// ends for appends and prepends), all at most [`MAX_RANK_LEN`] characters.
pub fn rebalanced(n: usize) -> Vec<String> {
    let n128 = u128::try_from(n).unwrap_or(u128::MAX / 2).saturating_add(1);
    let mut w: u32 = 6;
    while w < 24 && (36u128.pow(w) / 3) / n128 < 36 * 36 {
        w += 1;
    }
    let space = 36u128.pow(w);
    let base = space / 3;
    let gap = (base / n128).max(1);
    let width = usize::try_from(w).unwrap_or(6);
    (0..n)
        .map(|i| {
            let mut v = base.saturating_add(gap.saturating_mul(u128::try_from(i).unwrap_or(0).saturating_add(1)));
            let mut d = vec![0u8; width];
            for slot in d.iter_mut().rev() {
                *slot = u8::try_from(v % 36).unwrap_or(0);
                v /= 36;
            }
            text(&d)
        })
        .collect()
}
