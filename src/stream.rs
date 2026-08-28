//! `quivive tile --stream`: the pwetty push contract — spawn once, emit one
//! JSON line per CHANGE, stay alive, exit cleanly on stdout EOF.
//!
//! S9 of `docs/spec.md`, verbatim: "quivive tile --stream follows the pwetty
//! push contract: spawn once, emit exactly one JSON line per CHANGE in the
//! payload, stay alive between changes, exit cleanly on stdout EOF; pwetty
//! keeps the last content and respawns after ~1 s."
//!
//! Three design decisions this module makes, all load-bearing:
//!
//! 1. **No caller-side mtime pruning (`reader::unchanged`), unlike
//!    `watch.rs`.** `tests/bench.rs`'s own `WARM_CEILING` comment says why a
//!    plain resumed tick is affordable at 1 Hz: "the number the whole design
//!    exists to make possible... what makes ticking at 1 Hz affordable" — the
//!    resume cursor already makes [`tile::build`] cheap to call every tick.
//!    Adding a second, external skip layer on top would buy nothing here and
//!    would cost real correctness: `reader::unchanged`'s watermark only ever
//!    advances on a tick that actually reads, so a repo that stops being
//!    *written to* — an agent going quiet with no further ledger lines —
//!    would never be re-read, and its ACTIVE/IDLE/STALE/DEAD state would
//!    freeze at whatever it was on the last write forever, never decaying
//!    with the clock the way `state::assess` intends. `watch.rs` can accept
//!    that trade for its own 2s notification cadence; a stream mode whose
//!    entire job is "the tile is honest right now" cannot.
//! 2. **Change detection is a byte-compare of the compact JSON, `at`
//!    excluded.** [`tile::Payload::at`] is the tick clock and differs on
//!    almost every tick even when nothing else does; comparing the raw line
//!    would make S9's "one line per CHANGE" fire every tick regardless of
//!    payload content, which is the one failure mode S9 exists to rule out.
//!    Excluding just `at` (not `remaining_ttl` or any other field) is
//!    deliberate: a `DeadHoldingPaths` item's countdown is content a person
//!    watching a bar wants refreshed, the same way a clock's own hand moving
//!    is not noise. See [`comparison_key`].
//! 3. **The first tick always emits.** `last` starts `None`, which can never
//!    equal `Some(key)` — pwetty just spawned this process (or is about to
//!    respawn it, per the module doc's own "~1 s" line) and has nothing of
//!    ours to compare against, or stale content from a previous life; either
//!    way the first real payload is always new information to it.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::cli::Common;
use crate::state::Thresholds;
use crate::{dur, registry, tile};

/// Default interval between ticks in `--stream` mode.
///
/// ~1s: `docs/spec.md` S9 itself says pwetty "respawns after ~1 s", so a tick
/// much slower than that would leave pwetty holding stale content longer than
/// its own respawn window already tolerates, and a tick much faster buys
/// nothing pwetty could redraw before its next respawn either. `tests/bench.rs`'s
/// `WARM_CEILING` (50ms, a 20x margin at 1 Hz) is the other half of why 1s is
/// affordable rather than just plausible.
pub const INTERVAL_DEFAULT: &str = "1s";

/// One iteration's outcome, named rather than left as a bare bool so `run`'s
/// loop reads as what S9 actually asks for, not as a generic "did it write."
#[derive(Debug)]
enum Step {
    /// The payload's content (`at` excluded) matches `last`; nothing was
    /// written this tick.
    Unchanged,
    /// A changed payload was written and flushed; carries the new
    /// comparison key for the caller to remember as `last`.
    Emitted(String),
    /// The write failed with a broken pipe: the consumer is gone. Not an
    /// error — see [`emit`].
    ConsumerGone,
}

/// Build one tick's payload and serialize it COMPACT — one line, since pwetty
/// splits on newlines and a pretty-printed payload would be many.
fn build_line(
    repo_roots: &[PathBuf],
    now: DateTime<Utc>,
    thresholds: &Thresholds,
    use_cursor: bool,
    degrade_unreadable: bool,
) -> Result<String> {
    let payload = tile::build(repo_roots, now, thresholds, use_cursor, degrade_unreadable)?;
    Ok(serde_json::to_string(&payload)?)
}

/// The identity a tick's line is compared against: `line`'s own JSON with the
/// `at` field removed. See the module doc's point 2 for why `at` alone is
/// excluded.
fn comparison_key(line: &str) -> Result<String> {
    let mut value: serde_json::Value = serde_json::from_str(line)?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("at");
    }
    Ok(serde_json::to_string(&value)?)
}

