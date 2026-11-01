//! Keyboard-first navigation: one scoped keymap, its dispatcher, the per-view key actions and the
//! `?` help overlay.
//!
//! The keymap ([`table`]) is generated from the engine registry's `CommandSpec::shortcut`
//! strings plus the UI-only bindings in [`UI_BINDINGS`]. The overlay renders that same table and
//! the dispatcher reads it, so the two cannot drift.
//!
//! Scope comes from the command id (`issue.*` → [`Scope::Issue`], `page.*` → [`Scope::Page`],
//! `search.*` → [`Scope::List`], `board.*` → [`Scope::Board`], anything else global), with
//! [`OVERRIDES`] for the exceptions. The current view's scope is tried before the global one.
//!
//! Dispatch rules ([`Gate`]): unmodified keys (no `Mod`/`Alt`) never fire while a text field has
//! focus or a dialog is open; modified ones still fire, except undo and redo, which belong to the
//! text field while typing. Nothing but saving fires behind the "Discard your changes?" dialog.

use std::cmp::Reverse;
use std::sync::OnceLock;

use egui::{InputState, Key, Modifiers, RichText};
use serde_json::json;
use trackcraft_core::{IssueKey, ProjectKey};

use crate::theme::Tokens;
use crate::{Action, FileRequest, Then, TrackApp, View, issues};

/// How long the second key of a `G then X` chord may wait, in seconds.
pub(crate) const CHORD_TIMEOUT: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Scope {
    Global,
    List,
    Board,
    Issue,
    Page,
    Backlog,
}

impl Scope {
    pub(crate) const ALL: [Scope; 6] = [Scope::Global, Scope::List, Scope::Board, Scope::Issue, Scope::Page, Scope::Backlog];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Scope::Global => "Everywhere",
            Scope::List => "Issue list",
            Scope::Board => "Board",
            Scope::Issue => "Issue",
            Scope::Page => "Page",
            Scope::Backlog => "Backlog",
        }
    }

    /// The scope a registry command's shortcut belongs to, from its id.
    pub(crate) fn of_command(id: &str) -> Scope {
        if let Some((_, Some(s), _, _)) = OVERRIDES.iter().find(|o| o.0 == id) {
            return *s;
        }
        match id.split('.').next().unwrap_or_default() {
            "issue" => Scope::Issue,
            "page" | "space" => Scope::Page,
            "search" => Scope::List,
            "board" => Scope::Board,
            _ => Scope::Global,
        }
    }
}

/// One key press: `Mod` is Cmd on macOS and Ctrl elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chord {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Chord {
    /// Parses `Mod+Shift+Z`, `C`, `F7`, `/`, `?`, `Alt+Left`, `Mod++`. `None` for anything else.
    pub(crate) fn parse(s: &str) -> Option<Chord> {
        let s = s.trim();
        if s.is_empty() || s.len() > 40 {
            return None;
        }
        // A trailing "+" is the Plus key ("Mod++").
        let (mods, key) = if s == "+" {
            ("", "+")
        } else if let Some(m) = s.strip_suffix("++") {
            (m, "+")
        } else {
            s.rsplit_once('+').unwrap_or(("", s))
        };
        let mut c = Chord { command: false, shift: false, alt: false, key: Key::from_name(key.trim())? };
        for m in mods.split('+').map(str::trim).filter(|m| !m.is_empty()) {
            match m.to_ascii_lowercase().as_str() {
                "mod" | "cmd" | "ctrl" | "command" | "meta" => c.command = true,
                "shift" => c.shift = true,
                "alt" | "option" | "opt" => c.alt = true,
                _ => return None,
            }
        }
        Some(c)
    }

    pub(crate) fn modifiers(self) -> Modifiers {
        let mut m = Modifiers::NONE;
        if self.command {
            m |= Modifiers::COMMAND;
        }
        if self.shift {
            m |= Modifiers::SHIFT;
        }
        if self.alt {
            m |= Modifiers::ALT;
        }
        m
    }

    /// No `Mod` or `Alt`: a key that a focused text field would type (Shift only changes case).
    pub(crate) fn plain(self) -> bool {
        !self.command && !self.alt
    }

    /// More modifiers first: egui's matching ignores extra Shift/Alt, so `Mod+Z` would also
    /// match `Mod+Shift+Z`.
    fn weight(self) -> usize {
        usize::from(self.command) + usize::from(self.shift) + usize::from(self.alt)
    }
}

