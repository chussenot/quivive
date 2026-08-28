//! vigil always produces a tile, or exits 1 saying why. It never panics and it
//! never exits non-zero over a condition that is not a fault.
//!
//! This is the file behind that promise. It matters more than it looks: the
//! consumer is a status bar, and a bar whose command panics shows a gap or a
//! stack trace where a fleet's health should be. Every input below is one a real
//! repository can present.

mod support;

use support::Fixture;
use vigil::state::Thresholds;
use vigil::tile::Tile;

fn tile_of(f: &Fixture) -> Tile {
    f.tile(true, &Thresholds::default())
}

#[test]
fn a_repository_with_no_pact_is_degraded_not_broken() {
    let t = tile_of(&Fixture::bare());
    assert_eq!(t.degraded, vec!["ledger"]);
    assert_eq!(t.fleet.total, 0);
    assert_eq!(t.worst, "quiet");
    // The text form must NOT report zeros here: `0A 0I 0S 0D` claims nothing is
    // running, and the truth is that vigil cannot see.
    assert_eq!(t.text(), "unreadable: ledger");
}

#[test]
fn an_empty_ledger_is_quiet_and_not_degraded() {
    let t = tile_of(&Fixture::new());
    assert!(t.degraded.is_empty(), "an empty ledger is a normal state");
    assert_eq!(t.worst, "quiet");
}

#[test]
fn a_missing_leases_directory_is_not_degraded() {
    // Nobody holding a path is the resting state of a repository. A bar dimmed
    // for it would be dimmed most of the time.
    let f = Fixture::new();
    f.event("a", "acquired", 5);
    assert!(tile_of(&f).degraded.is_empty());
}

#[test]
fn garbage_lines_are_counted_and_the_rest_still_folds() {
    let f = Fixture::new();
    f.raw("{ not json");
    f.event("survivor", "acquired", 5);
    f.raw("\0\0\0 binary junk");
    let t = tile_of(&f);
    assert_eq!(t.fleet.total, 1, "the good line must still be read");
    assert!(
        t.degraded.iter().any(|d| d.contains("2 unparsable")),
        "declines must be counted, not swallowed: {:?}",
        t.degraded
    );
}

#[test]
fn a_garbage_lock_file_is_counted_and_the_others_still_read() {
    let f = Fixture::new();
    f.event("a", "acquired", 5);
    f.lease("a", "src/a.rs", 5, 900, false);
    let leases = f.root().join(".pact").join("leases");
    std::fs::write(leases.join("broken.lock"), "not json").unwrap();
    let t = tile_of(&f);
    assert!(
        t.degraded.iter().any(|d| d.contains("1 unparsable lock")),
        "{:?}",
        t.degraded
    );
    assert_eq!(t.agents[0].leases, vec!["src/a.rs"]);
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
    let t = tile_of(&f);
    assert!(
        t.blocked_leases.is_empty(),
        "a lease that never expires under a live agent is not blocking"
    );
}

#[test]
fn a_lease_path_escaping_the_repository_is_ignored_by_the_worktree_reader() {
    // A lease path is data from a file on disk. Joining an absolute path, or one
    // with `..` in it, onto the repository root would let a lock file point vigil
    // at anything on the filesystem.
    let f = Fixture::new();
    f.event("a", "acquired", 5000); // stale by the ledger alone
    f.lease("a", "../../../etc/passwd", 5000, 900, false);
    f.lease("a", "/etc/hostname", 5000, 900, false);
    let t = tile_of(&f);
    assert_eq!(
        t.agents[0].state.as_str(),
        "dead",
        "an escaping path must not be stat'd into evidence of life"
    );
}

#[test]
fn a_ledger_that_is_a_directory_reads_as_absent() {
    // pact's own test suite creates exactly this shape. A directory opens
    // successfully and fails on read, which is a special case worth deciding once
    // rather than in the middle of the read loop.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".pact").join("events.jsonl")).unwrap();
    let readings = vigil::reader::read(&vigil::reader::Options {
        repo_root: dir.path().to_path_buf(),
        use_cursor: true,
    })
    .unwrap();
    assert_eq!(readings.degraded, vec!["ledger"]);
}

#[test]
fn a_repo_path_that_does_not_exist_is_the_one_real_error() {
    let err = vigil::reader::read(&vigil::reader::Options {
        repo_root: "/definitely/not/here".into(),
        use_cursor: true,
    });
    assert!(
        err.is_err(),
        "an unreadable --repo must exit 1, not degrade"
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
    vigil::reader::commit(std::path::Path::new("/definitely/not/here"), &readings);
    assert_eq!(tile_of(&f).fleet.total, 1);
}

#[test]
fn an_unknown_event_kind_counts_as_evidence() {
    // pact's schema keeps `kind` a plain string so an older reader shows an
    // unknown kind rather than refusing the line. vigil takes the same direction
    // deliberately: a new pact event kind starts working here the day pact ships
    // it, and the cost of being wrong is reporting an agent alive one tick longer.
    let f = Fixture::new();
    f.event("a", "a-kind-from-the-future", 5);
    let t = tile_of(&f);
    assert_eq!(t.fleet.active, 1);
    assert!(t.degraded.is_empty(), "an unknown kind is not a decline");
}

#[test]
fn evidence_in_the_future_reads_as_active_rather_than_as_a_negative_age() {
    // A clock that moved backwards is a real thing — pact keeps a
    // `clock_watermark` for exactly it — and `-3s` on a status bar is a bug
    // report from a user.
    let f = Fixture::new();
    f.event("from-the-future", "acquired", -5000);
    let t = tile_of(&f);
    assert_eq!(t.agents[0].age_s, 0);
    assert_eq!(t.agents[0].state.as_str(), "active");
}
