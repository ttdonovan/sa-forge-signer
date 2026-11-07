//! The window: status bar, keys with their limits, the audit log, and the selected entry.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};

use crate::Health;
use crate::config::{self, Config};
use crate::log::{Chain, Entry, Snapshot};

const POLL_EVERY: Duration = Duration::from_secs(1);
const LAMPORTS_PER_ZINK: u64 = 1_000_000_000;

const GOOD: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);
const WARN: Color32 = Color32::from_rgb(0xe3, 0x9b, 0x1b);
const BAD: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

pub struct Sources {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub config_error: Option<String>,
    pub state_dir: Option<PathBuf>,
    pub url: Option<String>,
    pub health: Option<Receiver<Health>>,
}

pub struct App {
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
    last_poll: Option<Instant>,
    filter: Option<String>,
    unresolved_only: bool,
    /// Index into `log.entries`.
    selected: Option<usize>,
}

impl App {
    pub fn new(s: Sources) -> Self {
        Self {
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
            last_poll: None,
            filter: None,
            unresolved_only: false,
            selected: None,
        }
    }

    /// Re-reads the audit log when its length or mtime changes. Read-only: never locks it.
    fn poll(&mut self) {
        if self.last_poll.is_some_and(|t| t.elapsed() < POLL_EVERY) {
            return;
        }
        self.last_poll = Some(Instant::now());
        let Some(path) = &self.audit_path else {
            self.log_error = Some("no state directory: pass --state-dir or --config".to_owned());
            return;
        };
        let stamp = std::fs::metadata(path)
            .ok()
            .and_then(|m| Some((m.len(), m.modified().ok()?)));
        if stamp.is_some() && stamp == self.stamp {
            return;
        }
        match Snapshot::read(path) {
            Ok(s) => {
                self.log = s;
                self.log_error = None;
                self.stamp = stamp;
            }
            Err(e) => self.log_error = Some(format!("reading {}: {e}", path.display())),
        }
    }

    fn keys(&self) -> Vec<config::Key> {
        let mut keys = self.config.keys.clone();
        for name in self.log.keys() {
            if !keys.iter().any(|k| k.name == name) {
                keys.push(config::Key {
                    name,
                    class: "not in config".to_owned(),
                    approval: "?".to_owned(),
                    rate_limit_per_minute: config::DEFAULT_RATE_PER_MINUTE,
                    daily_lamport_cap: config::DEFAULT_DAILY_LAMPORT_CAP,
                });
            }
        }
        keys
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            match (&self.url, &self.health) {
                (None, _) => ui.label("serve: not configured"),
                (Some(u), None) => ui.label(format!("serve {u}: checking")),
                (Some(u), Some(Health::Up)) => {
                    ui.label(RichText::new(format!("serve {u}: up")).color(GOOD))
                }
                (Some(u), Some(Health::Down(e))) => ui
                    .label(RichText::new(format!("serve {u}: down")).color(BAD))
                    .on_hover_text(e),
            };
            ui.separator();
            match &self.log.chain {
                Chain::Ok => ui.label(
                    RichText::new(format!(
                        "audit chain ok, {} entries",
                        self.log.entries.len()
                    ))
                    .color(GOOD),
                ),
                Chain::Broken(line, why) => ui.label(
                    RichText::new(format!("audit chain broken at line {line}: {why}")).color(BAD),
                ),
            };
            let unresolved = self
                .log
                .entries
                .iter()
                .filter(|e| self.log.unresolved(e))
                .count();
            if unresolved > 0 {
                ui.separator();
                ui.label(RichText::new(format!("{unresolved} unresolved")).color(WARN))
                    .on_hover_text("Sent with no known outcome: reconcile before retrying.");
            }
            for err in [&self.config_error, &self.log_error].into_iter().flatten() {
                ui.separator();
                ui.label(RichText::new(err).color(BAD));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!(
                "config: {}   audit: {}",
                self.config_path
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), |p| p.display().to_string()),
                self.audit_path
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), |p| p.display().to_string()),
            ));
        });
    }

    fn keys_panel(&mut self, ui: &mut egui::Ui, now: u64) {
        ui.heading("Keys");
        if ui
            .selectable_label(self.filter.is_none(), "All keys")
            .clicked()
        {
            self.filter = None;
        }
        ui.separator();
        for k in self.keys() {
            let u = self.log.usage(&k.name, k.daily_lamport_cap, now);
            let selected = self.filter.as_deref() == Some(k.name.as_str());
            if ui
                .selectable_label(selected, RichText::new(&k.name).strong())
                .clicked()
            {
                self.filter = if selected { None } else { Some(k.name.clone()) };
            }
            ui.label(format!("{} key, approval {}", k.class, k.approval));
            let rate = format!("rate {}/{} per minute", u.recent, k.rate_limit_per_minute);
            ui.label(rate);
            ui.add(
                egui::ProgressBar::new(fraction(u.spent, k.daily_lamport_cap)).text(format!(
                    "{} / {} ZINK in 24 h",
                    zink(u.spent),
                    zink(k.daily_lamport_cap)
                )),
            );
            if u.unresolved > 0 {
                ui.label(RichText::new(format!("{} unresolved", u.unresolved)).color(WARN));
            }
            ui.separator();
        }
    }

    fn entries_table(&mut self, ui: &mut egui::Ui, now: u64) {
        ui.horizontal(|ui| {
            ui.heading("Audit log");
            ui.checkbox(&mut self.unresolved_only, "unresolved only");
            if let Some(f) = &self.filter {
                ui.label(format!("key: {f}"));
            }
        });
        let rows: Vec<usize> = self
            .log
            .entries
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, e)| self.filter.as_ref().is_none_or(|f| &e.key == f))
            .filter(|(_, e)| !self.unresolved_only || self.log.unresolved(e))
            .map(|(i, _)| i)
            .collect();
        let mut clicked = None;
        TableBuilder::new(ui)
            .striped(true)
            .sense(egui::Sense::click())
            .column(Column::initial(70.0))
            .column(Column::initial(120.0))
            .column(Column::initial(90.0))
            .column(Column::initial(90.0))
            .column(Column::remainder().clip(true))
            .header(20.0, |mut h| {
                for title in ["age", "key", "command", "outcome", "summary / intent"] {
                    h.col(|ui| {
                        ui.strong(title);
                    });
                }
            })
            .body(|body| {
                body.rows(18.0, rows.len(), |mut row| {
                    let Some(&i) = rows.get(row.index()) else {
                        return;
                    };
                    let Some(e) = self.log.entries.get(i) else {
                        return;
                    };
                    row.set_selected(self.selected == Some(i));
                    let unresolved = self.log.unresolved(e);
                    row.col(|ui| {
                        ui.label(age(now, e.ts));
                    });
                    row.col(|ui| {
                        ui.label(&e.key);
                    });
                    row.col(|ui| {
                        ui.label(&e.command);
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(&e.outcome).color(outcome_color(e, unresolved)));
                    });
                    row.col(|ui| {
                        ui.label(e.intent.as_ref().map_or_else(
                            || e.summary.clone(),
                            |intent| format!("{}  ({intent})", e.summary),
                        ));
                    });
                    if row.response().clicked() {
                        clicked = Some(i);
                    }
                });
            });
        if let Some(i) = clicked {
            self.selected = if self.selected == Some(i) {
                None
            } else {
                Some(i)
            };
        }
    }

    fn detail(&self, ui: &mut egui::Ui, e: &Entry) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("detail")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    let opt = |v: Option<String>| v.unwrap_or_else(|| "-".to_owned());
                    let fields = [
                        ("ts", e.ts.to_string()),
                        ("key", e.key.clone()),
                        ("command", e.command.clone()),
                        ("outcome", e.outcome.clone()),
                        ("unresolved", self.log.unresolved(e).to_string()),
                        ("detail", opt(e.detail.clone())),
                        ("signature", opt(e.signature.clone())),
                        ("slot", opt(e.slot.map(|s| s.to_string()))),
                        (
                            "balance change",
                            opt(e.balance_change.map(|d| format!("{d} lamports"))),
                        ),
                        ("summary (builder, unverified)", e.summary.clone()),
                        ("intent", opt(e.intent.clone())),
                        ("programs", e.programs.join("\n")),
                        ("payload hash", e.payload_hash.clone()),
                        ("hash", e.hash.clone()),
                    ];
                    for (name, value) in fields {
                        ui.strong(name);
                        ui.add(egui::Label::new(RichText::new(value).monospace()).wrap());
                        ui.end_row();
                    }
                });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        if let Some(rx) = &self.health_rx {
            while let Ok(h) = rx.try_recv() {
                self.health = Some(h);
            }
        }
        // Keeps the log poll and the ages moving without input.
        ui.ctx().request_repaint_after(POLL_EVERY);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());

        egui::Panel::top("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("keys")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.keys_panel(ui, now));
            });
        if let Some(e) = self.selected.and_then(|i| self.log.entries.get(i)).cloned() {
            egui::Panel::bottom("detail")
                .resizable(true)
                .default_size(240.0)
                .show(ui, |ui| self.detail(ui, &e));
        }
        egui::CentralPanel::default().show(ui, |ui| self.entries_table(ui, now));
    }
}

