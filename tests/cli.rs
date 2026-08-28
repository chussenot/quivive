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

/// One ACTIVE agent, one STALE agent holding a live lease. Overall status is
/// `active` (S8: any live agent wins), because S16's `human-needed` only fires
/// for a DEAD holder — a STALE one does not qualify.
fn fleet() -> Fixture {
    let f = Fixture::new();
    f.event("agent-1", "acquired", 5);
    f.event("agent-2", "acquired", 412);
    f.lease("agent-2", "src/fold.rs", 412, 900, false);
    f
}

/// One DEAD agent holding a lease: `human-needed`, S16.
fn fleet_needing_a_human() -> Fixture {
    let f = Fixture::new();
    f.event("ghost", "acquired", 2400);
    f.lease("ghost", "src/fold.rs", 2400, 300, false);
    f
}

#[test]
fn the_default_output_is_the_json_payload() {
    // S11: the payload IS one JSON object. No flag is needed to ask for it.
    let f = fleet();
    let o = quivive(&["tile", "--repo", f.root().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).expect("default output is JSON");
    assert_eq!(v["v"], 1);
    assert_eq!(v["at"], support::NOW);
    assert_eq!(v["status"], "active");
    let repo = &v["repos"][0];
    for key in ["name", "path", "status", "agents", "attention"] {
        assert!(!repo[key].is_null(), "repos[0] must carry `{key}`");
    }
    for key in ["active", "idle", "stale", "dead"] {
        assert!(
            !repo["agents"][key].is_null(),
            "agents always carries all four counts, zeros included"
        );
    }
}

#[test]
fn text_is_one_line_and_summarizes_by_status() {
    let f = fleet();
    let o = quivive(&["tile", "--repo", f.root().to_str().unwrap(), "--text"]);
    assert_eq!(o.status.code(), Some(0));
    let out = stdout(&o);
    assert_eq!(out.lines().count(), 1, "--text is one line: {out:?}");
    assert_eq!(out.trim(), "active  1 repo: 1 active");
}

#[test]
fn a_quiet_repository_still_exits_zero() {
    // A bar must not go dark over the resting state of a repository.
    let f = Fixture::new();
    let o = quivive(&["tile", "--repo", f.root().to_str().unwrap(), "--text"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(stdout(&o).trim(), "all-quiet  1 repo: 1 all-quiet");
}

#[test]
fn a_repository_with_no_pact_exits_zero() {
    let f = Fixture::bare();
    let o = quivive(&["tile", "--repo", f.root().to_str().unwrap(), "--text"]);
    assert_eq!(o.status.code(), Some(0), "degraded is not an error");
    assert_eq!(stdout(&o).trim(), "no-fleet  1 repo: 1 no-fleet");
}

#[test]
fn exit_on_returns_two_when_the_overall_status_meets_or_exceeds_it() {
    let f = fleet(); // overall status is `active`
    let root = f.root().to_str().unwrap().to_string();
    for (threshold, expected) in [
        ("human-needed", 0), // active does not reach human-needed
        ("active", 2),
        ("drained", 2),
        ("all-quiet", 2),
        ("no-fleet", 2),
    ] {
        let o = quivive(&["tile", "--repo", &root, "--exit-on", threshold]);
        assert_eq!(
            o.status.code(),
            Some(expected),
            "--exit-on {threshold} on an active fleet"
        );
    }
}

#[test]
fn exit_on_human_needed_fires_for_a_dead_holder() {
    let f = fleet_needing_a_human();
    let o = quivive(&[
        "tile",
        "--repo",
        f.root().to_str().unwrap(),
        "--exit-on",
        "human-needed",
    ]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_usage_error_exits_one_and_not_claps_default_of_two() {
    // docs/tile-contract.md reserves 2 for --exit-on. A documented exit code the
    // binary does not use is worse than no documentation.
    let o = quivive(&["tile", "--no-such-flag"]);
    assert_eq!(o.status.code(), Some(1));
    let o = quivive(&["tile", "--active-window", "not-a-duration"]);
    assert_eq!(o.status.code(), Some(1));
    // An explicit --repo that does not exist is the one real error: it is not
    // a registry entry, so there is nothing to degrade around.
    let o = quivive(&["tile", "--repo", "/definitely/not/here"]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn windows_out_of_order_are_refused_rather_than_producing_an_impossible_payload() {
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
fn help_enumerates_the_exit_on_statuses_from_the_enum_itself() {
    // Rendered by clap from `RepoStatus` (S8's five), so --help cannot offer a
    // status the parser rejects or omit one it accepts. This asserts the
    // wiring, not the list.
    let help = stdout(&quivive(&["tile", "--help"]));
    for status in ["human-needed", "active", "drained", "all-quiet", "no-fleet"] {
        assert!(
            help.contains(status),
            "`{status}` missing from --help:\n{help}"
        );
    }
}

#[test]
fn help_renders_every_window_default_from_its_single_source() {
    // The four defaults are string consts in `state`, rendered into --help by clap
    // and parsed by `Thresholds::default()`. This asserts the wiring: if somebody
    // reintroduces a literal in `cli.rs`, the const and the help text can diverge
    // and this is what notices.
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
    // read, a cold read and a forced cold read must all agree. JSON is already
    // the default output, so no `--json` flag is needed to compare it.
    // docs/adr/0001-stream-first-tile.md.
    let f = fleet();
    let root = f.root().to_str().unwrap().to_string();
    let cold = stdout(&quivive(&["tile", "--repo", &root]));
    let warm = stdout(&quivive(&["tile", "--repo", &root]));
    let forced = stdout(&quivive(&["tile", "--repo", &root, "--no-cursor"]));
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
fn a_registry_less_machine_reports_no_fleet() {
    // The acceptance line quivive-eea quotes verbatim: over a no-registry
    // machine, `quivive tile` exits 0 with status no-fleet. Pointing
    // `XDG_CONFIG_HOME` at an empty tempdir is a machine with no registry file
    // at all — S2's "missing file is an empty registry, not an error".
    let empty_config = tempfile::tempdir().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_quivive"))
        .args(["tile"])
        .env("QUIVIVE_NOW", support::NOW)
        .env("XDG_CONFIG_HOME", empty_config.path())
        .env_remove("PACT_STATE_DIR")
        .output()
        .expect("the binary under test must run");
    assert_eq!(o.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["status"], "no-fleet");
    assert_eq!(v["repos"].as_array().unwrap().len(), 0);
}

#[test]
fn watch_starts_and_does_not_hang_the_suite() {
    // docs/spec.md S22: the whole CLI is tile, watch, why. `watch` (S14-S20,
    // notify-send on transitions) lands in quivive-8mq; until then, in THIS
    // branch, it is a real subcommand that fails loudly rather than silently
    // doing nothing.
    //
    // quivive-8mq landed on a sibling branch (agent/watch, commit 10e8815) this
    // worktree cannot see: src/watch.rs here is still the pre-quivive-8mq stub.
    // Once that branch merges, `quivive watch` becomes a real,
    // intentionally-infinite loop until interrupted — and `.output()`, used
    // everywhere else in this file, blocks until the child exits. A `watch`
    // that never exits on its own would hang this suite forever the moment
    // that lands, not just go red. Spawning with a BOUNDED wait instead is what
    // keeps this test meaningful in both states: it accepts either "exited
    // within the bound" (this branch's stub, checked below) or "still running
    // after the bound" (a real watch — success, not a hang) and never blocks
    // longer than the bound either way.
    let mut child = Command::new(env!("CARGO_BIN_EXE_quivive"))
        .arg("watch")
        .env("QUIVIVE_NOW", support::NOW)
        .env_remove("PACT_STATE_DIR")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the binary under test must run");

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    let exited = loop {
        if let Some(status) = child.try_wait().expect("polling the child must not fail") {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };

    match exited {
        Some(status) => {
            // THIS branch's stub: exits immediately, loudly, rather than
            // silently doing nothing.
            assert_eq!(status.code(), Some(1));
            let mut stderr = String::new();
            use std::io::Read;
            child
                .stderr
                .take()
                .expect("stderr was piped")
                .read_to_string(&mut stderr)
                .unwrap();
            assert!(stderr.contains("not implemented yet"), "{stderr}");
        }
        None => {
            // A real, running `watch` (post-quivive-8mq-merge): still alive
            // past the bound is the success case here, not a hang — clean it
            // up rather than asserting anything about its stub-era message.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn why_lists_the_attention_items_for_one_repo() {
    // docs/spec.md S21. quivive-15r landed `quivive::why`: exit 0, a real
    // answer on stdout, not the old "not implemented yet" refusal. This is
    // deliberately loose about the exact shape of that answer — S21 and its
    // own contract page are `quivive-15r`'s to pin, not this bead's — and
    // only asserts the two things `tile`'s own contract already guarantees
    // about the fixture: it ran to completion, and it did not fall back to
    // the stub's refusal message.
    let f = fleet_needing_a_human();
    let o = quivive(&["why", f.root().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert!(
        !String::from_utf8_lossy(&o.stderr).contains("not implemented yet"),
        "{o:?}"
    );
}

#[test]
fn tile_stream_emits_one_line_immediately_and_stays_alive() {
    // docs/spec.md S9, the pwetty push contract, driven through the real
    // process: `--stream` must actually be a long-lived subcommand that
    // emits and does not exit, which is a property of the binary's process
    // lifecycle, not of `quivive::stream::run` in isolation (unit-tested
    // directly in `src/stream.rs`).
    //
    // Bounded rather than blocking: the read runs on its own thread so a
    // stream that never produces a line fails this test after
    // `READ_DEADLINE` instead of hanging the suite.
    let f = fleet();
    let mut child = Command::new(env!("CARGO_BIN_EXE_quivive"))
        .args([
            "tile",
            "--repo",
            f.root().to_str().unwrap(),
            "--stream",
            // `dur::parse` has no sub-second unit; irrelevant here anyway —
            // the first tick fires immediately, before any sleep.
            "--interval",
            "1s",
        ])
        .env("QUIVIVE_NOW", support::NOW)
        .env_remove("PACT_STATE_DIR")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the binary under test must run");

    let stdout = child.stdout.take().expect("stdout was piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let mut line = String::new();
        let result = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(result.map(|n| (n, line)));
    });

    const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);
    let (n, line) = rx
        .recv_timeout(READ_DEADLINE)
        .expect("the first stream line must arrive well within the deadline")
        .expect("reading the first stream line must not fail");
    assert!(n > 0, "the first tick must emit immediately");
    let v: serde_json::Value =
        serde_json::from_str(line.trim()).expect("a single compact JSON line");
    assert_eq!(v["status"], "active");

    // Still alive after its first line: S9 says "stay alive between
    // changes," not "print once and exit" the way one-shot `tile` does.
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        child.try_wait().unwrap().is_none(),
        "--stream must not exit after emitting its first line"
    );

    let _ = child.kill();
    let _ = child.wait();
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
