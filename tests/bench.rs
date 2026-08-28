//! The two ceilings `docs/spec.md#the-tick` sets, measured rather than asserted
//! in prose.
//!
//! `#[ignore]` and release-only, run by `mise run bench`. Deliberately NOT part
//! of `mise run check`: it writes a large synthetic ledger and takes time, and a
//! required gate with those properties is one people switch off.
//!
//! It refuses to assert under `debug_assertions`, because a debug fold over
//! 100,000 events measures the profile rather than the code — and a ceiling that
//! fails for the profile is a ceiling somebody deletes.

mod support;

use std::io::Write;
use std::time::{Duration, Instant};

use chrono::TimeDelta;
use vigil::reader::{self, Options};
use vigil::state::Thresholds;
use vigil::tile::Tile;

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
    let readings = reader::read(&Options {
        repo_root: root.to_path_buf(),
        use_cursor,
    })
    .unwrap();
    let _ = Tile::build(&readings, "bench", support::now(), &Thresholds::default());
    reader::commit(root, &readings);
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