/// `"J / Down"` → alternatives; `"G then H"` → a two-key sequence. `None` if any part fails.
pub(crate) fn parse_keys(text: &str) -> Option<Vec<Vec<Chord>>> {
    text.split(" / ").map(|alt| alt.split(" then ").map(Chord::parse).collect::<Option<Vec<_>>>()).collect()
}

/// What a binding does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    /// A registry command, by id.
    Cmd(&'static str),
    Help,
    Back,
    SaveAs,
    ListNext,
    ListPrev,
    ListOpen,
    BoardLeft,
    BoardRight,
    BoardUp,
    BoardDown,
    BoardOpen,
    BoardNext,
    BoardPrev,
    GoHome,
    GoIssues,
    GoBoard,
    GoSpace,
    /// Listed in the overlay, handled where the widget lives (Escape, header clicks).
    Doc,
    /// Undo a Back (UI-8).
    Forward,
    /// Open Settings (UI-7).
    Settings,
    /// Peek at the selected issue (fixes).
    Peek,
    /// Open the Inbox (M4.5).
    GoInbox,
}

#[derive(Debug, Clone)]
pub(crate) struct Binding {
    pub scope: Scope,
    /// The keys as written (`Mod+K / Mod+Shift+P`); the overlay shows them through `crate::key`.
    pub keys: String,
    pub label: String,
    pub act: Act,
    /// Parsed `keys`; empty for documentation-only rows.
    pub seqs: Vec<Vec<Chord>>,
}

/// Registry commands whose UI scope, extra keys or wording differ from the defaults:
/// (id, scope override, extra keys shown first, label in the overlay).
const OVERRIDES: &[(&str, Option<Scope>, &str, &str)] = &[
    ("app.commands", None, "Mod+K", "Command palette"),
    ("issue.create", Some(Scope::Global), "", "Create an issue"),
    ("search.jql", Some(Scope::Global), "", "Search with JQL"),
    ("edit.undo", None, "", "Undo"),
    ("edit.redo", None, "", "Redo"),
    ("file.save", None, "", "Save"),
    ("file.open", None, "", "Open a workspace"),
    ("issue.edit", None, "", "Edit the description"),
    ("issue.assign", None, "", "Change the assignee"),
    ("issue.assignToMe", None, "", "Assign to me"),
    ("issue.comment", None, "", "Write a comment"),
    ("issue.transition", None, "T", "Change the status"),
    ("page.create", None, "", "Create a child page"),
    ("page.edit", None, "", "Edit the page"),
    ("page.labels", None, "", "Edit the labels"),
];