/// Write `line` and flush, translating a broken pipe into "the consumer is
/// gone" rather than an error: pwetty closing its read end (S9: "exit
/// cleanly on stdout EOF") is the intended, ordinary way this loop ends, not
/// a fault — a stderr scream on every bar reload would be exactly the noise
/// S9's contract is designed to avoid. Any other I/O failure is real and
/// propagates.
fn emit(out: &mut dyn Write, line: &str) -> Result<bool> {
    match writeln!(out, "{line}").and_then(|()| out.flush()) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// One tick: build the payload, decide against `last` whether it changed, and
/// emit if so. Pure aside from `out`, and takes `out` as a trait object
/// precisely so tests can drive it against an in-memory buffer or a real
/// closed pipe without spawning the binary.
#[allow(clippy::too_many_arguments)]
fn step(
    out: &mut dyn Write,
    repo_roots: &[PathBuf],
    now: DateTime<Utc>,
    thresholds: &Thresholds,
    use_cursor: bool,
    degrade_unreadable: bool,
    last: Option<&str>,
) -> Result<Step> {
    let line = build_line(repo_roots, now, thresholds, use_cursor, degrade_unreadable)?;
    let key = comparison_key(&line)?;
    if last == Some(key.as_str()) {
        return Ok(Step::Unchanged);
    }
    if !emit(out, &line)? {
        return Ok(Step::ConsumerGone);
    }
    Ok(Step::Emitted(key))
}

/// Stream one tile line per change, following the pwetty push contract (S9).
///
/// `interval` is the sleep between ticks — `--interval` on `quivive tile
/// --stream`, defaulting to [`INTERVAL_DEFAULT`]. Thresholds and the
/// registry-vs-`--repo` choice come from `common` exactly as they do for the
/// one-shot tile (`src/main.rs`'s `tick_once`): this module cannot reuse that
/// function directly (it is private to the binary crate), so the same few
/// lines are repeated here rather than exposed as new surface neither caller
/// needs beyond this.
pub fn run(common: &Common, interval: Duration) -> Result<()> {
    let thresholds = Thresholds {
        active: dur::parse(&common.active_window)?,
        idle: dur::parse(&common.idle_window)?,
        dead: dur::parse(&common.dead_window)?,
        forget: dur::parse(&common.forget)?,
    };
    thresholds.validate()?;

    let (repo_roots, degrade_unreadable) = match &common.repo {
        Some(path) => (vec![path.clone()], false),
        None => (registry::read()?, true),
    };

    let mut out = std::io::stdout().lock();
    let mut last: Option<String> = None;

    loop {
        // One clock read per tick, taken here and passed down — see
        // `crate::now`'s doc on why a tick reads the clock exactly once.
        let now = crate::now()?;
        match step(
            &mut out,
            &repo_roots,
            now,
            &thresholds,
            !common.no_cursor,
            degrade_unreadable,
            last.as_deref(),
        )? {
            Step::ConsumerGone => return Ok(()),
            Step::Emitted(key) => last = Some(key),
            Step::Unchanged => {}
        }
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn base_now() -> DateTime<Utc> {
        "2026-08-28T12:00:00Z".parse().unwrap()
    }

    fn ts(offset_secs: i64) -> DateTime<Utc> {
        base_now() + TimeDelta::seconds(offset_secs)
    }

    fn thresholds() -> Thresholds {
        Thresholds {
            active: Duration::from_secs(60),
            idle: Duration::from_secs(300),
            dead: Duration::from_secs(3600),
            forget: Duration::from_secs(86_400),
        }
    }

    /// A minimal `.pact/` fixture, written the way pact itself writes one —
    /// its own tempdir, not shared with any other test module (each test gets
    /// a fresh one via `new()`).
    struct Repo {
        dir: tempfile::TempDir,
    }

    impl Repo {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let state = dir.path().join(".pact");
            std::fs::create_dir_all(&state).unwrap();
            std::fs::write(state.join("events.jsonl"), "").unwrap();
            Self { dir }
        }

        fn root(&self) -> PathBuf {
            self.dir.path().to_path_buf()
        }

        fn event(&self, agent: &str, at: DateTime<Utc>) {
            let line = format!(
                r#"{{"at":"{}","agent":"{}","kind":"acquired","path":"src/{}.rs","detail":null,"ttl_secs":900,"chain_hash":"deadbeef"}}"#,
                at.to_rfc3339(),
                agent,
                agent,
            );
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.path().join(".pact").join("events.jsonl"))
                .unwrap();
            writeln!(f, "{line}").unwrap();
        }
    }

    // -----------------------------------------------------------------------
    // The `at` field problem: two ticks over identical evidence, seconds
    // apart, must not look like a change.
    // -----------------------------------------------------------------------

    #[test]
    fn comparison_key_ignores_at_but_nothing_else() {
        let a = r#"{"v":1,"at":"2026-08-28T12:00:00Z","status":"active","repos":[]}"#;
        let b = r#"{"v":1,"at":"2026-08-28T12:00:05Z","status":"active","repos":[]}"#;
        assert_eq!(
            comparison_key(a).unwrap(),
            comparison_key(b).unwrap(),
            "an `at`-only difference must compare equal"
        );

        let c = r#"{"v":1,"at":"2026-08-28T12:00:00Z","status":"drained","repos":[]}"#;
        assert_ne!(
            comparison_key(a).unwrap(),
            comparison_key(c).unwrap(),
            "a real content difference must still compare unequal"
        );
    }

    #[test]
    fn two_identical_ticks_over_unchanged_evidence_emit_exactly_one_line() {
        let repo = Repo::new();
        repo.event("agent-1", ts(-10));
        let roots = vec![repo.root()];
        let th = thresholds();
        let mut out = Vec::new();

        let first = step(&mut out, &roots, ts(0), &th, true, true, None).unwrap();
        let last = match first {
            Step::Emitted(key) => key,
            other => panic!("the first tick must always emit: {other:?}"),
        };

        // A second tick, a second later, over the SAME evidence: only `at`
        // differs in the raw payload.
        let second = step(&mut out, &roots, ts(1), &th, true, true, Some(&last)).unwrap();
        assert!(
            matches!(second, Step::Unchanged),
            "an at-only difference must not emit: {second:?}"
        );

        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text.lines().count(),
            1,
            "exactly one line for two identical ticks: {text:?}"
        );
    }

    #[test]
    fn a_fixture_change_between_ticks_emits_a_second_line() {
        let repo = Repo::new();
        repo.event("agent-1", ts(-10));
        let roots = vec![repo.root()];
        let th = thresholds();
        let mut out = Vec::new();

        let first = step(&mut out, &roots, ts(0), &th, true, true, None).unwrap();
        let last = match first {
            Step::Emitted(key) => key,
            other => panic!("the first tick must always emit: {other:?}"),
        };

        // A second agent shows up between ticks: a real change to `repos[]`,
        // not just the clock.
        repo.event("agent-2", ts(1));
        let second = step(&mut out, &roots, ts(2), &th, true, true, Some(&last)).unwrap();
        assert!(
            matches!(second, Step::Emitted(_)),
            "a real fixture change must emit: {second:?}"
        );

        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text.lines().count(),
            2,
            "one line per tick that actually changed: {text:?}"
        );
        let v0: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        let v1: serde_json::Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
        assert_eq!(v0["repos"][0]["agents"]["active"], 1);
        assert_eq!(v1["repos"][0]["agents"]["active"], 2);
    }

    // -----------------------------------------------------------------------
    // Clean EOF: a closed read end is "the consumer is gone," not an error.
    // -----------------------------------------------------------------------

    #[test]
    fn a_closed_read_end_reports_consumer_gone_not_an_error() {
        // A real closed pipe, not a mock — BrokenPipe is a property of the
        // kernel object, and this is the exact condition S9 calls "exit
        // cleanly on stdout EOF": pwetty closed its end and expects this
        // process to stop quietly, not to scream on stderr.
        let mut child = std::process::Command::new("true")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("spawning `true` must succeed");
        // Take the handle before waiting: `Child::wait` drops `child.stdin`
        // as part of reaping the process, so a `.take()` after it always
        // sees `None` regardless of whether the pipe was ever connected.
        let mut sink = child.stdin.take().expect("stdin was piped");
        // `true` exits immediately without reading stdin; waiting for it
        // guarantees the read end is gone before we write.
        child.wait().expect("`true` must exit");

        let result = emit(
            &mut sink,
            r#"{"v":1,"at":"x","status":"no-fleet","repos":[]}"#,
        );
        assert!(
            matches!(result, Ok(false)),
            "a write past a closed read end must report `consumer gone`, not error: {result:?}"
        );
    }
}
