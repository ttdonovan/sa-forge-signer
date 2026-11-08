//! The window: status bar, keys with their limits, the audit log, and the selected entry.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::link::Link;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableEvent, TableState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, Theme, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Pixels, Render,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window, div, px,
};

use crate::Health;
use crate::config::{self, Config};
use crate::format::{age, fraction, outcome_label, shorten_addresses, signed_zink, when, zink};
use crate::log::{Chain, Entry, Row, SIGN_INTENT, Snapshot};
use crate::theme;

const POLL_EVERY: Duration = Duration::from_millis(500);
/// How long a new row stays highlighted.
const FRESH: Duration = Duration::from_secs(3);
/// Between reads of only the appended lines, the whole log is read and its chain checked this often
/// (if it changed), so an edit above the last line read is still seen.
const FULL_EVERY: Duration = Duration::from_secs(60);

pub struct Sources {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub config_error: Option<String>,
    pub state_dir: Option<PathBuf>,
    pub url: Option<String>,
    pub health: Option<Receiver<Health>>,
}

/// How a value reads: drives its colour.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Good,
    Warn,
    Bad,
    Muted,
}

impl Tone {
    fn of(e: &Entry, unresolved: bool) -> Self {
        if unresolved {
            return Self::Warn;
        }
        match e.outcome.as_str() {
            "confirmed" | "ok" => Self::Good,
            "failed" | "refused" | "error" => Self::Bad,
            _ => Self::Muted,
        }
    }

    fn color(self, cx: &App) -> Hsla {
        let t = cx.theme();
        match self {
            Self::Good => t.success,
            Self::Warn => t.warning,
            Self::Bad => t.danger,
            Self::Muted => t.muted_foreground,
        }
    }
}

/// One table row, with its text built when the log or the filters change.
struct RowView {
    entry: usize,
    ts: u64,
    key: SharedString,
    command: SharedString,
    outcome: SharedString,
    tone: Tone,
    change: SharedString,
    intent: Option<SharedString>,
    summary: SharedString,
    /// When this row's newest entry arrived, while the console was running.
    arrived: Option<Instant>,
}

/// The audit log as a `DataTable`: rows and columns are rebuilt by [`Console::rebuild`].
#[derive(Default)]
pub struct LogTable {
    rows: Vec<RowView>,
    columns: Vec<Column>,
    now: u64,
}

impl LogTable {
    fn set_columns(&mut self, show_key: bool, raw: bool) {
        let mut cols = vec![Column::new("age", "age").width(px(80.0))];
        if show_key {
            cols.push(Column::new("key", "key").width(px(120.0)));
        }
        if raw {
            cols.push(Column::new("command", "command").width(px(100.0)));
        }
        cols.push(Column::new("outcome", "outcome").width(px(100.0)));
        cols.push(Column::new("zink", "ZINK").width(px(110.0)).text_right());
        cols.push(Column::new("what", "intent / summary").width(px(900.0)));
        self.columns = cols;
    }
}

impl TableDelegate for LogTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns
            .get(col_ix)
            .cloned()
            .unwrap_or_else(|| Column::new("none", ""))
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        label()
            .size_full()
            .text_color(cx.theme().muted_foreground)
            .child(
                self.columns
                    .get(col_ix)
                    .map(|c| c.name.to_uppercase())
                    .unwrap_or_default(),
            )
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let row = div().id(("row", row_ix));
        match self.rows.get(row_ix).and_then(|r| r.arrived) {
            Some(t) if t.elapsed() < FRESH => row.bg(cx.theme().success.opacity(0.18)),
            _ => row,
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let (Some(r), Some(col)) = (self.rows.get(row_ix), self.columns.get(col_ix)) else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        match col.key.as_ref() {
            "age" => div().child(age(self.now, r.ts)).into_any_element(),
            "key" => div().child(r.key.clone()).into_any_element(),
            "command" => div().child(r.command.clone()).into_any_element(),
            "outcome" => h_flex()
                .gap_2()
                .text_color(r.tone.color(cx))
                .child(dot(r.tone.color(cx)))
                .child(r.outcome.clone())
                .into_any_element(),
            "zink" => div()
                .text_color(muted)
                .child(r.change.clone())
                .into_any_element(),
            _ => h_flex()
                .gap_2()
                .overflow_hidden()
                .whitespace_nowrap()
                .children(r.intent.clone())
                .child(div().text_color(muted).child(r.summary.clone()))
                .into_any_element(),
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        h_flex()
            .size_full()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child("No entries: the log fills as the agent checks and signs, or no entry matches the filters.")
    }
}