/// UI-only bindings: (scope, keys, label, action).
pub(crate) const UI_BINDINGS: &[(Scope, &str, &str, Act)] = &[
    (Scope::Global, "?", "Keyboard shortcuts", Act::Help),
    (Scope::Global, "Alt+Left", "Back", Act::Back),
    (Scope::Global, "Mod+Shift+S", "Save as", Act::SaveAs),
    (Scope::Global, "G then H", "Go to Home", Act::GoHome),
    (Scope::Global, "G then I", "Go to Issues", Act::GoIssues),
    (Scope::Global, "G then B", "Go to the board", Act::GoBoard),
    (Scope::Global, "G then P", "Go to the space home", Act::GoSpace),
    (Scope::Global, "Escape", "Close a dialog, cancel an edit", Act::Doc),
    // The backlog handles its own keys (`backlog::keys`); listed here for the overlay.
    (Scope::Backlog, "J / Down", "Next issue", Act::Doc),
    (Scope::Backlog, "K / Up", "Previous issue", Act::Doc),
    (Scope::Backlog, "Enter / O", "Open the selected issue", Act::Doc),
    (Scope::Backlog, "Mod+Shift+Up / Mod+Shift+Down", "Rank the issue up or down one step", Act::Doc),
    (Scope::Backlog, "Mod+Shift+Home / Mod+Shift+End", "Rank to the top or bottom of its section", Act::Doc),
    (Scope::Backlog, "Shift+S", "Move to sprint…", Act::Doc),
    (Scope::List, "J / Down", "Next issue", Act::ListNext),
    (Scope::List, "K / Up", "Previous issue", Act::ListPrev),
    (Scope::List, "Enter", "Open the selected issue", Act::ListOpen),
    (Scope::List, "Click a header", "Sort by that column", Act::Doc),
    (Scope::Board, "H / Left", "Card to the left", Act::BoardLeft),
    (Scope::Board, "L / Right", "Card to the right", Act::BoardRight),
    (Scope::Board, "K / Up", "Card above", Act::BoardUp),
    (Scope::Board, "J / Down", "Card below", Act::BoardDown),
    (Scope::Board, "Enter", "Open the selected card", Act::BoardOpen),
    (Scope::Board, "Shift+Right / ]", "Move the card to the next status", Act::BoardNext),
    (Scope::Board, "Shift+Left / [", "Move the card to the previous status", Act::BoardPrev),
    (Scope::Issue, "Escape", "Cancel editing", Act::Doc),
    (Scope::Global, "Alt+Right", "Forward", Act::Forward),              // UI-8
    (Scope::Global, "Mod+,", "Settings", Act::Settings),                // UI-7
    (Scope::List, "Space", "Peek at the selected issue", Act::Peek),    // fixes
    (Scope::Board, "Space", "Peek at the selected card", Act::Peek),    // fixes
    (Scope::Backlog, "Space", "Peek at the selected issue", Act::Peek), // fixes
    (Scope::Global, "G then N", "Go to notifications", Act::GoInbox),   // M4.5
];

/// The keymap: every registry shortcut, then the UI-only bindings.
pub(crate) fn table() -> &'static [Binding] {
    static TABLE: OnceLock<Vec<Binding>> = OnceLock::new();
    TABLE.get_or_init(build)
}

fn build() -> Vec<Binding> {
    let mut out = Vec::new();
    for c in trackcraft_engine::commands() {
        let o = OVERRIDES.iter().find(|o| o.0 == c.id);
        let extra = o.map(|o| o.2).unwrap_or_default();
        let mut alts: Vec<&str> = Vec::new();
        for a in extra.split(" / ").chain(c.shortcut.split(" / ")).map(str::trim).filter(|a| !a.is_empty()) {
            if !alts.contains(&a) {
                alts.push(a);
            }
        }
        if alts.is_empty() {
            continue;
        }
        let keys = alts.join(" / ");
        // A shortcut the parser cannot read is left out rather than half-bound.
        let Some(seqs) = parse_keys(&keys) else { continue };
        let label = o.map(|o| o.3.to_string()).unwrap_or_else(|| c.label.to_string());
        out.push(Binding { scope: Scope::of_command(c.id), keys, label, act: Act::Cmd(c.id), seqs });
    }
    // The overlay lists commands in `OVERRIDES` order (palette, create, search first), then the rest.
    let rank = |b: &Binding| OVERRIDES.iter().position(|o| b.act == Act::Cmd(o.0)).unwrap_or(usize::MAX);
    out.sort_by_key(rank);
    for (scope, keys, label, act) in UI_BINDINGS {
        let seqs = if *act == Act::Doc { vec![] } else { parse_keys(keys).unwrap_or_default() };
        out.push(Binding { scope: *scope, keys: (*keys).to_string(), label: (*label).to_string(), act: *act, seqs });
    }
    out
}

/// The current view's own scope (besides [`Scope::Global`]). A page being edited has none, so
/// its letters stay out of the editor's way.
pub(crate) fn view_scope(app: &TrackApp) -> Option<Scope> {
    match &app.view {
        View::Issues => Some(Scope::List),
        View::Board(_) | View::SprintBoard(..) => Some(Scope::Board),
        View::Backlog(_) => Some(Scope::Backlog),
        View::Issue(_) => Some(Scope::Issue),
        View::Page(id) if !app.page_edit.as_ref().is_some_and(|e| e.id == *id) => Some(Scope::Page),
        View::Page(_) | View::Home | View::PageHistory(_) => None,
        View::Projects | View::Spaces => None, // UI-8
        View::Settings(_) => None,             // UI-7
        View::PageDiff(..) => None,            // UI-28
        View::Inbox => None,                   // M4.5
        View::Reports(_) => None,              // UI-24
    }
}

