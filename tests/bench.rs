//! The ceilings `docs/spec.md#the-tick` sets, measured rather than asserted in
//! prose: `the_tick_ceilings_hold` for the single-repo cold/warm numbers, and
//! `steady_state_registry_tick_meets_s6` for S6's own words — "steady-state
//! tick cost is under 10 ms per repo" — measured the way a fleet actually
//! accrues that cost: across a *registry* of many repos, most of them pruned
//! by mtime (S5) rather than re-read.
//!
//! `#[ignore]` and release-only, run by `mise run bench`. Deliberately NOT part
//! of `mise run check`: both write synthetic ledgers and take time, and a
//! required gate with those properties is one people switch off.
//!
//! Both refuse to assert under `debug_assertions`, because a debug fold over a
//! synthetic ledger measures the profile rather than the code — and a ceiling
//! that fails for the profile is a ceiling somebody deletes.

mod support;

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::TimeDelta;
use quivive::reader;
use quivive::state::Thresholds;
use quivive::tile;

/// The size the ceilings are quoted at. Large enough to be a busy long-lived
/// repository, and 20x pact's own rewrite threshold of 5000 lines — so this also
/// measures the case pact itself would have compacted away, which is the
/// pessimistic one.
const EVENTS: usize = 100_000;

/// A warm tick is the number the whole design exists to make possible: it is what
/// makes ticking at 1 Hz affordable, and its failure is the documented reversal
/// condition for the "no daemon" deferral (D2). 50 ms leaves a 20x margin at 1 Hz.
const WARM_CEILING: Duration = Duration::from_millis(50);

/// A cold tick is allowed to be slow — it happens once, after a rewrite or a
/// deleted cursor. It is not allowed to be *wrong*, which `goldens.rs` asserts
/// and this only times. The ceiling exists so that "slow" cannot quietly become
/// "the bar hangs for a second".
const COLD_CEILING: Duration = Duration::from_millis(2000);

fn synthetic_ledger(dir: &std::path::Path, events: usize) {
    let state = dir.join(".pact");
    std::fs::create_dir_all(&state).unwrap();
    let f = std::fs::File::create(state.join("events.jsonl")).unwrap();
    let mut w = std::io::BufWriter::new(f);
    let base = support::now();
    // 64 agents, so the fold's map stays a realistic size while the line count
    // grows: a ledger of 100,000 events from 100,000 distinct agents would
    // measure BTreeMap insertion, not the read.
    for i in 0..events {
        let at = base - TimeDelta::seconds((events - i) as i64);
        writeln!(
            w,
            r#"{{"at":"{}","agent":"agent-{}","kind":"acquired","path":"src/f{}.rs","detail":null,"ttl_secs":900,"chain_hash":"deadbeef{}"}}"#,
            at.to_rfc3339(),
            i % 64,
            i % 512,
            i
        )
        .unwrap();
    }
    w.flush().unwrap();
}

fn tick(root: &std::path::Path, use_cursor: bool) -> Duration {
    let t = Instant::now();
    // `tile::tick` is the read-classify-commit sequence the real CLI runs per
    // repo, so this measures the actual per-tick cost rather than a bench-only
    // approximation of it.
    let _ = tile::tick(root, support::now(), &Thresholds::default(), use_cursor).unwrap();
    t.elapsed()
}

