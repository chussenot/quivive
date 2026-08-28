//! The tile contract, pinned.
//!
//! `mise run tile-goldens` runs exactly this file. A diff here is a **question,
//! not a failure**, and it has two possible answers with opposite fixes:
//!
//! 1. The contract changed → regenerate (`UPDATE_GOLDENS=1`), update
//!    `docs/tile-contract.md`, and decide whether `v` moves. Additive changes do
//!    not move it; a removal, a rename, a type change, a meaning change or a
//!    change to severity order does.
//! 2. The fold broke → the goldens are right and the code is wrong.
//!
//! Establish which before touching anything. Regenerating a golden to turn a red
//! gate green is how a breaking change ships without anybody deciding to ship it.
//!
//! Every golden is computed against the frozen clock in `support::NOW`, which is
//! what makes a payload a fixed string at all. A golden that needed a `sleep`, a
//! real clock or a retry would be evidence that purity has been lost.
//!
//! **This is a starting base, not the final set.** `quivive-jx3` rewrites these
//! against the five S13 samples (`all-quiet`, `active`, `human-needed`,
//! `drained`, `no-fleet`) in both repos. The scenarios here exercise the same
//! ground from the reader/fold side — declines, the forget sweep, the ledger's
//! purity invariant — and are kept intentionally small so that bead has a clean
//! base to build on rather than a second, competing set of samples to reconcile.
//!
//! **Path normalization**: a [`quivive::tile::RepoEntry`]'s `name` and `path`
//! come from the fixture's own tempdir, which is different on every run — so
//! [`assert_golden`] replaces the fixture's real path (and its basename) with
//! fixed placeholders before comparing. Nothing else in the payload is ever
//! rewritten.

mod support;

use std::path::{Path, PathBuf};

use quivive::state::Thresholds;
use quivive::tile::Payload;
use support::Fixture;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens")
}

/// Replace this fixture's volatile tempdir path (and its basename) with fixed
/// placeholders, longest string first so the basename substitution cannot
/// corrupt a path substitution already made.
fn normalize(json: &str, root: &Path) -> String {
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let path = canon.display().to_string();
    let name = canon
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = json.replace(&path, "REPO_PATH");
    if !name.is_empty() {
        out = out.replace(&name, "REPO_NAME");
    }
    out
}

/// Compare, or rewrite when `UPDATE_GOLDENS=1`.
fn assert_golden(name: &str, root: &Path, payload: &Payload) {
    let json = normalize(&serde_json::to_string_pretty(payload).unwrap(), root) + "\n";
    let path = golden_dir().join(format!("{name}.json"));
    if std::env::var("UPDATE_GOLDENS").is_ok() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, &json).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "no golden at {}. If this scenario is new, run:\n    \
             UPDATE_GOLDENS=1 cargo test --test goldens\n\
             then read the diff before committing it.",
            path.display()
        )
    });
    assert_eq!(
        expected, json,
        "golden {name}.json differs.\n\n\
         This is a question, not a failure: did the CONTRACT change (regenerate, \
         update docs/tile-contract.md, decide about `v`) or did the FOLD break \
         (the golden is right)?\n"
    );
}

/// Two active, one idle, one stale (holding a live lease), one dead (holding
/// nothing) — a working fleet with nothing needing a human, because S16 only
/// fires for a DEAD holder and this dead agent holds no lease.
fn active_fleet() -> Fixture {
    let f = Fixture::new();
    f.event("agent-1", "acquired", 5); // ACTIVE
    f.event("agent-2", "renewed", 30); // ACTIVE
    f.event("agent-3", "acquired", 120); // IDLE, and holds a live lease
    f.event("agent-4", "acquired", 412); // STALE
    f.event("agent-6", "acquired", 2400); // DEAD, holds nothing
    f.lease("agent-3", "src/tile.rs", 120, 900, false);
    f
}

#[test]
fn active_fleet_has_no_attention_items() {
    let f = active_fleet();
    let payload = f.tile(true, &Thresholds::default());
    assert_eq!(payload.status, quivive::state::RepoStatus::Active);
    assert!(payload.repos[0].attention.is_empty());
    assert_golden("active_fleet", f.root(), &payload);
}

#[test]
fn all_quiet_repository() {
    // A pact-initialised repository nobody has worked in.
    let f = Fixture::new();
    let payload = f.tile(true, &Thresholds::default());
    assert_eq!(payload.status, quivive::state::RepoStatus::AllQuiet);
    assert_golden("all_quiet", f.root(), &payload);
}

#[test]
fn no_fleet_bare() {
    // No `.pact/` at all — a normal repository, not a broken one.
    let f = Fixture::bare();
    let payload = f.tile(true, &Thresholds::default());
    assert_eq!(payload.status, quivive::state::RepoStatus::NoFleet);
    assert_golden("no_fleet_bare", f.root(), &payload);
}