/// The shortcut for `act` in the current view (its scope first, then global), as display text.
pub(crate) fn shortcut_for(app: &TrackApp, act: Act) -> Option<String> {
    let scope = view_scope(app);
    let bound = |s: Option<Scope>| table().iter().find(|b| b.act == act && Some(b.scope) == s && !b.seqs.is_empty());
    bound(scope).or_else(|| bound(Some(Scope::Global))).map(|b| crate::key(&b.keys))
}

/// What may fire this frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gate {
    /// A text field has keyboard focus.
    pub typing: bool,
    /// Any dialog (create, new page, labels, palette, help, guard) is open.
    pub modal: bool,
    /// The only dialog open is the `?` overlay (so `?` can close it).
    pub help_only: bool,
    /// "Discard your changes?" is open.
    pub guarding: bool,
}

impl Gate {
    fn allows(self, act: Act, first: Chord) -> bool {
        if self.guarding && !matches!(act, Act::Cmd("file.save" | "file.open") | Act::SaveAs) {
            return false;
        }
        if first.plain() {
            return !self.typing && (!self.modal || (self.help_only && act == Act::Help));
        }
        // Undo and redo belong to the text field while typing (G2).
        !(self.typing && matches!(act, Act::Cmd("edit.undo" | "edit.redo")))
    }
}

/// Consumes this frame's bound key presses and returns what they ask for, in order.
pub(crate) fn dispatch(app: &mut TrackApp, i: &mut InputState, gate: Gate) -> Vec<Act> {
    let scope = view_scope(app);
    let active: Vec<&Binding> = table().iter().filter(|b| b.scope == Scope::Global || Some(b.scope) == scope).collect();
    let mut fired = Vec::new();
    let now = i.time;

    // The second key of a pending `G then X`.
    if let Some((first, at)) = app.chord.take()
        && now - at <= CHORD_TIMEOUT
    {
        for b in &active {
            for seq in &b.seqs {
                if let [a, second] = seq.as_slice()
                    && *a == first
                    && gate.allows(b.act, first)
                    && i.consume_key(second.modifiers(), second.key)
                {
                    fired.push(b.act);
                    return fired;
                }
            }
        }
        // Any other key ends the chord and is handled as usual; otherwise keep waiting.
        if !i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })) {
            app.chord = Some((first, at));
        }
    }

    // `?` also arrives as Shift+/ from backends that report the physical key.
    if active.iter().any(|b| b.act == Act::Help)
        && let Some(q) = Chord::parse("?")
        && gate.allows(Act::Help, q)
        && i.consume_key(Modifiers::SHIFT, Key::Slash)
    {
        fired.push(Act::Help);
    }

    // Single chords: most modifiers first, then the view's scope before the global one.
    let mut singles: Vec<(&Binding, Chord)> = active
        .iter()
        .flat_map(|b| {
            b.seqs.iter().filter_map(move |s| match s.as_slice() {
                [c] => Some((*b, *c)),
                _ => None,
            })
        })
        .collect();
    singles.sort_by_key(|(b, c)| (Reverse(c.weight()), b.scope == Scope::Global));
    for (b, c) in singles {
        if gate.allows(b.act, c) && i.consume_key(c.modifiers(), c.key) {
            fired.push(b.act);
        }
    }

    // The first key of a sequence starts waiting for the second.
    for b in &active {
        for seq in &b.seqs {
            if let [first, _] = seq.as_slice()
                && gate.allows(b.act, *first)
                && i.consume_key(first.modifiers(), first.key)
            {
                app.chord = Some((*first, now));
            }
        }
    }
    fired
}

