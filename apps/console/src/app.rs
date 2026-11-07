//! The window: status bar, keys with their limits, the audit log, and the selected entry.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};

use crate::Health;
use crate::config::{self, Config};
use crate::log::{Chain, Entry, Row, Snapshot};

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
    /// One row per audit entry instead of per signature.
    raw: bool,
    /// Index into `log.entries` of the selected row's entry.
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
            raw: false,
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

    /// Rows newest first, one per signature unless `raw`, after the key and unresolved filters.
    fn visible_rows(&self) -> Vec<Row> {
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
                    self.filter.as_ref().is_none_or(|f| &e.key == f)
                        && (!self.unresolved_only || self.log.unresolved(e))
                })
            })
            .collect()
    }

    fn entries_table(&mut self, ui: &mut egui::Ui, now: u64) {
        ui.horizontal(|ui| {
            ui.heading("Audit log");
            ui.checkbox(&mut self.unresolved_only, "unresolved only");
            ui.checkbox(&mut self.raw, "raw entries")
                .on_hover_text("One row per audit entry, as the hash chain stores them, instead of one per signature.");
            if let Some(f) = &self.filter {
                ui.label(format!("key: {f}"));
            }
        });
        let rows = self.visible_rows();
        if rows.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.weak(if self.log.entries.is_empty() {
                    "No entries yet: the log fills as the agent checks and signs."
                } else {
                    "No entries match the filters."
                });
            });
            return;
        }
        let mut clicked = None;
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .sense(egui::Sense::click())
            .column(Column::initial(70.0))
            .column(Column::initial(110.0));
        if self.raw {
            table = table.column(Column::initial(90.0));
        }
        let mut titles = vec!["age", "key"];
        if self.raw {
            titles.push("command");
        }
        titles.extend(["outcome", "intent / summary"]);
        table
            .column(Column::initial(90.0))
            .column(Column::remainder().clip(true))
            .header(20.0, |mut h| {
                for title in titles {
                    h.col(|ui| {
                        ui.strong(title);
                    });
                }
            })
            .body(|body| {
                body.rows(18.0, rows.len(), |mut row| {
                    let Some(r) = rows.get(row.index()) else {
                        return;
                    };
                    let Some(e) = self.log.entries.get(r.entry) else {
                        return;
                    };
                    row.set_selected(self.selected == Some(r.entry));
                    let unresolved = self.log.unresolved(e);
                    row.col(|ui| {
                        ui.label(age(now, e.ts));
                    });
                    row.col(|ui| {
                        ui.label(&e.key);
                    });
                    if self.raw {
                        row.col(|ui| {
                            ui.label(&e.command);
                        });
                    }
                    row.col(|ui| {
                        ui.label(
                            RichText::new(outcome_label(e, unresolved))
                                .color(outcome_color(e, unresolved)),
                        );
                    });
                    row.col(|ui| {
                        let summary = shorten_addresses(&e.summary);
                        match &e.intent {
                            Some(intent) => {
                                ui.label(intent);
                                ui.weak(summary);
                            }
                            None => {
                                ui.label(summary);
                            }
                        }
                    });
                    if row.response().clicked() {
                        clicked = Some(r.entry);
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

    /// The selected row: what was signed, what happened, and the entries that record it.
    fn detail(&mut self, ui: &mut egui::Ui, now: u64, row: Row) {
        let Some(e) = self.log.entries.get(row.entry).cloned() else {
            return;
        };
        let intent = row.intent.and_then(|i| self.log.entries.get(i)).cloned();
        let unresolved = self.log.unresolved(&e);
        let explorer = self.config.explorer_url.clone();
        let link = |path: &str, value: &str| {
            explorer
                .as_ref()
                .map(|base| format!("{base}/{path}/{value}"))
        };
        ui.horizontal(|ui| {
            ui.heading("Entry");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("close")
                    .on_hover_text("Close (click the row again)")
                    .clicked()
                {
                    self.selected = None;
                }
            });
        });
        ui.label(
            RichText::new(outcome_label(&e, unresolved))
                .color(outcome_color(&e, unresolved))
                .strong(),
        );
        if unresolved {
            ui.label(
                RichText::new("Sent with no known outcome: reconcile before retrying.").color(WARN),
            );
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            section(ui, "What", |ui| {
                text_field(ui, "intent", e.intent.as_deref().unwrap_or("-"));
                text_field(ui, "summary", &e.summary)
                    .on_hover_text("Written by the payload's builder; the signer does not verify it.");
                for p in &e.programs {
                    id_field(ui, "program", p, link("address", p));
                }
            });
            section(ui, "Result", |ui| {
                text_field(ui, "key", &e.key);
                if let Some(i) = &intent {
                    text_field(ui, "sent", &when(now, i.ts));
                }
                text_field(ui, if intent.is_some() { "settled" } else { "time" }, &when(now, e.ts));
                if let Some(d) = &e.detail {
                    text_field(ui, "detail", d);
                }
                if let Some(sig) = &e.signature {
                    id_field(ui, "signature", sig, link("tx", sig));
                }
                if let Some(slot) = e.slot {
                    id_field(ui, "slot", &slot.to_string(), link("block", &slot.to_string()));
                }
                if let Some(d) = e.balance_change {
                    text_field(ui, "balance change", &signed_zink(d));
                }
                if let Some(d) = intent.as_ref().and_then(|i| i.balance_change) {
                    text_field(ui, "reserved", &signed_zink(d))
                        .on_hover_text("The simulated spend the signer reserved against the daily cap before sending.");
                }
            });
            section(ui, "Integrity", |ui| {
                id_field(ui, "payload hash", &e.payload_hash, None);
                id_field(ui, "entry hash", &e.hash, None);
                id_field(ui, "previous hash", &e.prev, None);
                if let Some(i) = &intent {
                    id_field(ui, "intent entry hash", &i.hash, None);
                }
            });
        });
    }
}

/// A collapsible group of fields.
fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(true)
        .show(ui, add);
}

