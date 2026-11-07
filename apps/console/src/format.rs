//! Text formatting for the console: times, ZINK amounts, shortened addresses and outcomes.

use crate::log::{Entry, SIGN_INTENT};

const LAMPORTS_PER_ZINK: u64 = 1_000_000_000;

/// What a row's outcome means: a merged sign whose intent was pending reads as its final outcome.
pub fn outcome_label(e: &Entry, unresolved: bool) -> String {
    if unresolved && e.command == SIGN_INTENT {
        "unresolved".to_owned()
    } else {
        e.outcome.clone()
    }
}

/// UTC date and time, and the age.
pub fn when(now: u64, ts: u64) -> String {
    format!(
        "{} UTC ({})",
        utc(ts).unwrap_or_else(|| ts.to_string()),
        age(now, ts)
    )
}

/// `YYYY-MM-DD HH:MM:SS` for a unix time, by the civil-from-days algorithm (Howard Hinnant).
pub fn utc(ts: u64) -> Option<String> {
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
pub fn shorten_addresses(text: &str) -> String {
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
pub fn signed_zink(lamports: i64) -> String {
    let sign = if lamports < 0 { "-" } else { "" };
    format!("{sign}{} ZINK", zink(lamports.unsigned_abs()))
}

pub fn age(now: u64, ts: u64) -> String {
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
pub fn zink(lamports: u64) -> String {
    let whole = lamports.checked_div(LAMPORTS_PER_ZINK).unwrap_or(0);
    let frac = lamports.checked_rem(LAMPORTS_PER_ZINK).unwrap_or(0);
    if frac == 0 {
        return whole.to_string();
    }
    let digits = format!("{frac:09}");
    format!("{whole}.{}", digits.trim_end_matches('0'))
}

/// `part / whole` clamped to 0..=1, to a tenth of a percent, without float casts.
pub fn fraction(part: u64, whole: u64) -> f32 {
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