#[test]
fn human_needed_dead_holder() {
    // S16, verbatim: a DEAD agent holds paths. This is the one case a `STALE`
    // holder does NOT produce (see `active_fleet` above) — S16 names DEAD only.
    let f = Fixture::new();
    f.event("agent-1", "acquired", 5); // ACTIVE, for contrast
    f.event("ghost", "acquired", 2400); // DEAD
    f.lease("ghost", "src/fold.rs", 2400, 300, false);
    let payload = f.tile(true, &Thresholds::default());
    assert_eq!(payload.status, quivive::state::RepoStatus::HumanNeeded);
    assert_eq!(payload.repos[0].attention.len(), 1);
    assert_golden("human_needed_dead_holder", f.root(), &payload);
}

#[test]
fn drained_fleet() {
    // A plan exists, and once had a live agent, but none remains: fleet
    // evidence without anybody currently working — S8's `drained`.
    let f = Fixture::new();
    f.event("done-agent", "released", 100_000); // long dead, holds nothing
    f.plan(&[("proj-1", &[])], &[("proj-1", 0)], &[]);
    let payload = f.tile(true, &Thresholds::default());
    assert_eq!(payload.status, quivive::state::RepoStatus::Drained);
    assert_golden("drained_fleet", f.root(), &payload);
}