pub struct Console {
    config: Config,
    config_path: Option<PathBuf>,
    config_error: Option<String>,
    audit_path: Option<PathBuf>,
    url: Option<String>,
    health_rx: Option<Receiver<Health>>,
    health: Option<Health>,
    log: Snapshot,
    log_error: Option<String>,
    /// Length and mtime of the audit log when last read.
    stamp: Option<(u64, SystemTime)>,
    /// When the log was last read whole, and its stamp then.
    full: Option<(Instant, Option<(u64, SystemTime)>)>,
    /// When each entry appeared after the first read, for highlighting new rows.
    arrived: HashMap<usize, Instant>,
    key_filter: Option<String>,
    unresolved_only: bool,
    /// One row per audit entry instead of per signature.
    raw: bool,
    /// Index into `log.entries` of the selected row's entry.
    selected: Option<usize>,
    focus: FocusHandle,
    search: Entity<InputState>,
    table: Entity<TableState<LogTable>>,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}

impl Console {
    pub fn new(s: Sources, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter by intent, summary, key or signature  ( / )")
        });
        let table = cx.new(|cx| {
            TableState::new(LogTable::default(), window, cx)
                .row_header(false)
                .col_movable(false)
                .sortable(false)
                .col_selectable(false)
                .loop_selection(false)
        });
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rebuild(cx);
                }
            }),
            cx.subscribe_in(&table, window, |this, table, event, _, cx| {
                if let TableEvent::SelectRow(ix) = event {
                    this.selected = table.read(cx).delegate().rows.get(*ix).map(|r| r.entry);
                    cx.notify();
                }
            }),
        ];
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_EVERY).await;
                if this.update(cx, Self::tick).is_err() {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        window.focus(&table.focus_handle(cx), cx);
        let mut this = Self {
            config: s.config,
            config_path: s.config_path,
            config_error: s.config_error,
            audit_path: s.state_dir.map(|d| d.join("audit.jsonl")),
            url: s.url,
            health_rx: s.health,
            health: None,
            log: Snapshot::empty(),
            log_error: None,
            stamp: None,
            full: None,
            arrived: HashMap::new(),
            key_filter: None,
            unresolved_only: false,
            raw: false,
            selected: None,
            focus,
            search,
            table,
            _subscriptions: subscriptions,
            _poll: poll,
        };
        this.read_log();
        this.rebuild(cx);
        this
    }

    /// Takes the latest health result and re-reads the log if it changed; redraws for the ages
    /// and the fading highlight.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if let Some(rx) = &self.health_rx {
            while let Ok(h) = rx.try_recv() {
                self.health = Some(h);
            }
        }
        if self.read_log() || self.arrived.values().any(|t| t.elapsed() < FRESH) {
            self.arrived.retain(|_, t| t.elapsed() < FRESH);
            self.rebuild(cx);
        } else {
            let now = unix_now();
            self.table.update(cx, |t, cx| {
                t.delegate_mut().now = now;
                cx.notify();
            });
        }
        cx.notify();
    }

    /// Re-reads the audit log when its length or mtime changes. Read-only: never locks it.
    /// Returns whether it changed.
    fn read_log(&mut self) -> bool {
        let Some(path) = &self.audit_path else {
            self.log_error = Some("no state directory: pass --state-dir or --config".to_owned());
            return false;
        };
        let stamp = std::fs::metadata(path)
            .ok()
            .and_then(|m| Some((m.len(), m.modified().ok()?)));
        let whole = self
            .full
            .is_none_or(|(at, was)| at.elapsed() >= FULL_EVERY && was != stamp);
        if !whole && stamp.is_some() && stamp == self.stamp {
            return false;
        }
        let first = self.stamp.is_none();
        let old = self.log.entries.len();
        let read = if whole {
            Snapshot::read(path).map(|s| self.log = s)
        } else {
            self.log.refresh(path)
        };
        match read {
            Ok(()) => {
                let new = self.log.entries.len();
                if !first && new > old {
                    let now = Instant::now();
                    for i in old..new {
                        self.arrived.insert(i, now);
                    }
                } else if new < old {
                    self.arrived.clear();
                }
                if whole {
                    self.full = Some((Instant::now(), stamp));
                }
                self.log_error = None;
                self.stamp = stamp;
                true
            }
            Err(e) => {
                // A failed read may leave a partial snapshot: read it whole next time.
                self.full = None;
                self.log_error = Some(format!("reading {}: {e}", path.display()));
                false
            }
        }
    }

    fn keys(&self) -> Vec<config::Key> {
        let mut keys = self.config.keys.clone();
        for name in self.log.keys() {
            if !keys.iter().any(|k| k.name == name) {
                keys.push(config::Key {
                    name,
                    pubkey: None,
                    class: "not in config".to_owned(),
                    approval: "?".to_owned(),
                    rate_limit_per_minute: config::DEFAULT_RATE_PER_MINUTE,
                    daily_lamport_cap: config::DEFAULT_DAILY_LAMPORT_CAP,
                });
            }
        }
        keys
    }

    /// Rows newest first, one per signature unless `raw`, after the key, unresolved and text filters.
    fn visible_rows(&self, needle: &str) -> Vec<Row> {
        let rows: Vec<Row> = if self.raw {
            (0..self.log.entries.len())
                .map(|entry| Row {
                    entry,
                    intent: None,
                })
                .collect()
        } else {
            self.log.rows()
        };
        rows.into_iter()
            .rev()
            .filter(|r| {
                self.log.entries.get(r.entry).is_some_and(|e| {
                    self.key_filter.as_ref().is_none_or(|f| &e.key == f)
                        && (!self.unresolved_only || self.log.unresolved(e))
                        && matches(e, needle)
                })
            })
            .collect()
    }

    /// Rebuilds the table's rows and columns, keeping the selected entry selected.
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "Entity::update needs &mut; the lint does not see through it"
    )]
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let needle = self.search.read(cx).value().to_lowercase();
        let rows: Vec<RowView> = self
            .visible_rows(&needle)
            .into_iter()
            .filter_map(|r| {
                let e = self.log.entries.get(r.entry)?;
                let unresolved = self.log.unresolved(e);
                let arrived = [Some(r.entry), r.intent]
                    .into_iter()
                    .flatten()
                    .filter_map(|i| self.arrived.get(&i).copied())
                    .max();
                Some(RowView {
                    entry: r.entry,
                    ts: e.ts,
                    key: e.key.clone().into(),
                    command: e.command.clone().into(),
                    outcome: outcome_label(e, unresolved).into(),
                    tone: Tone::of(e, unresolved),
                    change: e
                        .balance_change
                        .map_or_else(|| "-".to_owned(), signed_zink)
                        .trim_end_matches(" ZINK")
                        .to_owned()
                        .into(),
                    intent: e.intent.clone().map(Into::into),
                    summary: shorten_addresses(&e.summary).into(),
                    arrived,
                })
            })
            .collect();
        let selected_ix = self
            .selected
            .and_then(|s| rows.iter().position(|r| r.entry == s));
        let show_key = self.key_filter.is_none() && self.keys().len() > 1;
        let raw = self.raw;
        let now = unix_now();
        self.table.update(cx, |t, cx| {
            let previous = t.selected_row();
            let d = t.delegate_mut();
            d.rows = rows;
            d.now = now;
            d.set_columns(show_key, raw);
            t.refresh(cx);
            if let Some(ix) = selected_ix {
                if previous != Some(ix) {
                    t.set_selected_row(ix, cx);
                }
            } else if previous.is_some() {
                t.clear_selection(cx);
            }
            cx.notify();
        });
    }

    fn close_detail(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.table.update(cx, TableState::clear_selection);
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let typing = self.search.focus_handle(cx).is_focused(window);
        match ev.keystroke.key.as_str() {
            "escape" if typing => {
                window.focus(&self.table.focus_handle(cx), cx);
                cx.stop_propagation();
            }
            "escape" if self.selected.is_some() => {
                self.close_detail(cx);
                cx.stop_propagation();
            }
            "/" if !typing => {
                self.search.update(cx, |s, cx| s.focus(window, cx));
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn status_bar(&self, cx: &App) -> impl IntoElement {
        let t = cx.theme();
        let (serve, serve_color) = match (&self.url, &self.health) {
            (None, _) => ("not configured".to_owned(), t.muted_foreground),
            (Some(u), None) => (format!("checking {u}"), t.muted_foreground),
            (Some(u), Some(Health::Up)) => (format!("up {u}"), t.success),
            (Some(u), Some(Health::Down(e))) => (format!("down {u} ({e})"), t.danger),
        };
        let (chain, chain_color) = match &self.log.chain {
            Chain::Ok => (format!("ok, {} entries", self.log.entries.len()), t.success),
            Chain::Broken(line, why) => (format!("broken at line {line}: {why}"), t.danger),
        };
        let unresolved = self
            .log
            .entries
            .iter()
            .filter(|e| self.log.unresolved(e))
            .count();
        let paths = format!(
            "config: {}    audit: {}",
            self.config_path
                .as_ref()
                .map_or_else(|| "none".to_owned(), |p| p.display().to_string()),
            self.audit_path
                .as_ref()
                .map_or_else(|| "none".to_owned(), |p| p.display().to_string()),
        );
        v_flex()
            .px_3()
            .py_2()
            .gap_1()
            .bg(theme::c(theme::PANEL))
            .border_b_1()
            .border_color(t.border)
            .child(
                h_flex()
                    .gap_6()
                    .text_sm()
                    .child(chip("serve", serve, serve_color, t))
                    .child(chip("audit chain", chain, chain_color, t))
                    .when(unresolved > 0, |d| {
                        d.child(chip(
                            "unresolved",
                            format!("{unresolved}: reconcile before retrying"),
                            t.warning,
                            t,
                        ))
                    })
                    .children(self.log.entries.last().map(|e| {
                        chip(
                            "last activity",
                            age(unix_now(), e.ts),
                            t.muted_foreground,
                            t,
                        )
                    }))
                    .children(
                        [&self.config_error, &self.log_error]
                            .into_iter()
                            .flatten()
                            .map(|err| div().text_color(t.danger).child(err.clone())),
                    ),
            )
            .child(div().text_xs().text_color(t.muted_foreground).child(paths))
    }

    fn keys_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        let now = unix_now();
        let t = cx.theme().clone();
        let explorer = self.config.explorer_url.clone();
        let mut panel = v_flex()
            .size_full()
            .p_3()
            .gap_3()
            .bg(theme::c(theme::PANEL))
            .child(
                h_flex().justify_between().child(heading("Keys")).child(
                    Button::new("all-keys")
                        .small()
                        .ghost()
                        .label("All keys")
                        .disabled(self.key_filter.is_none())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.key_filter = None;
                            this.rebuild(cx);
                        })),
                ),
            );
        for (ix, k) in self.keys().into_iter().enumerate() {
            panel = panel.child(self.key_card(ix, &k, now, &t, explorer.as_deref(), cx));
        }
        panel
    }

    /// A key's class, pubkey, last activity and limits; click to filter the log by it.
    fn key_card(
        &self,
        ix: usize,
        k: &config::Key,
        now: u64,
        t: &Theme,
        explorer: Option<&str>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let u = self.log.usage(&k.name, k.daily_lamport_cap, now);
        let selected = self.key_filter.as_deref() == Some(k.name.as_str());
        let last = self
            .log
            .entries
            .iter()
            .rev()
            .find(|e| e.key == k.name)
            .map(|e| e.ts);
        let name = k.name.clone();
        let recent = u64::try_from(u.recent).unwrap_or(u64::MAX);
        v_flex()
            .id(("key", ix))
            .p_3()
            .gap_1p5()
            .bg(theme::c(theme::RAISED))
            .rounded(t.radius_lg)
            .border_1()
            .border_l_2()
            .border_color(if selected { t.info } else { t.border })
            .hover(|s| s.bg(theme::c(theme::BORDER)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.key_filter = if this.key_filter.as_deref() == Some(name.as_str()) {
                    None
                } else {
                    Some(name.clone())
                };
                this.rebuild(cx);
            }))
            .child(
                div()
                    .font_family(theme::LABEL_FONT)
                    .font_semibold()
                    .child(k.name.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .child(format!("{} key, approval {}", k.class, k.approval)),
            )
            .children(k.pubkey.clone().map(|pk| {
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .font_family(t.mono_font_family.clone())
                            .child(shorten_addresses(&pk)),
                    )
                    .child(Clipboard::new(("copy-key", ix)).value(pk.clone()))
                    .children(explorer.as_ref().map(|base| {
                        Link::new(("key-link", ix))
                            .href(format!("{base}/address/{pk}"))
                            .child("explorer ↗")
                    }))
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .child(last.map_or_else(
                        || "no activity yet".to_owned(),
                        |ts| format!("last activity {}", age(now, ts)),
                    )),
            )
            .child(limit_bar(
                ("rate", ix),
                format!("rate {} / {} per minute", u.recent, k.rate_limit_per_minute),
                fraction(recent, u64::from(k.rate_limit_per_minute)),
                t,
            ))
            .child(limit_bar(
                ("cap", ix),
                format!(
                    "{} / {} ZINK in 24 h",
                    zink(u.spent),
                    zink(k.daily_lamport_cap)
                ),
                fraction(u.spent, k.daily_lamport_cap),
                t,
            ))
            .when(u.unresolved > 0, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(t.warning)
                        .child(format!("{} unresolved", u.unresolved)),
                )
            })
    }

    fn log_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .p_2()
                    .gap_3()
                    .child(heading("Audit log"))
                    .child(
                        Checkbox::new("unresolved-only")
                            .label("unresolved only")
                            .checked(self.unresolved_only)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.unresolved_only = *checked;
                                this.rebuild(cx);
                            })),
                    )
                    .child(
                        Checkbox::new("raw")
                            .label("raw entries")
                            .checked(self.raw)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.raw = *checked;
                                this.rebuild(cx);
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.search).small().cleanable(true)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.table).stripe(true).bordered(false)),
            )
    }

    /// The selected row: what was signed, what happened, and the entries that record it.
    fn detail(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let i = self.selected?;
        let e = self.log.entries.get(i)?;
        let intent = if self.raw {
            None
        } else {
            self.log
                .rows()
                .into_iter()
                .find(|r| r.entry == i)
                .and_then(|r| r.intent)
                .and_then(|j| self.log.entries.get(j))
        };
        let unresolved = self.log.unresolved(e);
        let now = unix_now();
        let t = cx.theme().clone();
        let [what, result, integrity] = self.detail_fields(e, intent, now, &t);
        Some(
            v_flex()
                .id("detail")
                .size_full()
                .bg(theme::c(theme::PANEL))
                .p_3()
                .gap_3()
                .overflow_y_scrollbar()
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_xl()
                                .font_family(theme::LABEL_FONT)
                                .font_semibold()
                                .text_color(Tone::of(e, unresolved).color(cx))
                                .child(outcome_label(e, unresolved)),
                        )
                        .child(
                            Button::new("close")
                                .small()
                                .ghost()
                                .label("close  esc")
                                .on_click(cx.listener(|this, _, _, cx| this.close_detail(cx))),
                        ),
                )
                .when(unresolved, |d| {
                    d.child(
                        div()
                            .text_color(t.warning)
                            .child("Sent with no known outcome: reconcile before retrying."),
                    )
                })
                .when(e.command != "sign" && e.command != SIGN_INTENT, |d| {
                    d.child(
                        div()
                            .text_color(t.muted_foreground)
                            .child(format!("command: {}", e.command)),
                    )
                })
                .child(section("What", what, &t))
                .child(section("Result", result, &t))
                .child(section("Integrity", integrity, &t))
                .into_any_element(),
        )
    }

    /// The detail panel's sections: what was signed, what happened, and the chain hashes.
    fn detail_fields(
        &self,
        e: &Entry,
        intent: Option<&Entry>,
        now: u64,
        t: &Theme,
    ) -> [Vec<AnyElement>; 3] {
        let explorer = self.config.explorer_url.clone();
        let link = |path: &str, value: &str| {
            explorer
                .as_ref()
                .map(|base| format!("{base}/{path}/{value}"))
        };
        let mut what = vec![text_field("intent", e.intent.as_deref().unwrap_or("-"), t)];
        what.push(text_field(
            "summary (from the builder, not verified)",
            &e.summary,
            t,
        ));
        for (n, p) in e.programs.iter().enumerate() {
            what.push(id_field(
                ("program", n),
                "program",
                p,
                link("address", p),
                t,
            ));
        }
        let mut result = vec![text_field("key", &e.key, t)];
        if let Some(i) = intent {
            result.push(text_field("sent", &when(now, i.ts), t));
        }
        result.push(text_field(
            if intent.is_some() { "settled" } else { "time" },
            &when(now, e.ts),
            t,
        ));
        if let Some(d) = &e.detail {
            result.push(text_field("detail", d, t));
        }
        if let Some(sig) = &e.signature {
            result.push(id_field(("sig", 0), "signature", sig, link("tx", sig), t));
        }
        if let Some(slot) = e.slot {
            let slot = slot.to_string();
            result.push(id_field(
                ("slot", 0),
                "slot",
                &slot,
                link("block", &slot),
                t,
            ));
        }
        if let Some(d) = e.balance_change {
            result.push(text_field("balance change", &signed_zink(d), t));
        }
        if let Some(d) = intent.and_then(|i| i.balance_change) {
            result.push(text_field(
                "reserved against the daily cap",
                &signed_zink(d),
                t,
            ));
        }
        let mut integrity = vec![
            id_field(("h", 0), "payload hash", &e.payload_hash, None, t),
            id_field(("h", 1), "entry hash", &e.hash, None, t),
            id_field(("h", 2), "previous hash", &e.prev, None, t),
        ];
        if let Some(i) = intent {
            integrity.push(id_field(("h", 3), "intent entry hash", &i.hash, None, t));
        }
        [what, result, integrity]
    }
}

