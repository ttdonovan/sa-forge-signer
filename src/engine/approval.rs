//! The `confirm` prompt at the controlling terminal.

use std::io::{BufRead, BufReader, Write};

use anyhow::Result;

use super::Report;
use crate::config::KeyConfig;
use crate::text;

/// Asks on the controlling terminal, never on stdin, so an agent piping input cannot approve.
pub fn confirm(report: &Report, key: &KeyConfig) -> Result<bool> {
    let Ok(mut tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return Ok(false);
    };
    write!(tty, "{}", approval_prompt(report, key))?;
    tty.flush()?;
    let mut answer = String::new();
    BufReader::new(tty).read_line(&mut answer)?;
    Ok(answer.trim() == "yes")
}

/// The `confirm` prompt: the signer's own figures first, then the builder's summary, escaped and
/// capped so it cannot redraw the lines above it.
pub fn approval_prompt(report: &Report, key: &KeyConfig) -> String {
    let spend = report
        .balance_change
        .map_or_else(|| "unknown".to_owned(), |d| format!("{d} lamports"));
    format!(
        "\nsa-forge-signer: approve a signature with {:?} ({:?} key)?\n  simulated balance change: {spend}\n  compute units: {}\n  summary (from the builder, not verified): {}\nType 'yes' to sign: ",
        key.name,
        key.class,
        report
            .compute_units
            .map_or_else(|| "unknown".to_owned(), |u| u.to_string()),
        text::printable(&report.summary, text::MAX_PRINTED),
    )
}
