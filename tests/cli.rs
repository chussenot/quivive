//! The binary's own surface: exit codes, the two output forms, and the promise
//! that a frozen clock makes two invocations byte-identical.
//!
//! These spawn the real binary rather than calling the library, because three of
//! the things being asserted are properties of the *process* — its exit code, its
//! stdout, and the fact that `QUIVIVE_NOW` reaches it — and a library-level test
//! of any of them would assert something adjacent to the truth.

mod support;

use std::process::{Command, Output};

use support::Fixture;

fn quivive(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quivive"))
        .args(args)
        .env("QUIVIVE_NOW", support::NOW)
        // Unset, or a developer with pact's state redirected sees every fixture
        // read as an absent ledger.
        .env_remove("PACT_STATE_DIR")
        .output()
        .expect("the binary under test must run")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn fleet() -> Fixture {
    let f = Fixture::new();
    f.event("agent-1", "acquired", 5);
    f.event("agent-2", "acquired", 412);
    f.lease("agent-2", "src/fold.rs", 412, 300, false);
    f
}

#[test]
fn a_tile_is_one_line_and_exits_zero() {
    let f = fleet();
    let o = quivive(&["tile", "--repo", f.root().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    let out = stdout(&o);
    assert_eq!(out.lines().count(), 1, "a tile is one line: {out:?}");
    assert!(out.contains("worst=stale"), "{out}");
}

#[test]
fn a_quiet_repository_still_exits_zero() {
    // A bar must not go dark over the resting state of a repository.
    let f = Fixture::new();
    let o = quivive(&["tile", "--repo", f.root().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(stdout(&o).trim(), "0A 0I 0S 0D  worst=quiet");
}

#[test]
fn a_repository_with_no_pact_exits_zero_and_says_it_cannot_see() {
    let f = Fixture::bare();
    let o = quivive(&["tile", "--repo", f.root().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "degraded is not an error");
    assert_eq!(stdout(&o).trim(), "unreadable: ledger");
}

#[test]
fn json_is_the_contract_and_parses() {
    let f = fleet();
    let o = quivive(&["tile", "--json", "--repo", f.root().to_str().unwrap()]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).expect("--json must be JSON");
    assert_eq!(v["v"], 1);
    assert_eq!(v["at"], support::NOW);
    // Every field docs/tile-contract.md names, present by name. A field renamed
    // in the struct and not in the doc fails here rather than in a consumer.
    for key in [
        "v",
        "at",
        "repo",
        "fleet",
        "worst",
        "agents",
        "blocked_leases",
        "degraded",
    ] {
        assert!(!v[key].is_null(), "the contract names `{key}`");
    }
    for key in ["active", "idle", "stale", "dead", "total"] {
        assert!(
            !v["fleet"][key].is_null(),
            "fleet always carries all five counts, zeros included"
        );
    }
}

#[test]
fn exit_on_returns_two_when_the_tile_reaches_that_state() {
    let f = fleet(); // worst is STALE
    let root = f.root().to_str().unwrap().to_string();
    for (threshold, expected) in [("active", 2), ("idle", 2), ("stale", 2), ("dead", 0)] {
        let o = quivive(&["tile", "--repo", &root, "--exit-on", threshold]);
        assert_eq!(
            o.status.code(),
            Some(expected),
            "--exit-on {threshold} on a stale fleet"
        );
        assert_eq!(
            stdout(&o).lines().count(),
            1,
            "--exit-on still prints the tile"
        );
    }
}

#[test]
fn a_usage_error_exits_one_and_not_claps_default_of_two() {
    // docs/tile-contract.md reserves 2 for --exit-on. A documented exit code the
    // binary does not use is worse than no documentation.
    let o = quivive(&["tile", "--no-such-flag"]);
    assert_eq!(o.status.code(), Some(1));
    let o = quivive(&["tile", "--active-window", "not-a-duration"]);
    assert_eq!(o.status.code(), Some(1));
    let o = quivive(&["tile", "--repo", "/definitely/not/here"]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn windows_out_of_order_are_refused_rather_than_producing_an_impossible_tile() {
    let f = fleet();
    let o = quivive(&[
        "tile",
        "--repo",
        f.root().to_str().unwrap(),
        "--idle-window",
        "10s",
        "--active-window",
        "60s",
    ]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("must be shorter than"),
        "the error must name the two flags"
    );
}

#[test]
fn help_and_version_exit_zero() {
    for args in [vec!["--help"], vec!["--version"], vec!["tile", "--help"]] {
        let o = quivive(&args);
        assert_eq!(o.status.code(), Some(0), "{args:?}");
        assert!(!stdout(&o).is_empty(), "{args:?} printed nothing");
    }
}

#[test]
fn help_enumerates_the_exit_on_states_from_the_enum_itself() {
    // Rendered by clap from `State`, so --help cannot offer a state the parser
    // rejects or omit one it accepts. This asserts the wiring, not the list.
    let help = stdout(&quivive(&["tile", "--help"]));
    for state in ["active", "idle", "stale", "dead"] {
        assert!(
            help.contains(state),
            "`{state}` missing from --help:\n{help}"
        );
    }
}

#[test]
fn help_renders_every_window_default_from_its_single_source() {
    // The four defaults are string consts in `state`, rendered into --help by clap
    // and parsed by `Thresholds::default()`. This asserts the wiring: if somebody
    // reintroduces a literal in `cli.rs`, the const and the help text can diverge
    // and this is what notices. The first draft of this crate had exactly that
    // drift, in the same session as the role file describing it.
    let help = stdout(&quivive(&["tile", "--help"]));
    for d in [
        quivive::state::ACTIVE_DEFAULT,
        quivive::state::IDLE_DEFAULT,
        quivive::state::DEAD_DEFAULT,
        quivive::state::FORGET_DEFAULT,
    ] {
        assert!(
            help.contains(&format!("[default: {d}]")),
            "--help does not render `{d}` as a default:\n{help}"
        );
    }
}

#[test]
fn a_frozen_clock_makes_two_invocations_byte_identical() {
    // The purity claim, asserted on the process rather than the library: a warm
    // read, a cold read and a forced cold read must all agree.
    // docs/adr/0001-stream-first-tile.md.
    let f = fleet();
    let root = f.root().to_str().unwrap().to_string();
    let cold = stdout(&quivive(&["tile", "--json", "--repo", &root]));
    let warm = stdout(&quivive(&["tile", "--json", "--repo", &root]));
    let forced = stdout(&quivive(&[
        "tile",
        "--json",
        "--repo",
        &root,
        "--no-cursor",
    ]));
    assert_eq!(cold, warm, "resuming changed the tile");
    assert_eq!(warm, forced, "--no-cursor changed the tile");
}

#[test]
fn no_cursor_leaves_no_cursor_behind() {
    let f = Fixture::new();
    f.event("a", "acquired", 5);
    let o = quivive(&["tile", "--repo", f.root().to_str().unwrap(), "--no-cursor"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(
        !f.has_cursor(),
        "--no-cursor must not write the file it was told to ignore"
    );
}

#[test]
fn a_tile_over_a_repository_with_no_pact_leaves_no_trace() {
    // Creating `.pact/` would be quivive initialising somebody else's tool in a
    // repository it had nothing to say about.
    let f = Fixture::bare();
    let _ = quivive(&["tile", "--repo", f.root().to_str().unwrap()]);
    assert!(!f.root().join(".pact").exists());
}

#[test]
fn watch_is_not_implemented_yet() {
    // docs/spec.md S22: the whole CLI is tile, watch, why. `watch` (S14-S20,
    // notify-send on transitions) lands in a later bead; until then it is a
    // real subcommand that fails loudly rather than silently doing nothing.
    let o = quivive(&["watch"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("not implemented yet"),
        "{o:?}"
    );
}

#[test]
fn why_is_not_implemented_yet() {
    // docs/spec.md S21. Lands in a later bead; until then it is a real
    // subcommand that fails loudly rather than silently doing nothing.
    let f = fleet();
    let o = quivive(&["why", f.root().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("not implemented yet"),
        "{o:?}"
    );
}

#[test]
fn tile_stream_is_not_implemented_yet() {
    // docs/spec.md S9. Lands in a later bead; until then it is a real flag that
    // fails loudly rather than silently doing nothing.
    let f = fleet();
    let o = quivive(&["tile", "--repo", f.root().to_str().unwrap(), "--stream"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("not implemented yet"),
        "{o:?}"
    );
}

#[test]
fn a_bad_quivive_now_is_an_error_rather_than_a_silent_fallback_to_the_wall_clock() {
    // A test seam that silently ignores a malformed value would make every
    // golden-style comparison pass for the wrong reason — which is the exact
    // mistake docs/studies/conventions-run.md records twice.
    let f = fleet();
    let o = Command::new(env!("CARGO_BIN_EXE_quivive"))
        .args(["tile", "--repo", f.root().to_str().unwrap()])
        .env("QUIVIVE_NOW", "yesterday afternoon")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("QUIVIVE_NOW"));
}