impl Render for Console {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        let detail = self.detail(cx);
        let t = cx.theme();
        v_flex()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(self.status_bar(cx))
            .child(
                h_resizable("main")
                    .child(
                        resizable_panel()
                            .size(px(300.0))
                            .size_range(px(240.0)..px(480.0))
                            .child(
                                div()
                                    .id("keys")
                                    .size_full()
                                    .overflow_y_scrollbar()
                                    .child(self.keys_panel(cx)),
                            ),
                    )
                    .child(
                        resizable_panel()
                            .size_range(px(420.0)..Pixels::MAX)
                            .child(self.log_panel(cx)),
                    )
                    .child(
                        resizable_panel()
                            .size(px(480.0))
                            .size_range(px(380.0)..px(900.0))
                            .visible(detail.is_some())
                            .children(detail),
                    ),
            )
    }
}

/// Whether an entry's intent, summary, key, signature or outcome contains `needle` (lowercase).
fn matches(e: &Entry, needle: &str) -> bool {
    needle.is_empty()
        || [
            e.intent.as_deref(),
            Some(e.summary.as_str()),
            Some(e.key.as_str()),
            e.signature.as_deref(),
            Some(e.outcome.as_str()),
        ]
        .into_iter()
        .flatten()
        .any(|s| s.to_lowercase().contains(needle))
}

