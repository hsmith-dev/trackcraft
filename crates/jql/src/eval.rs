//! Evaluation against any [`Record`].
//!
//! Text comparison is case-insensitive (as observed in Jira). Dates compare as UTC instants;
//! values may be `2026-10-31`, `2026-10-31 14:00`, relative offsets (`-7d`, `2w`, `-4h`, `30m`)
//! or the date functions.

use std::cmp::Ordering;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};

use crate::{Dir, Expr, History, HistoryOp, JqlError, Op, Operand, Predicate, Query};

/// What a record reports for a field.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    /// The record has no such field or it is empty.
    None,
    Text(String),
    /// Multi-value fields (labels, components, fix versions).
    Many(Vec<String>),
    Number(f64),
    Time(DateTime<Utc>),
    /// Ordered choice (priority, status category): rank for `<`/`>` and sorting, name for `=`.
    Ranked {
        name: String,
        rank: i64,
    },
}

/// One change of a field's value, for `WAS` and `CHANGED`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange {
    pub at: DateTime<Utc>,
    /// Who made the change (a user id, compared case-insensitively with `BY`).
    pub by: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// A field's value over time: the value it started with, then each change in time order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FieldHistory {
    /// When the item came into being (the start of the first value); `None` = unknown.
    pub created: Option<DateTime<Utc>>,
    /// Who set the first value (the creator), for `WAS … BY`.
    pub created_by: Option<String>,
    /// The value the item started with (`None` = empty).
    pub initial: Option<String>,
    /// Changes, oldest first.
    pub changes: Vec<FieldChange>,
}

/// A searchable item.
pub trait Record {
    fn field(&self, name: &str) -> FieldValue;
    /// Free text for `text ~` (summary, description, comments).
    fn text(&self) -> String;
    /// The value history of `name` for `WAS`/`CHANGED`; `None` when the field keeps no history
    /// (the clause is then an error).
    fn history(&self, _name: &str) -> Option<FieldHistory> {
        None
    }
}

/// Evaluation context: the searcher and the clock (passed in, so evaluation is deterministic).
#[derive(Debug, Clone)]
pub struct Context {
    pub current_user: Option<String>,
    pub now: DateTime<Utc>,
}

pub fn matches(q: &Query, r: &dyn Record, cx: &Context) -> Result<bool, JqlError> {
    match &q.filter {
        None => Ok(true),
        Some(e) => eval(e, r, cx),
    }
}