#[test]
#[ignore = "release-only, writes a large synthetic ledger: mise run bench"]
fn the_tick_ceilings_hold() {
    let dir = tempfile::tempdir().unwrap();
    synthetic_ledger(dir.path(), EVENTS);
    let bytes = std::fs::metadata(dir.path().join(".pact/events.jsonl"))
        .unwrap()
        .len();
    println!("ledger: {EVENTS} events, {} KiB", bytes / 1024);

    // Cold: no cursor exists yet, so this is the full parse.
    let cold = tick(dir.path(), true);
    println!("cold tick (full re-read of {EVENTS}): {cold:?}");

    // Warm: the cursor is now at EOF. This is the shape of every tick after the
    // first, and on an idle fleet it reads zero new bytes.
    let warm_idle = tick(dir.path(), true);
    println!("warm tick (nothing appended):        {warm_idle:?}");

    // Warm with real work: a handful of new events, which is a busy second.
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join(".pact/events.jsonl"))
            .unwrap();
        for i in 0..8 {
            writeln!(
                f,
                r#"{{"at":"{}","agent":"agent-{i}","kind":"renewed","path":"src/x.rs"}}"#,
                support::now().to_rfc3339()
            )
            .unwrap();
        }
    }
    let warm_busy = tick(dir.path(), true);
    println!("warm tick (8 events appended):       {warm_busy:?}");

    // A forced cold read, for the ratio. This is the number that says whether the
    // cursor is earning its keep at all.
    let forced = tick(dir.path(), false);
    println!("forced cold tick:                    {forced:?}");
    println!(
        "speedup, cold/warm: {:.0}x",
        forced.as_secs_f64() / warm_busy.as_secs_f64().max(1e-9)
    );

    if cfg!(debug_assertions) {
        println!(
            "\ndebug profile: NOT asserting. A debug fold over {EVENTS} events \
             measures the profile, not the code — run `mise run bench`."
        );
        return;
    }

    assert!(
        warm_idle < WARM_CEILING,
        "idle warm tick {warm_idle:?} exceeds {WARM_CEILING:?}"
    );
    assert!(
        warm_busy < WARM_CEILING,
        "busy warm tick {warm_busy:?} exceeds {WARM_CEILING:?} — this is the \
         documented reversal condition for the no-daemon deferral (D2), so a \
         genuine failure here is an ADR conversation, not a tuning exercise"
    );
    assert!(
        cold < COLD_CEILING,
        "cold tick {cold:?} exceeds {COLD_CEILING:?}"
    );
}

// ---------------------------------------------------------------------------
// S6, verbatim: "Steady-state tick cost is under 10 ms per repo, release
// profile, enforced by mise run bench." The bench above measures one repo's
// cold and warm cost in isolation; this one measures the number a fleet
// actually pays every second: a *registry* of many repos, ticked the way
// `quivive watch` ticks it — S5's mtime-prune gate first (`reader::unchanged`),
// a real (cursor-resumed) tick only for a repo that changed.
// ---------------------------------------------------------------------------

/// A registry this size is a busy but plausible fleet — large enough that a
/// per-repo constant (a `stat` here, a directory read there) shows up in the
/// average, small enough that the whole bench still runs in seconds, not
/// minutes.
const REGISTRY_REPOS: usize = 200;

/// Per repo, a realistic amount of history — a season of a project, not
/// `EVENTS`' synthetic worst case. S6's ceiling is about steady state, where
/// most of this has already been folded into a cursor and only the mtime scan
/// touches it every pass.
const EVENTS_PER_REPO: usize = 200;

/// One in ten repos gets new events between the cold pass and the timed
/// steady-state pass — a fleet where most repos are quiet at any given moment
/// and a handful are genuinely being worked, which is the normal case S6's
/// ceiling has to hold for, not the degenerate all-idle or all-busy ones.
const BUSY_EVERY: usize = 10;

/// Steady state, per repo, over the whole registry.
const REGISTRY_WARM_CEILING_PER_REPO: Duration = Duration::from_millis(10);

fn small_ledger(root: &Path, events: usize, agents: usize) {
    let state = root.join(".pact");
    std::fs::create_dir_all(&state).unwrap();
    let f = std::fs::File::create(state.join("events.jsonl")).unwrap();
    let mut w = std::io::BufWriter::new(f);
    // The real wall clock, deliberately, not `support::now()`'s frozen NOW:
    // `reader::unchanged` (S5's mtime-prune gate) compares against the
    // filesystem's own mtimes, which the OS stamps from the real clock. A
    // tick's judgment clock and a prune watermark can be frozen together
    // (ADR-0001's purity is about the FOLD, not about disk metadata this
    // bench never asks the fold to interpret) — but they cannot be frozen
    // against each other's different clock, or `unchanged` compares a fixed
    // timestamp against a real one and the gate stops meaning anything.
    let base = chrono::Utc::now();
    for i in 0..events {
        let at = base - TimeDelta::seconds((events - i) as i64);
        writeln!(
            w,
            r#"{{"at":"{}","agent":"agent-{}","kind":"acquired","path":"src/f{}.rs","detail":null,"ttl_secs":900,"chain_hash":"deadbeef{}"}}"#,
            at.to_rfc3339(),
            i % agents,
            i % 32,
            i
        )
        .unwrap();
    }
    w.flush().unwrap();
}

