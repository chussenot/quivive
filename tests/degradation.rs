//! quivive always produces a payload, or exits 1 saying why. It never panics
//! and it never exits non-zero over a condition that is not a fault.
//!
//! This is the file behind that promise. It matters more than it looks: the
//! consumer is a status bar, and a bar whose command panics shows a gap or a
//! stack trace where a fleet's health should be. Every input below is one a
//! real repository can present.
//!
//! S11's payload has no `degraded` field (unlike the old single-repo tile) —
//! see the doc comment on `quivive::tile::Payload` for why. What this file
//! calls "degraded" is therefore asserted at the READER level
//! (`reader::read(..).degraded`), which is the layer that still names it; the
//! payload-level assertions here check that a decline never turns into a
//! wrong count or a panic, which is the actual promise this file exists to
//! keep.

mod support;

use quivive::state::{RepoStatus, Thresholds};
use support::Fixture;

fn entry(f: &Fixture) -> quivive::tile::RepoEntry {
    quivive::tile::tick(f.root(), support::now(), &Thresholds::default(), true)
        .expect("a fixture repository is always readable")
}

#[test]
fn a_repository_with_no_pact_is_degraded_not_broken() {
    let f = Fixture::bare();
    assert_eq!(f.read(true).degraded, vec!["ledger"]);
    let e = entry(&f);
    assert_eq!(e.status, RepoStatus::NoFleet);
    assert_eq!(
        e.agents.active + e.agents.idle + e.agents.stale + e.agents.dead,
        0
    );
}

#[test]
fn an_empty_ledger_is_quiet_and_not_degraded() {
    let f = Fixture::new();
    assert!(
        f.read(true).degraded.is_empty(),
        "an empty ledger is a normal state"
    );
    assert_eq!(entry(&f).status, RepoStatus::AllQuiet);
}

#[test]
fn a_missing_leases_directory_is_not_degraded() {
    // Nobody holding a path is the resting state of a repository. A bar dimmed
    // for it would be dimmed most of the time.
    let f = Fixture::new();
    f.event("a", "acquired", 5);
    assert!(f.read(true).degraded.is_empty());
}

#[test]
fn garbage_lines_are_counted_and_the_rest_still_folds() {
    let f = Fixture::new();
    f.raw("{ not json");
    f.event("survivor", "acquired", 5);
    f.raw("\0\0\0 binary junk");
    // `f.read` first, and `entry` (which commits the cursor) second: `entry`
    // committing BEFORE `f.read` would let the second, cursor-resumed read see
    // nothing new to decline — 0 unparsable lines, not because they were
    // swallowed but because they were already consumed. Reading first checks
    // declines against the same cold pass `entry` would otherwise use.
    let degraded = f.read(true).degraded;
    assert!(
        degraded.iter().any(|d| d.contains("2 unparsable")),
        "declines must be counted, not swallowed: {degraded:?}"
    );
    assert_eq!(
        entry(&f).agents.active,
        1,
        "the good line must still be read"
    );
}