/// A label, then a thin bar coloured by how close it is to the limit.
fn limit_bar(
    id: (&'static str, usize),
    label: String,
    fraction: f32,
    t: &Theme,
) -> impl IntoElement {
    let color = if fraction >= 1.0 {
        t.danger
    } else if fraction >= 0.8 {
        t.warning
    } else {
        t.success
    };
    v_flex().gap_1().child(div().text_xs().child(label)).child(
        Progress::new(id)
            .value(fraction * 100.0)
            .color(color)
            .h(px(4.0)),
    )
}

/// A titled group of fields.
fn section(title: &str, fields: Vec<AnyElement>, t: &Theme) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(heading(title).pb_1().border_b_1().border_color(t.border))
        .children(fields)
}

/// A small uppercase teal label, the HUD's panel and section heading.
fn heading(text: &str) -> Div {
    label()
        .font_semibold()
        .text_color(theme::c(theme::TEAL))
        .child(text.to_uppercase())
}

/// A status-bar item: a coloured dot, an uppercase label and its value.
fn chip(label: &str, value: String, color: Hsla, t: &Theme) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(dot(color))
        .child(
            self::label()
                .text_color(t.muted_foreground)
                .child(label.to_uppercase()),
        )
        .child(div().text_color(color).child(value))
}