fn append_events(root: &Path, n: usize, tag: usize) {
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(root.join(".pact").join("events.jsonl"))
        .unwrap();
    for i in 0..n {
        writeln!(
            f,
            r#"{{"at":"{}","agent":"agent-live-{tag}-{i}","kind":"renewed","path":"src/x.rs"}}"#,
            chrono::Utc::now().to_rfc3339()
        )
        .unwrap();
    }
}

#[test]
#[ignore = "release-only, writes REGISTRY_REPOS synthetic ledgers: mise run bench"]
fn steady_state_registry_tick_meets_s6() {
    let base = tempfile::tempdir().unwrap();
    let roots: Vec<std::path::PathBuf> = (0..REGISTRY_REPOS)
        .map(|i| {
            let root = base.path().join(format!("repo-{i}"));
            small_ledger(&root, EVENTS_PER_REPO, 16);
            root
        })
        .collect();

    let thresholds = Thresholds::default();

    // Cold pass: establishes every repo's resume cursor, and a watermark —
    // exactly `watch::run`'s own bookkeeping (`src/watch.rs`), reproduced here
    // rather than imported because iterating a registry with `reader::unchanged`
    // is documented as the CALLER's job, not `tile`'s (see its doc comment) —
    // this bench is one such caller, same as `watch` is.
    let mut watermark: std::collections::HashMap<
        std::path::PathBuf,
        chrono::DateTime<chrono::Utc>,
    > = std::collections::HashMap::new();
    for root in &roots {
        let now = chrono::Utc::now();
        let _ = tile::tick(root, now, &thresholds, true).unwrap();
        watermark.insert(root.clone(), now);
    }

    // A whole-second gap: mtime resolution on some filesystems is coarse
    // enough that a write landing in the same second as the watermark can
    // compare equal rather than greater (documented on `reader::unchanged`).
    // Without this the busy repos below would sometimes get pruned instead of
    // re-read, and the steady-state pass would measure the wrong workload.
    std::thread::sleep(Duration::from_millis(1100));

    // Simulate the fraction of the registry that is genuinely busy this pass.
    for (i, root) in roots.iter().enumerate() {
        if i % BUSY_EVERY == 0 {
            append_events(root, 5, i);
        }
    }

    // The number S6 sets a ceiling on: one steady-state pass over the whole
    // registry, mtime-pruned first, a real cursor-resumed tick only where the
    // prune gate says something changed.
    let started = Instant::now();
    let mut pruned = 0usize;
    let mut reread = 0usize;
    for root in &roots {
        let now = chrono::Utc::now();
        let prior = *watermark.get(root).expect("every repo has a watermark");
        if reader::unchanged(root, prior) {
            pruned += 1;
            continue;
        }
        reread += 1;
        let _ = tile::tick(root, now, &thresholds, true).unwrap();
        watermark.insert(root.clone(), now);
    }
    let elapsed = started.elapsed();
    let per_repo = elapsed / REGISTRY_REPOS as u32;

    println!(
        "registry: {REGISTRY_REPOS} repos, {EVENTS_PER_REPO} events each, \
         1/{BUSY_EVERY} busy this pass"
    );
    println!("steady-state pass: {pruned} pruned, {reread} re-read, {elapsed:?} total");
    println!(
        "steady-state tick: {per_repo:?}/repo (S6 ceiling: {REGISTRY_WARM_CEILING_PER_REPO:?}/repo)"
    );
    assert_eq!(
        reread,
        REGISTRY_REPOS.div_ceil(BUSY_EVERY),
        "the mtime-prune gate reread a different set of repos than were actually \
         touched — the steady-state number above is not measuring what this test \
         says it is"
    );

    if cfg!(debug_assertions) {
        println!(
            "\ndebug profile: NOT asserting. A debug fold over {REGISTRY_REPOS} repos \
             measures the profile, not the code — run `mise run bench`."
        );
        return;
    }

    assert!(
        per_repo < REGISTRY_WARM_CEILING_PER_REPO,
        "steady-state tick {per_repo:?}/repo exceeds S6's {REGISTRY_WARM_CEILING_PER_REPO:?}/repo \
         (docs/spec.md#the-tick)"
    );
}