/// A label over a plain value: intents, summaries, times and amounts.
fn text_field(ui: &mut egui::Ui, name: &str, value: &str) -> egui::Response {
    ui.weak(name);
    labelled(ui, RichText::new(value))
}

/// A label over a monospace identifier (address, signature, slot, hash); with `url`, a copy button
/// and a link out.
fn id_field(ui: &mut egui::Ui, name: &str, value: &str, url: Option<String>) -> egui::Response {
    ui.horizontal(|ui| {
        ui.weak(name);
        if let Some(u) = url {
            if ui.small_button("copy").clicked() {
                ui.ctx().copy_text(value.to_owned());
            }
            ui.hyperlink_to("explorer ↗", u);
        }
    });
    labelled(ui, RichText::new(value).monospace())
}

fn labelled(ui: &mut egui::Ui, value: RichText) -> egui::Response {
    let r = ui.add(egui::Label::new(value).wrap());
    ui.add_space(4.0);
    r
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
        if let Some(i) = self.selected {
            // In the merged view the selection is the final entry; find its intent.
            let row = if self.raw {
                None
            } else {
                self.log.rows().into_iter().find(|r| r.entry == i)
            }
            .unwrap_or(Row {
                entry: i,
                intent: None,
            });
            egui::Panel::right("detail")
                .resizable(true)
                .default_size(420.0)
                .show(ui, |ui| self.detail(ui, now, row));
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

/// What a row's outcome means: a merged sign whose intent was pending reads as its final outcome.
fn outcome_label(e: &Entry, unresolved: bool) -> String {
    if unresolved && e.command == crate::log::SIGN_INTENT {
        "unresolved".to_owned()
    } else {
        e.outcome.clone()
    }
}

/// UTC date and time, and the age.
fn when(now: u64, ts: u64) -> String {
    format!(
        "{} UTC ({})",
        utc(ts).unwrap_or_else(|| ts.to_string()),
        age(now, ts)
    )
}

/// `YYYY-MM-DD HH:MM:SS` for a unix time, by the civil-from-days algorithm (Howard Hinnant).
fn utc(ts: u64) -> Option<String> {
    let days = ts.checked_div(86_400)?;
    let secs = ts.checked_rem(86_400)?;
    let z = days.checked_add(719_468)?;
    let era = z.checked_div(146_097)?;
    let doe = z.checked_rem(146_097)?;
    let yoe = doe
        .checked_sub(doe.checked_div(1_460)?)?
        .checked_add(doe.checked_div(36_524)?)?
        .checked_sub(doe.checked_div(146_096)?)?
        .checked_div(365)?;
    let doy = doe.checked_sub(
        yoe.checked_mul(365)?
            .checked_add(yoe.checked_div(4)?)?
            .checked_sub(yoe.checked_div(100)?)?,
    )?;
    let mp = doy.checked_mul(5)?.checked_add(2)?.checked_div(153)?;
    let day = doy
        .checked_sub(mp.checked_mul(153)?.checked_add(2)?.checked_div(5)?)?
        .checked_add(1)?;
    let month = if mp < 10 {
        mp.checked_add(3)?
    } else {
        mp.checked_sub(9)?
    };
    let year = yoe
        .checked_add(era.checked_mul(400)?)?
        .checked_add(u64::from(month <= 2))?;
    Some(format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        secs.checked_div(3_600)?,
        secs.checked_rem(3_600)?.checked_div(60)?,
        secs.checked_rem(60)?
    ))
}

/// Base58 runs of 32 to 44 characters (addresses, signatures are longer) shortened to `GwDE…dFvg`.
fn shorten_addresses(text: &str) -> String {
    const BASE58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let n = run.chars().count();
        if (32..=44).contains(&n) {
            out.extend(run.chars().take(4));
            out.push('…');
            out.extend(run.chars().skip(n.saturating_sub(4)));
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in text.chars() {
        if BASE58.contains(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// A signed lamport change as ZINK, e.g. `-0.000005 ZINK`.
fn signed_zink(lamports: i64) -> String {
    let sign = if lamports < 0 { "-" } else { "" };
    format!("{sign}{} ZINK", zink(lamports.unsigned_abs()))
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
    fn formats_utc() {
        assert_eq!(utc(0).as_deref(), Some("1970-01-01 00:00:00"));
        assert_eq!(utc(951_782_400).as_deref(), Some("2000-02-29 00:00:00"));
        assert_eq!(utc(1_794_087_225).as_deref(), Some("2026-11-07 21:33:45"));
    }

    #[test]
    fn shortens_addresses_only() {
        assert_eq!(
            shorten_addresses(
                "Supply fleet at GwDEHQLddZwKoHzpv3DmtJqPkgZQ8iAv27zwQVCKdFvg: load 1880 fuel"
            ),
            "Supply fleet at GwDE…dFvg: load 1880 fuel"
        );
        assert_eq!(shorten_addresses("Daily check-in"), "Daily check-in");
        assert_eq!(signed_zink(-5_000), "-0.000005 ZINK");
    }

    #[test]
    fn ages() {
        assert_eq!(age(100, 100), "0s ago");
        assert_eq!(age(7_200, 0), "2h ago");
    }
}