fn outcome_color(e: &Entry, unresolved: bool) -> Color32 {
    if unresolved {
        return WARN;
    }
    match e.outcome.as_str() {
        "confirmed" | "ok" => GOOD,
        "failed" | "refused" | "error" => BAD,
        _ => Color32::GRAY,
    }
}

fn age(now: u64, ts: u64) -> String {
    let s = now.saturating_sub(ts);
    let (n, unit) = match s {
        0..60 => (s, "s"),
        60..3_600 => (s.checked_div(60).unwrap_or(0), "m"),
        3_600..86_400 => (s.checked_div(3_600).unwrap_or(0), "h"),
        _ => (s.checked_div(86_400).unwrap_or(0), "d"),
    };
    format!("{n}{unit} ago")
}

/// Lamports as ZINK with up to 9 decimals, trailing zeros trimmed.
fn zink(lamports: u64) -> String {
    let whole = lamports.checked_div(LAMPORTS_PER_ZINK).unwrap_or(0);
    let frac = lamports.checked_rem(LAMPORTS_PER_ZINK).unwrap_or(0);
    if frac == 0 {
        return whole.to_string();
    }
    let digits = format!("{frac:09}");
    format!("{whole}.{}", digits.trim_end_matches('0'))
}

/// `part / whole` clamped to 0..=1, to a tenth of a percent, without float casts.
fn fraction(part: u64, whole: u64) -> f32 {
    let permille = part
        .saturating_mul(1_000)
        .checked_div(whole)
        .unwrap_or(1_000)
        .min(1_000);
    f32::from(u16::try_from(permille).unwrap_or(1_000)) / 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_zink() {
        assert_eq!(zink(50_000_000), "0.05");
        assert_eq!(zink(1_000_000_000), "1");
        assert_eq!(zink(1_000_000_001), "1.000000001");
    }

    #[test]
    fn clamps_fraction() {
        assert!((fraction(5, 10) - 0.5).abs() < f32::EPSILON);
        assert!((fraction(20, 10) - 1.0).abs() < f32::EPSILON);
        assert!((fraction(1, 0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn ages() {
        assert_eq!(age(100, 100), "0s ago");
        assert_eq!(age(7_200, 0), "2h ago");
    }
}