#[test]
fn a_garbage_lock_file_is_counted_and_the_others_still_read() {
    let f = Fixture::new();
    f.event("a", "acquired", 5);
    f.lease("a", "src/a.rs", 5, 900, false);
    let leases = f.root().join(".pact").join("leases");
    std::fs::write(leases.join("broken.lock"), "not json").unwrap();
    let readings = f.read(true);
    assert!(
        readings
            .degraded
            .iter()
            .any(|d| d.contains("1 unparsable lock")),
        "{:?}",
        readings.degraded
    );
    assert_eq!(
        readings
            .leases
            .iter()
            .map(|l| l.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/a.rs"],
        "the good lock must still be read"
    );
    f.commit(&readings);
}

#[test]
fn a_lock_with_an_absurd_ttl_does_not_panic() {
    // ttl_secs is a u64 read from a file on disk. Adding u64::MAX seconds to a
    // timestamp overflows, and a panic is the one failure mode a status bar
    // cannot survive.
    let f = Fixture::new();
    f.event("a", "acquired", 5);
    let leases = f.root().join(".pact").join("leases");
    std::fs::create_dir_all(&leases).unwrap();
    std::fs::write(
        leases.join("huge.lock"),
        format!(
            r#"{{"agent":"a","path":"src/a.rs","acquired_at":"{}","ttl_secs":{}}}"#,
            support::NOW,
            u64::MAX
        ),
    )
    .unwrap();
    let e = entry(&f);
    assert!(
        e.attention.is_empty(),
        "a lease held by a live agent is not blocking, however absurd its ttl"
    );
}

#[test]
fn an_escaping_lease_path_is_not_stat_d_into_evidence_of_life() {
    // A lease path is data from a file on disk. Joining an absolute path, or one
    // with `..` in it, onto the repository root would let a lock file point
    // quivive at anything on the filesystem. If it were (wrongly) followed, the
    // agent below would read as alive instead of dead.
    let f = Fixture::new();
    f.event("a", "acquired", 5000); // stale by the ledger alone
    f.lease("a", "../../../etc/passwd", 5000, 900, false);
    f.lease("a", "/etc/hostname", 5000, 900, false);
    assert_eq!(entry(&f).agents.dead, 1);
}

#[test]
fn a_ledger_that_is_a_directory_reads_as_absent() {
    // pact's own test suite creates exactly this shape. A directory opens
    // successfully and fails on read, which is a special case worth deciding once
    // rather than in the middle of the read loop.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".pact").join("events.jsonl")).unwrap();
    let readings = quivive::reader::read(&quivive::reader::Options {
        repo_root: dir.path().to_path_buf(),
        use_cursor: true,
    })
    .unwrap();
    assert_eq!(readings.degraded, vec!["ledger"]);
}

#[test]
fn a_repo_path_that_does_not_exist_is_the_one_real_error() {
    let err = quivive::reader::read(&quivive::reader::Options {
        repo_root: "/definitely/not/here".into(),
        use_cursor: true,
    });
    assert!(
        err.is_err(),
        "an unreadable --repo must exit 1, not degrade"
    );
    // `tile::tick` propagates the same error rather than swallowing it — it is
    // `tile::build`'s caller (the registry path in `tile::run`) that decides to
    // degrade a bad path instead of failing the whole payload; see the doc
    // comment on `tile::tick`.
    assert!(
        quivive::tile::tick(
            std::path::Path::new("/definitely/not/here"),
            support::now(),
            &Thresholds::default(),
            true,
        )
        .is_err()
    );
}

#[test]
fn an_unwritable_state_directory_does_not_fail_a_tick() {
    // The cursor is a cache. Not being able to write it must cost speed and
    // nothing else.
    let f = Fixture::new();
    f.event("a", "acquired", 5);
    let readings = f.read(true);
    // A directory that does not exist: `commit` must not create it, and must not
    // complain about not creating it.
    quivive::reader::commit(std::path::Path::new("/definitely/not/here"), &readings);
    assert_eq!(entry(&f).agents.active, 1);
}

#[test]
fn an_unknown_event_kind_counts_as_evidence() {
    // pact's schema keeps `kind` a plain string so an older reader shows an
    // unknown kind rather than refusing the line. quivive takes the same
    // direction deliberately: a new pact event kind starts working here the day
    // pact ships it, and the cost of being wrong is reporting an agent alive one
    // tick longer.
    let f = Fixture::new();
    f.event("a", "a-kind-from-the-future", 5);
    assert_eq!(entry(&f).agents.active, 1);
    assert!(
        f.read(true).degraded.is_empty(),
        "an unknown kind is not a decline"
    );
}

#[test]
fn evidence_in_the_future_reads_as_active_rather_than_dead() {
    // A clock that moved backwards is a real thing — pact keeps a
    // `clock_watermark` for exactly it. If the negative age were not clamped,
    // this agent would classify as long dead instead of freshly active.
    let f = Fixture::new();
    f.event("from-the-future", "acquired", -5000);
    assert_eq!(entry(&f).agents.active, 1);
}
