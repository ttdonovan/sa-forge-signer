//! Timings of the audit log's reads on synthetic logs, for decisions about the log's layout. Run
//! with `just bench`; ignored by `cargo test`. Figures depend on the machine: compare runs, not
//! absolute numbers.
#![expect(
    clippy::arithmetic_side_effects,
    reason = "test-only timing harness on small fixed inputs"
)]

use std::time::Instant;

use crate::audit::{Audit, Entry, LOOKBACK};

fn line(ts: u64, command: &str, i: u64) -> String {
    let e = Entry {
        ts,
        key: "alice-session".into(),
        command: command.into(),
        payload_hash: format!("{i:064x}"),
        programs: vec!["C4SAgeKLgb3pTLWhVr6NRwWyYFuTR7ZeSXFrzoLwfMzF".into()],
        summary: "Move fleet ALPHA-1 from (12, -4) to (13, -4) and start mining".into(),
        outcome: if command == "sign" {
            "confirmed"
        } else {
            "pending"
        }
        .into(),
        signature: Some(format!("{i:088}")),
        slot: Some(i),
        balance_change: Some(-5_000),
        policy: Some(format!("{:064x}", 7)),
        prev: format!("{i:064x}"),
        hash: format!("{:064x}", i.saturating_add(1)),
        ..Entry::default()
    };
    let mut s = serde_json::to_string(&e).unwrap();
    s.push('\n');
    s
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

/// One key signing `per_day` times a day for `days` days, up to `now`: an intent and a final each.
fn run(name: &str, per_day: u64, days: u64) {
    let dir = std::env::temp_dir().join(format!(
        "sa-forge-signer-bench-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let now: u64 = 1_800_000_000;
    let span = days * 86_400;
    let total = per_day * days;
    let mut text = String::new();
    for i in 0..total {
        let ts = now - span + i * span / total;
        text.push_str(&line(ts, "sign-intent", 2 * i));
        text.push_str(&line(ts, "sign", 2 * i + 1));
    }
    std::fs::write(dir.join("audit.jsonl"), &text).unwrap();
    let mb = f64::from(u32::try_from(text.len() / 1_000).unwrap()) / 1e3;
    drop(text);
    let audit = Audit::new(&dir);
    let key = crate::config::Config::load_str(
        "[keys.alice-session]\nclass = \"session\"\npubkey = \"7JJhMrnvgqxS6MW59QQcr2Ki1NjZ7SrAjST4YtAFE8zR\"\n",
    )
    .unwrap()
    .keys
    .remove(0);

    let t = Instant::now();
    let all = audit.entries().unwrap().len();
    let full = ms(t);
    let t = Instant::now();
    let window = audit.recent(now).unwrap();
    let back = ms(t);
    let t = Instant::now();
    let usage = crate::facts::usage(&window, &key, now);
    let count = ms(t);
    let t = Instant::now();
    audit
        .append(Entry {
            ts: now,
            key: "alice-session".into(),
            command: "bench".into(),
            ..Entry::default()
        })
        .unwrap();
    let append = ms(t);
    // A sign: one read-back, usage before and after simulation, the intent and the final.
    let sign = (count + append).mul_add(2.0, back);
    println!(
        "| {name} | {per_day}/day x {days} d | {all} / {mb:.0} MB | {full:.0} ms | {back:.1} ms ({} entries, {}h) | {count:.2} ms | {append:.2} ms | {sign:.0} ms |",
        window.len(),
        LOOKBACK / 3_600
    );
    assert!(usage.spent > 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "timing; run with `just bench`"]
fn audit_reads() {
    println!("| log | rate | entries / size | full read | read-back | usage | append | per sign |");
    println!("|---|---|---|---|---|---|---|---|");
    run("steady-month", 2_000, 30);
    run("steady-year", 2_000, 365);
    run("one key at its limit, a week", 43_200, 7);
}