#[test]
fn damaged_lines_are_counted_not_fatal() {
    let f = Fixture::new();
    f.event("agent-1", "acquired", 10);
    f.raw("this is not json");
    f.raw(r#"{"agent":"agent-2","kind":"acquired"}"#); // no `at`
    f.raw(r#"{"at":"not a timestamp","agent":"a","kind":"acquired"}"#);
    f.raw(""); // a blank line is NOT damage; pact's rewrite can leave one
    f.lease_staging_file("half-written.tmp"); // nor is pact's staging sibling
    let payload = f.tile(true, &Thresholds::default());
    assert_eq!(
        payload.repos[0].agents.active, 1,
        "the good line still folds"
    );
    assert_golden("declines", f.root(), &payload);
}

#[test]
fn an_expired_row_does_not_resurrect_its_agent() {
    // pact writes `expired` under the name of the holder whose claim ENDED —
    // the sweeper wrote the row, and the named agent by definition did
    // nothing. Same for `displaced`. Counting either as evidence would
    // resurrect exactly the agent that just went quiet, which is the most
    // misleading thing quivive could say. This golden is that rule.
    let f = Fixture::new();
    f.event("ghost", "acquired", 3000);
    f.event("ghost", "expired", 1); // one second ago, and must not count
    f.event("ghost-2", "acquired", 3000);
    f.event("ghost-2", "displaced", 1); // likewise
    f.event("live", "stolen", 1); // the agent that DID act: counts
    let payload = f.tile(true, &Thresholds::default());
    assert_eq!(payload.repos[0].agents.active, 1, "only `live` counts");
    assert_golden("expired_kinds", f.root(), &payload);
}

#[test]
fn forgetting_spares_an_agent_that_is_blocking_a_lease() {
    // The sweep exists so a week-old repository does not carry forty dead
    // names in its counts forever. But an agent whose expired claim is
    // blocking somebody is the single most actionable thing in the tile, so
    // it is never swept.
    let f = Fixture::new();
    f.event("long-gone", "released", 100_000);
    f.event("long-gone-but-holding", "acquired", 100_000);
    f.lease("long-gone-but-holding", "src/state.rs", 100_000, 900, false);
    let payload = f.tile(true, &Thresholds::default());
    // `long-gone` is forgotten (quiet past `forget`, holds nothing);
    // `long-gone-but-holding` survives as the fleet's one DEAD count and
    // produces the attention item S16 asks for.
    assert_eq!(payload.repos[0].agents.dead, 1);
    assert_eq!(payload.status, quivive::state::RepoStatus::HumanNeeded);
    assert_golden("forget", f.root(), &payload);
}

// ---------------------------------------------------------------------------
// The invariant. Not a golden — a property, and the load-bearing one.
// ---------------------------------------------------------------------------

/// **Deleting the cursor and re-reading must produce a byte-identical
/// payload.**
///
/// `docs/adr/0001-stream-first-tile.md`. Everything else in the crate is in
/// service of this sentence, and the reason it is asserted over every scenario
/// rather than one is that the interesting failures are scenario-specific:
/// declines, blank lines, and leases all touch the fold differently.
#[test]
fn a_cursor_is_correct_to_throw_away() {
    let thresholds = Thresholds::default();
    let scenarios: Vec<(&str, Fixture)> = vec![
        ("active_fleet", active_fleet()),
        ("all_quiet", Fixture::new()),
        ("no_fleet_bare", Fixture::bare()),
    ];
    for (name, f) in scenarios {
        let cold = serde_json::to_string_pretty(&f.tile(true, &thresholds)).unwrap();
        assert!(
            f.is_bare() || f.has_cursor(),
            "{name}: a tick over a real ledger must leave a cursor"
        );
        let warm = serde_json::to_string_pretty(&f.tile(true, &thresholds)).unwrap();
        f.delete_cursor();
        let recold = serde_json::to_string_pretty(&f.tile(true, &thresholds)).unwrap();
        let forced = serde_json::to_string_pretty(&f.tile(false, &thresholds)).unwrap();
        assert_eq!(cold, warm, "{name}: resuming changed the tile");
        assert_eq!(warm, recold, "{name}: deleting the cursor changed the tile");
        assert_eq!(warm, forced, "{name}: --no-cursor changed the tile");
    }
}

#[test]
fn a_tick_landing_mid_append_does_not_consume_the_half_line() {
    // The single most likely bug in the crate. The ledger is written by another
    // process, so a tick can see half a line; advancing the cursor over it would
    // drop the rest of that event forever, and the tile would be wrong until the
    // next rewrite.
    let thresholds = Thresholds::default();
    let f = Fixture::new();
    f.event("agent-1", "acquired", 10);
    f.partial(r#"{"at":"2026-08-28T08:59:59Z","agent":"agent-2","kind":"acq"#);

    let mid = f.entry(true, &thresholds);
    assert_eq!(mid.agents.active, 1, "the half line must not be counted");

    // Now the writer finishes the line. The next tick must see agent-2.
    f.raw(r#"uired","path":"src/x.rs"}"#);
    let after = f.entry(true, &thresholds);
    assert_eq!(
        after.agents.active, 2,
        "the completed line must be picked up by the resumed read, not skipped"
    );
    assert_eq!(
        serde_json::to_string(&after).unwrap(),
        serde_json::to_string(&f.entry(false, &thresholds)).unwrap(),
        "a resumed read across a completed partial line must match a cold one"
    );
}

#[test]
fn a_rewritten_ledger_falls_back_to_a_full_re_read() {
    // pact rewrites events.jsonl down to its newest 4000 lines once it passes
    // 5000, so this is routine, not exotic. The case a bare byte offset cannot
    // catch is a rewrite that then grows back PAST the old offset: the length
    // check passes and resuming there silently skips every event since.
    let thresholds = Thresholds::default();
    let f = Fixture::new();
    for i in 0..40 {
        f.event(&format!("old-{i}"), "acquired", 10);
    }
    let before = f.entry(true, &thresholds);
    assert_eq!(before.agents.active, 40);

    // Rewrite to a SHORTER set, then grow it back past the old offset.
    f.rewrite(&[r#"{"at":"2026-08-28T08:59:55Z","agent":"kept","kind":"acquired"}"#]);
    for i in 0..60 {
        f.event(&format!("new-{i}"), "acquired", 10);
    }

    // Assert the *mechanism*, not only the outcome: this read must have been a
    // full re-read. Without this the test would still pass if resuming happened
    // to land on the same answer, which is how soak draft one in
    // docs/studies/conventions-run.md proved nothing.
    assert!(
        f.read(true).cold,
        "a rewritten ledger must force a cold re-read"
    );

    let after = f.entry(true, &thresholds);
    let cold = f.entry(false, &thresholds);
    assert_eq!(
        serde_json::to_string(&after).unwrap(),
        serde_json::to_string(&cold).unwrap(),
        "a rewrite that grew back past the cursor must force a cold re-read"
    );
    assert_eq!(after.agents.active, 61, "kept + 60 new");
}

#[test]
fn an_idle_tick_keeps_the_cursor_usable() {
    // A tick that consumes nothing must carry the prior tail forward, not drop
    // it. Dropping it left the next tick unable to verify its resume point, so
    // every tick after the first quiet one degraded to a full re-read: a cursor
    // that silently stops working exactly when the fleet is calm.
    let thresholds = Thresholds::default();
    let f = Fixture::new();
    f.event("agent-1", "acquired", 10);
    let _ = f.entry(true, &thresholds);
    let first = quivive::cursor::load(&f.root().join(".pact")).unwrap();
    assert!(first.tail_len > 0);

    assert!(
        !f.read(true).cold,
        "a tick with a valid cursor must not be a cold read"
    );
    let _ = f.entry(true, &thresholds); // nothing appended
    let second = quivive::cursor::load(&f.root().join(".pact")).unwrap();
    assert_eq!(second.offset, first.offset);
    assert_eq!(
        second.tail_len, first.tail_len,
        "a quiet tick must not forget the tail it can still verify"
    );
    assert_eq!(second.tail_hash, first.tail_hash);
}