/// Carries out one fired binding.
pub(crate) fn run(app: &mut TrackApp, act: Act) {
    match act {
        Act::Cmd(id) => command(app, id),
        Act::Help => app.help = !app.help,
        Act::Back => app.push(Action::Back),
        Act::SaveAs => app.push(Action::File(FileRequest::SaveAs)),
        Act::ListNext => list_step(app, 1),
        Act::ListPrev => list_step(app, -1),
        Act::ListOpen => list_step(app, 0),
        Act::BoardLeft | Act::BoardRight | Act::BoardUp | Act::BoardDown | Act::BoardOpen | Act::BoardNext | Act::BoardPrev => {
            if let View::Board(p) | View::SprintBoard(p, _) = app.view.clone() {
                board_step(app, &p, act);
            }
        }
        Act::GoHome => app.push(Action::Nav(View::Home)),
        Act::GoIssues => app.push(Action::Nav(View::Issues)),
        Act::GoBoard => {
            let current = match &app.view {
                View::Board(p) => Some(p.clone()),
                View::Issue(k) => Some(k.project.clone()),
                _ => None,
            };
            let p: Option<ProjectKey> = current.or_else(|| app.session.ws.projects.keys().next().cloned());
            match p {
                Some(p) => app.push(Action::Nav(View::Board(p))),
                None => app.push(Action::Toast("There are no projects yet".into())),
            }
        }
        Act::GoSpace => {
            let ws = &app.session.ws;
            let current = match &app.view {
                View::Page(id) | View::PageHistory(id) => ws.pages.get(id).and_then(|p| ws.spaces.get(&p.space)),
                _ => None,
            };
            match current.or_else(|| ws.spaces.values().next()).map(|s| s.homepage) {
                Some(home) => app.push(Action::Nav(View::Page(home))),
                None => app.push(Action::Toast("There are no spaces yet".into())),
            }
        }
        Act::Doc => {}
        Act::Forward => app.push(Action::Forward), // UI-8
        // UI-7
        Act::Settings => app.push(Action::Nav(View::Settings(crate::settings::Tab::Appearance))),
        Act::Peek => crate::peek::toggle(app),              // fixes
        Act::GoInbox => app.push(Action::Nav(View::Inbox)), // M4.5
    }
}

/// A registry command fired by its shortcut: opens the matching UI where there is one, else runs
/// it with the view's context.
fn command(app: &mut TrackApp, id: &'static str) {
    let Some(spec) = trackcraft_engine::command(id) else { return };
    // M4.7: on a server workspace the shortcuts say why nothing happens.
    if matches!(id, "edit.undo" | "edit.redo") && app.is_remote() {
        app.push(Action::Toast(crate::REMOTE_UNDO.into()));
        return;
    }
    if let Some(why) = (spec.enabled)(&app.session) {
        app.push(Action::Toast(format!("{}: {why}", spec.label)));
        return;
    }
    match (id, app.view.clone()) {
        ("app.commands", _) => app.push(Action::TogglePalette),
        ("search.jql", _) => app.focus_search = true,
        ("issue.create", _) => app.push(Action::OpenCreate(None)),
        ("file.save", _) => app.push(Action::File(FileRequest::Save)),
        ("file.open", _) => app.push(Action::File(FileRequest::Open)),
        ("edit.undo" | "edit.redo", _) => app.push(Action::Run { id, params: json!({}), then: Then::Nothing }),
        ("issue.edit", View::Issue(k)) => edit_description(app, &k),
        ("issue.assign", View::Issue(_)) => app.issue_ui.open_assignee = true,
        ("issue.assignToMe", View::Issue(k)) => {
            app.push(Action::Run { id, params: json!({"key": k.to_string()}), then: Then::Toast(format!("Assigned {k} to you")) })
        }
        ("issue.comment", View::Issue(_)) => app.issue_ui.focus_comment = true,
        ("issue.transition", View::Issue(_)) => app.issue_ui.open_status = true,
        ("page.create", View::Page(pid)) => {
            if let Some(p) = app.session.ws.pages.get(&pid) {
                app.push(Action::OpenNewPage { space: p.space.to_string(), parent: Some(pid) });
            }
        }
        ("page.edit", View::Page(pid)) => {
            if app.page_edit.is_none() {
                app.push(Action::EditPage(pid));
            }
        }
        ("page.labels", View::Page(pid)) => {
            if let Some(p) = app.session.ws.pages.get(&pid) {
                app.page_labels = Some((pid, p.labels.join(" ")));
            }
        }
        _ => app.push(Action::Run { id, params: crate::palette::context_params(app), then: Then::Toast(format!("Done: {}", spec.label)) }),
    }
}

fn edit_description(app: &mut TrackApp, key: &IssueKey) {
    let Some(issue) = app.session.ws.issues.get(key) else { return };
    if app.issue_ui.editing_description.is_none() {
        app.issue_ui.editing_description = Some(issue.description.to_markdown());
    }
    app.issue_ui.focus_description = true;
}