/// Sorts by the query's `ORDER BY` (stable; ties keep input order).
pub fn sort<R: Record>(q: &Query, items: &mut [R]) {
    if q.order.is_empty() {
        return;
    }
    items.sort_by(|a, b| {
        for (f, dir) in &q.order {
            let o = cmp_values(&a.field(f), &b.field(f));
            let o = if *dir == Dir::Desc { o.reverse() } else { o };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
}

fn eval(e: &Expr, r: &dyn Record, cx: &Context) -> Result<bool, JqlError> {
    Ok(match e {
        Expr::And(a, b) => eval(a, r, cx)? && eval(b, r, cx)?,
        Expr::Or(a, b) => eval(a, r, cx)? || eval(b, r, cx)?,
        Expr::Not(a) => !eval(a, r, cx)?,
        Expr::Clause { field, op, value } => clause(field, *op, value, r, cx)?,
        Expr::History(h) => history(h, r, cx)?,
    })
}

/// A resolved time window `[start, end)`; `None` bounds are open.
#[derive(Clone, Copy)]
struct Window {
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
}

impl Window {
    fn overlaps(&self, other: &Window) -> bool {
        let starts_before_end = match (self.start, other.end) {
            (Some(s), Some(e)) => s < e,
            _ => true,
        };
        let ends_after_start = match (self.end, other.start) {
            (Some(e), Some(s)) => e > s,
            _ => true,
        };
        starts_before_end && ends_after_start
    }
    fn contains(&self, t: DateTime<Utc>) -> bool {
        self.start.is_none_or(|s| t >= s) && self.end.is_none_or(|e| t < e)
    }
}

/// The predicates of one clause, resolved once per record (dates against `cx.now`).
struct Resolved {
    windows: Vec<Window>,
    by: Vec<Vec<String>>,
    from: Vec<Operand>,
    to: Vec<Operand>,
}

fn time_of(op: &Operand, cx: &Context) -> Result<DateTime<Utc>, JqlError> {
    let describe = || match op {
        Operand::Value(s) => s.chars().take(40).collect(),
        Operand::Func(f, _) => format!("{f}()"),
        Operand::Empty => "EMPTY".into(),
        Operand::List(_) => "a list".into(),
    };
    to_time(op, cx)?.ok_or_else(|| JqlError::BadDate(describe()))
}

fn is_bare_date(op: &Operand) -> bool {
    matches!(op, Operand::Value(s) if NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").is_ok())
}

/// `[midnight, next midnight)` of the day holding `t`.
fn day_of(t: DateTime<Utc>) -> Window {
    let start = t.date_naive().and_hms_opt(0, 0, 0).map(|d| d.and_utc());
    Window { start, end: start.and_then(|s| s.checked_add_signed(Duration::days(1))) }
}

fn resolve(preds: &[Predicate], cx: &Context) -> Result<Resolved, JqlError> {
    let mut r = Resolved { windows: vec![], by: vec![], from: vec![], to: vec![] };
    for p in preds {
        match p {
            // AFTER/BEFORE/DURING bounds are exclusive of the instant itself, as in Jira's examples
            // (`AFTER "2026/10/01"` means after the start of that day).
            Predicate::After(d) => r.windows.push(Window { start: Some(time_of(d, cx)?), end: None }),
            Predicate::Before(d) => r.windows.push(Window { start: None, end: Some(time_of(d, cx)?) }),
            Predicate::On(d) => r.windows.push(day_of(time_of(d, cx)?)),
            Predicate::During(a, b) => {
                let start = time_of(a, cx)?;
                let b_time = time_of(b, cx)?;
                // A bare end date includes that whole day.
                let end = if is_bare_date(b) { day_of(b_time).end } else { b_time.checked_add_signed(Duration::seconds(1)) };
                r.windows.push(Window { start: Some(start), end });
            }
            Predicate::By(users) => r.by.push(users.iter().map(|u| resolve_text(u, cx)).collect::<Result<_, _>>()?),
            Predicate::From(v) => r.from.push(v.clone()),
            Predicate::To(v) => r.to.push(v.clone()),
        }
    }
    Ok(r)
}

fn value_is(have: Option<&str>, want: &Operand, cx: &Context) -> Result<bool, JqlError> {
    if *want == Operand::Empty {
        return Ok(have.is_none_or(str::is_empty));
    }
    let want = resolve_text(want, cx)?;
    Ok(have.is_some_and(|h| h.eq_ignore_ascii_case(&want)))
}

fn by_matches(by: &[Vec<String>], who: Option<&str>) -> bool {
    by.iter().all(|users| who.is_some_and(|w| users.iter().any(|u| u.eq_ignore_ascii_case(w))))
}

fn history(h: &History, r: &dyn Record, cx: &Context) -> Result<bool, JqlError> {
    let unsupported = || JqlError::HistoryUnsupported { op: h.op.as_str(), field: h.field.chars().take(80).collect() };
    let fh = r.history(&h.field).ok_or_else(unsupported)?;
    let p = resolve(&h.predicates, cx)?;
    if h.op == HistoryOp::Changed {
        for c in &fh.changes {
            if !p.windows.iter().all(|w| w.contains(c.at)) || !by_matches(&p.by, Some(&c.by)) {
                continue;
            }
            let mut ok = true;
            for v in &p.from {
                ok = ok && value_is(c.from.as_deref(), v, cx)?;
            }
            for v in &p.to {
                ok = ok && value_is(c.to.as_deref(), v, cx)?;
            }
            if ok {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    // The spans the field held each value: [created, first change), then [change, next change)….
    let mut spans: Vec<(Window, Option<&str>, Option<&str>)> = Vec::with_capacity(fh.changes.len().saturating_add(1));
    let first_end = fh.changes.first().map(|c| c.at);
    spans.push((Window { start: fh.created, end: first_end }, fh.initial.as_deref(), fh.created_by.as_deref()));
    for (i, c) in fh.changes.iter().enumerate() {
        let end = fh.changes.get(i.saturating_add(1)).map(|n| n.at);
        spans.push((Window { start: Some(c.at), end }, c.to.as_deref(), Some(c.by.as_str())));
    }
    let wanted: Vec<&Operand> = match &h.value {
        Some(Operand::List(items)) => items.iter().collect(),
        Some(v) => vec![v],
        None => vec![],
    };
    let mut was = false;
    'spans: for (span, value, setter) in &spans {
        if !p.windows.iter().all(|w| span.overlaps(w)) || !by_matches(&p.by, *setter) {
            continue;
        }
        for want in &wanted {
            if value_is(*value, want, cx)? {
                was = true;
                break 'spans;
            }
        }
    }
    Ok(match h.op {
        HistoryOp::Was | HistoryOp::WasIn => was,
        _ => !was,
    })
}

fn clause(field: &str, op: Op, value: &Operand, r: &dyn Record, cx: &Context) -> Result<bool, JqlError> {
    let is_text_field = field.eq_ignore_ascii_case("text");
    let fv = if is_text_field { FieldValue::Text(r.text()) } else { r.field(field) };
    let empty = match &fv {
        FieldValue::None => true,
        FieldValue::Text(s) => s.is_empty(),
        FieldValue::Many(v) => v.is_empty(),
        _ => false,
    };
    Ok(match op {
        Op::Is => empty,
        Op::IsNot => !empty,
        Op::In | Op::NotIn => {
            let Operand::List(items) = value else { return Ok(false) };
            let mut hit = false;
            for it in items {
                if equals(&fv, it, cx)? {
                    hit = true;
                    break;
                }
            }
            if op == Op::In { hit } else { !hit && !empty }
        }
        Op::Eq if *value == Operand::Empty => empty,
        Op::Ne if *value == Operand::Empty => !empty,
        Op::Eq => equals(&fv, value, cx)?,
        // Jira's `!=` does not match items whose field is empty.
        Op::Ne => !empty && !equals(&fv, value, cx)?,
        Op::Contains | Op::NotContains => {
            let needle = resolve_text(value, cx)?.to_lowercase();
            let hay = match &fv {
                FieldValue::Text(s) => s.to_lowercase(),
                FieldValue::Many(v) => v.join(" ").to_lowercase(),
                FieldValue::Ranked { name, .. } => name.to_lowercase(),
                _ => String::new(),
            };
            let hit = needle.split_whitespace().all(|w| hay.contains(w.trim_matches('*')));
            if op == Op::Contains { hit } else { !hit }
        }
        Op::Gt | Op::Ge | Op::Lt | Op::Le => {
            if empty {
                return Ok(false);
            }
            let Some(o) = ordered(&fv, value, cx)? else { return Ok(false) };
            match op {
                Op::Gt => o == Ordering::Greater,
                Op::Ge => o != Ordering::Less,
                Op::Lt => o == Ordering::Less,
                _ => o != Ordering::Greater,
            }
        }
    })
}

fn resolve_text(v: &Operand, cx: &Context) -> Result<String, JqlError> {
    match v {
        Operand::Value(s) => Ok(s.clone()),
        Operand::Func(name, _) if name.eq_ignore_ascii_case("currentUser") => Ok(cx.current_user.clone().unwrap_or_default()),
        Operand::Func(name, _) => match date_func(name, cx) {
            Some(t) => Ok(t.to_rfc3339()),
            None => Err(JqlError::UnknownFunction(name.clone())),
        },
        Operand::Empty => Ok(String::new()),
        Operand::List(_) => Err(JqlError::BadOperand { op: "=", value: "a list".into() }),
    }
}

fn equals(fv: &FieldValue, v: &Operand, cx: &Context) -> Result<bool, JqlError> {
    if *v == Operand::Empty {
        return Ok(matches!(fv, FieldValue::None) || fv == &FieldValue::Text(String::new()) || fv == &FieldValue::Many(vec![]));
    }
    if let FieldValue::Time(t) = fv {
        let Some(want) = to_time(v, cx)? else { return Ok(false) };
        // A bare date equals the whole day.
        if let Operand::Value(s) = v
            && NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
        {
            return Ok(t.date_naive() == want.date_naive());
        }
        return Ok(*t == want);
    }
    let want = resolve_text(v, cx)?;
    Ok(match fv {
        FieldValue::None => false,
        FieldValue::Text(s) => s.eq_ignore_ascii_case(&want),
        FieldValue::Many(list) => list.iter().any(|s| s.eq_ignore_ascii_case(&want)),
        FieldValue::Number(n) => want.parse::<f64>().is_ok_and(|w| (w - n).abs() < f64::EPSILON),
        FieldValue::Ranked { name, .. } => name.eq_ignore_ascii_case(&want),
        FieldValue::Time(_) => false,
    })
}

fn ordered(fv: &FieldValue, v: &Operand, cx: &Context) -> Result<Option<Ordering>, JqlError> {
    Ok(match fv {
        FieldValue::Time(t) => to_time(v, cx)?.map(|w| t.cmp(&w)),
        FieldValue::Number(n) => resolve_text(v, cx)?.parse::<f64>().ok().and_then(|w| n.partial_cmp(&w)),
        // Comparing a ranked field needs the other side's rank, which only the record knows; the
        // model passes ranks as numbers (`priority > 2`) or names it resolves itself.
        FieldValue::Ranked { rank, .. } => resolve_text(v, cx)?.parse::<i64>().ok().map(|w| rank.cmp(&w)),
        FieldValue::Text(s) => Some(s.to_lowercase().cmp(&resolve_text(v, cx)?.to_lowercase())),
        FieldValue::Many(_) | FieldValue::None => None,
    })
}

fn date_func(name: &str, cx: &Context) -> Option<DateTime<Utc>> {
    let day = cx.now.date_naive();
    match name.to_ascii_lowercase().as_str() {
        "now" => Some(cx.now),
        "startofday" => day.and_hms_opt(0, 0, 0).map(|d| d.and_utc()),
        "endofday" => day.and_hms_opt(23, 59, 59).map(|d| d.and_utc()),
        _ => None,
    }
}

fn to_time(v: &Operand, cx: &Context) -> Result<Option<DateTime<Utc>>, JqlError> {
    let s = match v {
        Operand::Func(name, _) => return date_func(name, cx).map(Some).ok_or_else(|| JqlError::UnknownFunction(name.clone())),
        Operand::Value(s) => s.trim(),
        _ => return Ok(None),
    };
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).map(|d| d.and_utc()));
    }
    for fmt in ["%Y-%m-%d %H:%M", "%Y/%m/%d %H:%M", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(Some(d.and_utc()));
        }
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Ok(Some(d.with_timezone(&Utc)));
    }
    Ok(relative(s).and_then(|d| cx.now.checked_add_signed(d)))
}

/// `-7d`, `2w`, `-4h`, `30m`, `+1d` (Jira's relative date syntax).
fn relative(s: &str) -> Option<Duration> {
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1i64, r),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let unit = rest.chars().last()?;
    let n: i64 = rest.get(..rest.len().saturating_sub(unit.len_utf8()))?.parse().ok()?;
    let n = n.checked_mul(sign)?;
    match unit {
        'w' => Duration::try_weeks(n),
        'd' => Duration::try_days(n),
        'h' => Duration::try_hours(n),
        'm' => Duration::try_minutes(n),
        _ => None,
    }
}

fn cmp_values(a: &FieldValue, b: &FieldValue) -> Ordering {
    use FieldValue as F;
    match (a, b) {
        // Empty values sort last ascending, as observed in Jira.
        (F::None, F::None) => Ordering::Equal,
        (F::None, _) => Ordering::Greater,
        (_, F::None) => Ordering::Less,
        (F::Number(x), F::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (F::Time(x), F::Time(y)) => x.cmp(y),
        (F::Ranked { rank: x, .. }, F::Ranked { rank: y, .. }) => x.cmp(y),
        (F::Text(x), F::Text(y)) => natural(x, y),
        (F::Many(x), F::Many(y)) => x.join(",").to_lowercase().cmp(&y.join(",").to_lowercase()),
        _ => Ordering::Equal,
    }
}

/// Case-insensitive order that compares embedded numbers numerically (`ATLAS-9` < `ATLAS-10`).
fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = x.next_if(char::is_ascii_digit) {
                    na.push(c);
                }
                let mut nb = String::new();
                while let Some(c) = y.next_if(char::is_ascii_digit) {
                    nb.push(c);
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    struct It {
        key: &'static str,
        status: &'static str,
        assignee: Option<&'static str>,
        labels: Vec<&'static str>,
        points: f64,
        created: DateTime<Utc>,
    }

    /// Status history of each test item: (initial, [(days ago, by, from, to)]).
    fn status_history(key: &str) -> Option<FieldHistory> {
        let ch = |d: i64, by: &str, from: &str, to: &str| FieldChange {
            at: now() - Duration::days(d),
            by: by.into(),
            from: Some(from.into()),
            to: Some(to.into()),
        };
        let (created, changes) = match key {
            "A-10" => (30, vec![ch(20, "ann", "To Do", "In Progress"), ch(10, "bob", "In Progress", "Done")]),
            "A-9" => (2, vec![]),
            "A-2" => (0, vec![]),
            _ => return None,
        };
        let initial = if changes.is_empty() { None } else { Some("To Do".to_string()) };
        Some(FieldHistory { created: Some(now() - Duration::days(created)), created_by: Some("ann".into()), initial, changes })
    }

    impl Record for It {
        fn field(&self, name: &str) -> FieldValue {
            match name.to_ascii_lowercase().as_str() {
                "key" => FieldValue::Text(self.key.into()),
                "status" => FieldValue::Text(self.status.into()),
                "assignee" => self.assignee.map_or(FieldValue::None, |a| FieldValue::Text(a.into())),
                "labels" => FieldValue::Many(self.labels.iter().map(|s| s.to_string()).collect()),
                "story points" => FieldValue::Number(self.points),
                "created" => FieldValue::Time(self.created),
                _ => FieldValue::None,
            }
        }
        fn text(&self) -> String {
            format!("{} fix the login bug", self.key)
        }
        fn history(&self, name: &str) -> Option<FieldHistory> {
            if !name.eq_ignore_ascii_case("status") {
                return None;
            }
            let mut h = status_history(self.key)?;
            if h.initial.is_none() {
                h.initial = Some(self.status.into());
            }
            Some(h)
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-31T12:00:00Z").map(|d| d.with_timezone(&Utc)).unwrap()
    }

    fn items() -> Vec<It> {
        vec![
            It { key: "A-10", status: "Done", assignee: Some("ann"), labels: vec!["ui"], points: 3.0, created: now() - Duration::days(30) },
            It { key: "A-9", status: "To Do", assignee: None, labels: vec![], points: 1.0, created: now() - Duration::days(2) },
            It { key: "A-2", status: "In Progress", assignee: Some("bob"), labels: vec!["ui", "api"], points: 5.0, created: now() },
        ]
    }

    fn keys(q: &str) -> Vec<&'static str> {
        let cx = Context { current_user: Some("bob".into()), now: now() };
        let q = parse(q).unwrap();
        let mut hits: Vec<It> = items().into_iter().filter(|i| matches(&q, i, &cx).unwrap()).collect();
        sort(&q, &mut hits);
        hits.iter().map(|i| i.key).collect()
    }

    #[test]
    fn evaluates() {
        assert_eq!(keys("status = done"), ["A-10"]);
        assert_eq!(keys("status != Done"), ["A-9", "A-2"]);
        assert_eq!(keys("assignee != ann"), ["A-2"]);
        assert_eq!(keys("assignee IS EMPTY"), ["A-9"]);
        assert_eq!(keys("assignee = currentUser()"), ["A-2"]);
        assert_eq!(keys("labels = ui ORDER BY key"), ["A-2", "A-10"]);
        assert_eq!(keys("labels NOT IN (api)"), ["A-10"]);
        assert_eq!(keys("\"story points\" >= 3 ORDER BY \"story points\" DESC"), ["A-2", "A-10"]);
        assert_eq!(keys("created >= -7d ORDER BY created"), ["A-9", "A-2"]);
        assert_eq!(keys("created < 2026-10-02"), ["A-10"]);
        assert_eq!(keys("text ~ \"login bug\" AND NOT status = done ORDER BY key DESC"), ["A-9", "A-2"]);
        assert_eq!(keys("ORDER BY key"), ["A-2", "A-9", "A-10"]);
    }

    #[test]
    fn history_operators() {
        assert_eq!(keys(r#"status WAS "In Progress" ORDER BY key"#), ["A-2", "A-10"]);
        assert_eq!(keys(r#"status WAS "To Do" ORDER BY key"#), ["A-9", "A-10"]);
        assert_eq!(keys(r#"status WAS NOT "To Do""#), ["A-2"]);
        assert_eq!(keys(r#"status WAS IN ("Done", "To Do") ORDER BY key"#), ["A-9", "A-10"]);
        assert_eq!(keys(r#"status WAS NOT IN ("Done", "To Do")"#), ["A-2"]);
        assert_eq!(keys(r#"status WAS "In Progress" BY ann ORDER BY key"#), ["A-2", "A-10"]);
        assert_eq!(keys(r#"status WAS Done BY bob"#), ["A-10"]);
        assert_eq!(keys(r#"status WAS "In Progress" BY (bob, sam)"#), [] as [&str; 0]);
        assert_eq!(keys(r#"status WAS "In Progress" BEFORE -25d"#), [] as [&str; 0]);
        assert_eq!(keys(r#"status WAS "In Progress" DURING (-15d, -12d)"#), ["A-10"]);
        assert_eq!(keys(r#"status WAS "Done" AFTER -1d"#), ["A-10"], "still Done now");
        assert_eq!(keys(r#"status WAS "In Progress" ON 2026-10-16"#), ["A-10"]);
        assert_eq!(keys(r#"status WAS "In Progress" ON 2026-10-01"#), [] as [&str; 0], "before it moved");
        assert_eq!(keys("status CHANGED"), ["A-10"]);
        assert_eq!(keys(r#"status CHANGED FROM "To Do" TO "In Progress""#), ["A-10"]);
        assert_eq!(keys(r#"status CHANGED FROM "To Do" TO "Done""#), [] as [&str; 0]);
        assert_eq!(keys("status CHANGED BY bob AFTER -11d BEFORE -9d"), ["A-10"]);
        assert_eq!(keys("status CHANGED BY currentUser() ON 2026-10-21"), ["A-10"]);
        assert_eq!(keys("status CHANGED BY ann ON 2026-10-21"), [] as [&str; 0]);
        assert_eq!(keys("status CHANGED DURING (2026-10-12, 2026-10-20)"), [] as [&str; 0]);
        assert_eq!(keys("status CHANGED DURING (2026-10-21, 2026-10-21)"), ["A-10"]);
        assert_eq!(keys("NOT status CHANGED ORDER BY key"), ["A-2", "A-9"]);
        assert_eq!(keys("status CHANGED TO EMPTY"), [] as [&str; 0]);
    }

    #[test]
    fn history_errors() {
        let cx = Context { current_user: None, now: now() };
        for (q, want) in [
            ("labels WAS ui", "not supported for field 'labels'"),
            ("labels CHANGED", "CHANGED operator is not supported"),
            ("status CHANGED AFTER yesterday", "is not a date"),
            ("status WAS a DURING (-1d, nope())", "unknown function"),
            ("status WAS a BY membersOf(x)", "unknown function"),
        ] {
            let q = parse(q).unwrap();
            let e = matches(&q, &items()[0], &cx).unwrap_err().to_string();
            assert!(e.contains(want), "{e}");
        }
    }

    #[test]
    fn huge_history_and_long_queries_stay_bounded() {
        struct Busy;
        impl Record for Busy {
            fn field(&self, _: &str) -> FieldValue {
                FieldValue::None
            }
            fn text(&self) -> String {
                String::new()
            }
            fn history(&self, _: &str) -> Option<FieldHistory> {
                let changes = (0..20_000)
                    .map(|i| FieldChange {
                        at: now() - Duration::minutes(i),
                        by: "x".into(),
                        from: Some(format!("s{}", i + 1)),
                        to: Some(format!("s{i}")),
                    })
                    .rev()
                    .collect();
                Some(FieldHistory { created: None, created_by: None, initial: Some("s20000".into()), changes })
            }
        }
        let cx = Context { current_user: None, now: now() };
        let list = (0..5_000).map(|i| format!("v{i}")).collect::<Vec<_>>().join(",");
        let q = parse(&format!("status WAS NOT IN ({list}) AND status CHANGED FROM s5 TO s4")).unwrap();
        assert!(matches(&q, &Busy, &cx).unwrap());
        let q = parse(&vec!["status WAS s1"; 200].join(" AND ")).unwrap();
        assert!(matches(&q, &Busy, &cx).unwrap());
    }

    #[test]
    fn long_flat_chains_do_not_overflow() {
        let cx = Context { current_user: None, now: now() };
        let q = vec!["status WAS x"; 4_000].join(" OR ");
        assert!(q.len() < crate::MAX_QUERY_LEN);
        let q = parse(&q).unwrap();
        assert!(!matches(&q, &items()[0], &cx).unwrap());
    }

    #[test]
    fn unknown_function_is_an_error() {
        let cx = Context { current_user: None, now: now() };
        let q = parse("assignee = membersOf(x)").unwrap();
        assert!(matches(&q, &items()[0], &cx).is_err());
        assert_eq!(relative("-99999999999999999d"), None);
    }
}