/// Small text in the label font: headings, field names, the status bar's labels.
fn label() -> Div {
    div().font_family(theme::LABEL_FONT).text_xs()
}

fn dot(color: Hsla) -> Div {
    div().size(px(7.0)).rounded_full().bg(color)
}

/// A label over a plain value: intents, summaries, times and amounts.
fn text_field(name: &str, value: &str, t: &Theme) -> AnyElement {
    v_flex()
        .child(
            label()
                .text_color(t.muted_foreground)
                .child(name.to_owned()),
        )
        .child(div().child(value.to_owned()))
        .into_any_element()
}

/// A label over a monospace identifier (address, signature, slot, hash); with `url`, a copy button
/// and a link out.
fn id_field(
    id: (&'static str, usize),
    name: &str,
    value: &str,
    url: Option<String>,
    t: &Theme,
) -> AnyElement {
    let header = h_flex().gap_2().text_xs().child(
        label()
            .text_color(t.muted_foreground)
            .child(name.to_owned()),
    );
    let header = match url {
        Some(u) => header
            .child(Clipboard::new(("copy", id.1)).value(value.to_owned()))
            .child(Link::new(id).href(u).child("explorer ↗")),
        None => header,
    };
    v_flex()
        .child(header)
        .child(
            div()
                .font_family(t.mono_font_family.clone())
                .text_sm()
                .child(value.to_owned()),
        )
        .into_any_element()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