/// Moves the list selection by `delta` rows (from nothing: the first or last row); 0 opens it.
fn list_step(app: &mut TrackApp, delta: i32) {
    let Ok(keys) = &app.jql_result else { return };
    let pos = app.list_sel.as_ref().and_then(|s| keys.iter().position(|k| k == s));
    if delta == 0 {
        if let Some(k) = pos.and_then(|p| keys.get(p)).cloned() {
            app.push(Action::Nav(View::Issue(k)));
        }
        return;
    }
    let last = keys.len().saturating_sub(1);
    let next = match pos {
        Some(p) if delta > 0 => p.saturating_add(1).min(last),
        Some(p) => p.saturating_sub(1),
        None if delta > 0 => 0,
        None => last,
    };
    if let Some(k) = keys.get(next).cloned() {
        app.list_sel = Some(k);
        app.scroll_sel = true;
    }
}

fn board_step(app: &mut TrackApp, project: &ProjectKey, act: Act) {
    let cols = issues::board_columns(app, project);
    let pos = app.board_sel.as_ref().and_then(|s| cols.iter().enumerate().find_map(|(c, col)| col.2.iter().position(|k| k == s).map(|r| (c, r))));
    let at = |c: usize, r: usize| -> Option<IssueKey> { cols.get(c).and_then(|col| col.2.get(r.min(col.2.len().saturating_sub(1)))).cloned() };
    if matches!(act, Act::BoardNext | Act::BoardPrev) {
        let Some((c, r)) = pos else { return };
        let target = if act == Act::BoardNext { c.checked_add(1) } else { c.checked_sub(1) };
        if let Some((status, _, _)) = target.and_then(|t| cols.get(t))
            && let Some(k) = cols.get(c).and_then(|col| col.2.get(r))
        {
            app.push(Action::Run {
                id: "issue.transition",
                params: json!({"key": k.to_string(), "to": status}),
                then: Then::Toast(format!("{k} moved to {status}")),
            });
            app.scroll_sel = true;
        }
        return;
    }
    if act == Act::BoardOpen {
        if let Some(k) = pos.and_then(|(c, r)| at(c, r)) {
            app.push(Action::Nav(View::Issue(k)));
        }
        return;
    }
    let (left, right) = (act == Act::BoardLeft, act == Act::BoardRight);
    let moved = match pos {
        None => cols.iter().position(|col| !col.2.is_empty()).and_then(|c| at(c, 0)),
        Some((c, r)) if left || right => {
            // The nearest column in that direction that has cards.
            let mut found = None;
            let mut n = c;
            while found.is_none() {
                n = match if left { n.checked_sub(1) } else { n.checked_add(1) } {
                    Some(n) if n < cols.len() => n,
                    _ => break,
                };
                if cols.get(n).is_some_and(|col| !col.2.is_empty()) {
                    found = at(n, r);
                }
            }
            found
        }
        Some((c, r)) if act == Act::BoardDown => at(c, r.saturating_add(1)),
        Some((c, r)) => at(c, r.saturating_sub(1)),
    };
    if let Some(k) = moved {
        app.board_sel = Some(k);
        app.scroll_sel = true;
    }
}

/// Escape outside dialogs on an issue: drop every unsaved edit.
pub(crate) fn cancel_issue_edits(app: &mut TrackApp) {
    let ui = &mut app.issue_ui;
    ui.editing_summary = None;
    ui.editing_description = None;
    ui.labels = None;
    ui.confirm_delete = false;
}

/// The page labels prompt (`L` on a page).
pub(crate) fn labels_dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some((id, mut text)) = app.page_labels.take() else { return };
    let mut close = false;
    let mut save = false;
    let modal = egui::Modal::new(egui::Id::unique("page-labels")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading("Edit labels");
        let l = ui.label(RichText::new("Labels, separated by spaces").color(t.muted)).id;
        let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text("howto design").desired_width(f32::INFINITY)).labelled_by(l);
        if ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            save = true;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Save").color(t.on_accent).strong()).fill(t.accent)).clicked() {
                save = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if save {
        let labels: Vec<&str> = text.split_whitespace().take(100).collect();
        app.push(Action::Run { id: "page.labels", params: json!({"id": id.0, "labels": labels}), then: Then::Toast("Labels saved".into()) });
        return;
    }
    if !close && !modal.should_close() {
        app.page_labels = Some((id, text));
    }
}

