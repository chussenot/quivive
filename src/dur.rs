//! Durations in and out: `--active-window 60s` on the way in, `6m52s` on the way
//! out.
//!
//! Hand-rolled rather than reached for a crate, because the two directions want
//! different things and neither is more than thirty lines: the parser accepts
//! what a person types on a bar's config line, and the formatter is a *rendering*
//! choice that only the text tile uses — the JSON tile emits `age_s` as an
//! integer precisely so a consumer is never handed a string it cannot get back
//! from (`docs/tile-contract.md`).

use std::time::Duration;

use anyhow::{Result, bail};

/// `90` (bare seconds), `45s`, `5m`, `2h`, `1d`.
///
/// A bare number means seconds. That is a deliberate kindness on a flag people
/// type into a status-bar config, and it is unambiguous because no unit is
/// spelled with a leading digit.
pub fn parse(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        bail!("empty duration");
    }
    let (digits, unit) = match s.find(|c: char| !c.is_ascii_digit()) {
        Some(i) => s.split_at(i),
        None => (s, "s"),
    };
    if digits.is_empty() {
        bail!("`{s}` has no number in it; try 60s, 5m or 2h");
    }
    let n: u64 = digits
        .parse()
        .map_err(|_| anyhow::anyhow!("`{digits}` is not a whole number of {unit}"))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        other => bail!("unknown unit `{other}`; use s, m, h or d"),
    };
    Ok(Duration::from_secs(secs))
}

/// `43s`, `6m52s`, `2h13m`, `3d4h`.
///
/// Coarsens as it grows, and that is the point: on a bar there is room for two
/// significant units and no more, and `7852s` is a number a human has to do
/// arithmetic on. The one place precision is kept to the second is under a
/// minute, where the difference between 5s and 55s is the difference between
/// "working" and "about to go idle".
pub fn human(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m{}s", s / 60, s % 60),
        3600..86_400 => format!("{}h{}m", s / 3600, (s % 3600) / 60),
        _ => format!("{}d{}h", s / 86_400, (s % 86_400) / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_numbers_are_seconds() {
        assert_eq!(parse("90").unwrap(), Duration::from_secs(90));
        assert_eq!(parse("90s").unwrap(), Duration::from_secs(90));
    }

    #[test]
    fn every_unit_scales() {
        assert_eq!(parse("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse("1d").unwrap(), Duration::from_secs(86_400));
    }

    #[test]
    fn nonsense_is_refused_with_the_units_named() {
        for bad in ["", "m", "5x", "-1", "5 m"] {
            assert!(parse(bad).is_err(), "`{bad}` should not parse");
        }
        let msg = parse("5x").unwrap_err().to_string();
        assert!(msg.contains("use s, m, h or d"), "unhelpful: {msg}");
    }

    #[test]
    fn human_keeps_two_units_and_never_more() {
        assert_eq!(human(0), "0s");
        assert_eq!(human(43), "43s");
        assert_eq!(human(412), "6m52s");
        assert_eq!(human(7980), "2h13m");
        assert_eq!(human(273_600), "3d4h");
    }

    #[test]
    fn a_negative_age_reads_as_zero_rather_than_as_a_minus_sign() {
        // A clock that moved backwards between the ledger's newest line and this
        // tick is a real thing (pact keeps a `clock_watermark` for exactly that),
        // and `-3s` on a status bar is a bug report from a user.
        assert_eq!(human(-3), "0s");
    }
}