/// The `?` overlay, built from [`table`].
pub(crate) fn help(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    if !app.help {
        return;
    }
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("shortcut-help")).show(ctx, |ui| {
        ui.set_width(720.0);
        ui.horizontal(|ui| {
            ui.heading("Keyboard shortcuts");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Close (Escape)").clicked() {
                    close = true;
                }
            });
        });
        ui.label(
            RichText::new(format!(
                "Single keys work when no text field has focus. Two-key chords: press G, then the next key within a second. {} works everywhere.",
                crate::key("Mod+K")
            ))
            .color(t.muted)
            .size(12.0),
        );
        ui.add_space(8.0);
        let (left, right): (Vec<Scope>, Vec<Scope>) = Scope::ALL.into_iter().partition(|a| matches!(a, Scope::Global | Scope::List));
        ui.columns(2, |cols| {
            for (n, (col, scopes)) in cols.iter_mut().zip([left, right]).enumerate() {
                // One grid per column, so descriptions line up across scopes.
                egui::Grid::new(("help-col", n)).num_columns(2).spacing([14.0, 5.0]).show(col, |ui| {
                    for (a, scope) in scopes.into_iter().enumerate() {
                        if a > 0 {
                            // A blank row between scopes (grids don't take `add_space`).
                            ui.label("");
                            ui.end_row();
                        }
                        ui.label(RichText::new(scope.title().to_uppercase()).size(11.0).strong().color(t.muted));
                        ui.end_row();
                        for b in table().iter().filter(|b| b.scope == scope) {
                            key_chip(ui, t, &crate::key(&b.keys));
                            ui.label(&b.label);
                            ui.end_row();
                        }
                    }
                });
            }
        });
    });
    if close || modal.should_close() {
        app.help = false;
    }
}

pub(crate) fn key_chip(ui: &mut egui::Ui, t: &Tokens, keys: &str) {
    egui::Frame::new()
        .fill(t.surface_alt)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(keys).monospace().size(12.0).color(t.text)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_parse() {
        let c = Chord::parse("Mod+Shift+Z").unwrap();
        assert!(c.command && c.shift && !c.alt && c.key == Key::Z);
        assert_eq!(Chord::parse("C").unwrap().key, Key::C);
        assert!(Chord::parse("C").unwrap().plain());
        assert_eq!(Chord::parse("F7").unwrap().key, Key::F7);
        assert_eq!(Chord::parse("/").unwrap().key, Key::Slash);
        assert_eq!(Chord::parse("?").unwrap().key, Key::Questionmark);
        let b = Chord::parse("Alt+Left").unwrap();
        assert!(b.alt && !b.plain() && b.key == Key::ArrowLeft);
        let p = Chord::parse("Mod++").unwrap();
        assert!(p.command && p.key == Key::Plus);
        assert_eq!(Chord::parse("ctrl + k").unwrap().key, Key::K);
        for bad in ["", "Mod+", "Hyper+K", "NotAKey", "Mod+Shift+", &"A".repeat(50)] {
            assert!(Chord::parse(bad).is_none(), "{bad:?} parsed");
        }
    }

    #[test]
    fn key_lists_parse() {
        let s = parse_keys("J / Down").unwrap();
        assert_eq!(s.len(), 2);
        let g = parse_keys("G then H").unwrap();
        assert_eq!(g.first().map(Vec::len), Some(2));
        assert!(parse_keys("Click a header").is_none());
    }

    #[test]
    fn scopes_come_from_ids() {
        assert_eq!(Scope::of_command("issue.assign"), Scope::Issue);
        assert_eq!(Scope::of_command("issue.create"), Scope::Global);
        assert_eq!(Scope::of_command("page.create"), Scope::Page);
        assert_eq!(Scope::of_command("search.jql"), Scope::Global);
        assert_eq!(Scope::of_command("board.view"), Scope::Board);
        assert_eq!(Scope::of_command("edit.redo"), Scope::Global);
    }
}
