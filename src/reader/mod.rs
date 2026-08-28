//! The tick: repo -> readers -> merged evidence -> the tile.
//!
//! See the data-flow diagram in `docs/spec.md#the-tick`. This module is the solid
//! edges of it; `cursor` is the dotted ones.

pub mod activity;
pub mod lease;
pub mod ledger;
pub mod plan;
pub mod sidecar;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::cursor::{self, Cursor};

/// Where pact keeps its state for this repository.
///
/// `PACT_STATE_DIR` is honoured because pact honours it: a repository whose state
/// has been redirected is one where `<repo>/.pact` is empty, and vigil reporting
/// "no ledger" there would be confidently wrong. pact's worktree-scope
/// redirection is deliberately NOT reimplemented — that is a second copy of
/// somebody else's resolution logic, which drifts. A worktree whose state lives
/// elsewhere reads as no ledger, which is honest.
pub fn state_dir(repo_root: &Path) -> PathBuf {
    match std::env::var_os("PACT_STATE_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => repo_root.join(".pact"),
    }
}

pub struct Readings {
    /// Newest evidence per agent, every source merged.
    pub agents: BTreeMap<String, DateTime<Utc>>,
    pub leases: Vec<lease::Lease>,
    /// The last clean `pact plan lint` snapshot, or `None` when this repository
    /// has never been planned. Feeds `S8`'s `drained` detection and `S18`'s
    /// gate-order detection downstream — this reader only parses it.
    pub plan: Option<plan::Snapshot>,
    /// `S18`'s started-bead evidence, folded from the events tail: every id
    /// an `acquired` row's `bead` field has ever named (`reader::ledger`'s own
    /// incremental fold, carried across resumed reads by the cursor). Empty
    /// when nothing has ever been started with `--bead`, which is the normal
    /// case for a repository not using pact's `--bead` flag.
    pub started: BTreeSet<String>,
    /// Every row of `.beads/interactions.jsonl` that parsed, oldest first.
    /// Empty when the sidecar is absent or bd's audit export is disabled — see
    /// `reader::sidecar`. What counts as a needs-decision bead (`S17`) is a
    /// judgement for whoever consumes this, not for the reader.
    pub interactions: Vec<sidecar::Row>,
    /// Readers that could not read, and decline counts, named for the tile's
    /// `degraded` field. Never an error: a repository with no pact in it is a
    /// normal repository, and a tile that exited non-zero over one would take a
    /// status bar down for a condition that is not a fault.
    pub degraded: Vec<String>,
    /// The cursor to persist, or `None` when the caller asked not to use one.
    pub cursor: Option<Cursor>,
    /// True when the ledger was fully re-read rather than resumed. Reported by
    /// `--json` so the fleet soak can tell the control leg from the treatment.
    pub cold: bool,
}

pub struct Options {
    pub repo_root: PathBuf,
    /// False forces a cold read and writes no cursor — `--no-cursor`. This is
    /// the fastest diagnostic in the codebase: if the tile changes when you pass
    /// it, the cursor is wrong and you already know which half to read.
    pub use_cursor: bool,
}

pub fn read(opts: &Options) -> Result<Readings> {
    // The one genuinely fatal condition: we were pointed at something that is not
    // a directory. Everything downstream of here degrades instead of failing.
    let repo_root = opts
        .repo_root
        .canonicalize()
        .with_context(|| format!("--repo {}", opts.repo_root.display()))?;
    anyhow::ensure!(
        repo_root.is_dir(),
        "--repo {} is not a directory",
        repo_root.display()
    );

    let state = state_dir(&repo_root);
    let prior = opts.use_cursor.then(|| cursor::load(&state)).flatten();

    let led = ledger::read(&state, prior);
    let lea = lease::read(&state);
    let act = activity::read(&state);
    let pln = plan::read(&state);
    let sid = sidecar::read(&repo_root);

    let mut degraded = Vec::new();
    if !led.present {
        degraded.push("ledger".to_string());
    }
    if led.declined > 0 {
        degraded.push(format!("ledger: {} unparsable line(s)", led.declined));
    }
    if lea.declined > 0 {
        degraded.push(format!("lease: {} unparsable lock(s)", lea.declined));
    }
    // A missing leases directory, a missing activity directory, a missing plan
    // and a missing sidecar are all deliberately NOT degraded: each is a normal
    // resting state (nobody holding a path, a fleet that has run no command yet,
    // a repository nobody has planned, bd's opt-in export left off), not a
    // fault, and a bar dimmed for any of them would be dimmed most of the time.
    if act.declined > 0 {
        degraded.push(format!("activity: {} unparsable record(s)", act.declined));
    }
    if pln.declined > 0 {
        degraded.push(format!("plan: {} unparsable file", pln.declined));
    }
    if sid.declined > 0 {
        degraded.push(format!("sidecar: {} unparsable line(s)", sid.declined));
    }

    // Merge, newest wins. The ledger's fold is the accumulator; lease and
    // activity evidence is layered on top and never written back into the
    // cursor — see `docs/adr/0001-stream-first-tile.md`'s "a reader that is not
    // append-only does not fit this design".
    let mut agents = led.agents.clone();
    let mut note = |agent: &str, at: DateTime<Utc>| {
        agents
            .entry(agent.to_string())
            .and_modify(|seen| {
                if at > *seen {
                    *seen = at;
                }
            })
            .or_insert(at);
    };
    for l in &lea.leases {
        note(&l.agent, l.acquired_at);
    }
    for (agent, at) in &act.agents {
        note(agent, *at);
    }

    Ok(Readings {
        agents,
        leases: lea.leases,
        plan: pln.snapshot,
        started: led.started,
        interactions: sid.rows,
        degraded,
        cursor: opts.use_cursor.then_some(led.cursor),
        cold: led.cold,
    })
}

/// Persist the cursor, if there is one to persist. Never fails a tick.
pub fn commit(repo_root: &Path, readings: &Readings) {
    if let Some(c) = &readings.cursor {
        let state = state_dir(repo_root);
        // Only into a state directory that already exists. Creating `.pact/` in a
        // repository that has no pact would be vigil initialising somebody else's
        // tool, and would make `vigil tile` leave a trace in a repo it had nothing
        // to say about.
        if state.is_dir() {
            cursor::save(&state, c);
        }
    }
}

/// The five paths a tick reads (`S4`), cheapest to stat first, and the newest
/// mtime among whichever of them exist.
///
/// `None` when nothing exists at all — the `no-fleet` case, and also (by
/// construction, since an mtime cannot be observed on a path that vanished) the
/// case where a repository's entire `.pact` was just removed. Both read as
/// "nothing to compare", which is why [`unchanged`] treats `None` as "cannot
/// prune, read for real" rather than as evidence of quiet: a real read of an
/// absent tree is itself a handful of failed `stat`s, no more expensive than
/// this scan, so there is nothing to gain by trying to distinguish the two here.
fn newest_source_mtime(repo_root: &Path) -> Option<DateTime<Utc>> {
    let state = state_dir(repo_root);
    let mut newest: Option<DateTime<Utc>> = None;
    let mut bump = |t: Option<DateTime<Utc>>| {
        if let Some(t) = t
            && newest.is_none_or(|n| t > n)
        {
            newest = Some(t);
        }
    };

    let mtime_of = |p: &Path| -> Option<DateTime<Utc>> {
        std::fs::metadata(p).ok()?.modified().ok().map(Into::into)
    };

    bump(mtime_of(&state.join(ledger::LEDGER_FILE)));

    // The leases directory's OWN mtime is enough: every mutation pact makes to a
    // lock file — acquire, renew, steal, release — goes through a create, a
    // rename-over-the-destination, or an unlink, and every one of those is a
    // change to the DIRECTORY's entries, not just to a file's content. A plain
    // in-place rewrite would not bump it; pact does not do one here.
    bump(mtime_of(&state.join(lease::LEASES_DIR)));

    // The activity directory's own mtime is NOT enough, and deliberately not
    // used: pact's `touch()` opens an EXISTING record and overwrites its
    // content in place (`src/activity.rs`, "the timestamp as CONTENT, not as
    // the file's mtime") — no rename, no create-or-remove once the file exists
    // for the first time. That never touches the directory's own entries, so an
    // agent renewing its record every tick would look quiet to a directory-level
    // check. Each record's own mtime has to be read instead — cheap, because a
    // fleet is tens of agents, not thousands.
    if let Ok(entries) = std::fs::read_dir(state.join(activity::ACTIVITY_DIR)) {
        for entry in entries.flatten() {
            bump(
                entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(Into::into),
            );
        }
    }

    bump(mtime_of(&state.join(plan::PLAN_FILE)));
    bump(mtime_of(
        &repo_root.join(".beads").join(sidecar::SIDECAR_FILE),
    ));

    newest
}

/// `S5`'s mtime-pruning gate: true when nothing the tick would read has changed
/// since `prior`, so the caller may skip this repo wholesale rather than
/// running any of the readers above.
///
/// **TOCTOU tolerance, stated rather than hidden**: on a filesystem whose mtime
/// resolution is coarser than a tick (a whole second, on some), a write that
/// lands in the same second as the mtime `prior` was captured from can compare
/// equal rather than greater, and this reports `unchanged` for one tick it
/// should not have. Nothing is lost — every reader here reads its source
/// whole or resumes from a verified cursor rather than trusting a delta, so the
/// very next tick whose mtime *does* advance catches up on everything the
/// skipped tick missed. That is the trade a presence tile is allowed to make:
/// a status bar that is one second slow to notice is not the failure mode
/// `docs/adr/0001-stream-first-tile.md` protects against; silently forgetting
/// an agent forever is, and this gate cannot cause that.
///
/// Persisting `prior` across ticks, and iterating repositories with it, is the
/// caller's job (multi-repo iteration lives with the tile bead) — this
/// function is a pure comparison, so a caller can freely re-check it as often
/// as it likes without touching disk beyond the stats above.
pub fn unchanged(repo_root: &Path, prior: DateTime<Utc>) -> bool {
    newest_source_mtime(repo_root).is_some_and(|newest| newest <= prior)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn repo_with_pact() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".pact")).unwrap();
        std::fs::write(dir.path().join(".pact").join(ledger::LEDGER_FILE), "").unwrap();
        dir
    }

    #[test]
    fn an_empty_pact_dir_has_a_source_mtime() {
        let dir = repo_with_pact();
        assert!(newest_source_mtime(dir.path()).is_some());
    }

    #[test]
    fn a_repository_with_no_pact_at_all_has_no_source_mtime() {
        let dir = tempfile::tempdir().unwrap();
        assert!(newest_source_mtime(dir.path()).is_none());
    }

    #[test]
    fn unchanged_is_true_when_the_prior_stamp_is_in_the_future() {
        let dir = repo_with_pact();
        let far_future = Utc::now() + chrono::Duration::days(365);
        assert!(unchanged(dir.path(), far_future));
    }

    #[test]
    fn a_new_ledger_write_moves_the_gate() {
        let dir = repo_with_pact();
        let prior = newest_source_mtime(dir.path()).unwrap();
        // Real filesystems can have coarser-than-microsecond mtime resolution
        // (see the TOCTOU note on `unchanged`); sleeping past a whole second is
        // what makes this assertion robust rather than flaky in CI.
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(dir.path().join(".pact").join(ledger::LEDGER_FILE), "more").unwrap();
        assert!(
            !unchanged(dir.path(), prior),
            "a ledger write after `prior` must not be pruned away"
        );
    }

    #[test]
    fn an_activity_record_rewritten_in_place_still_moves_the_gate() {
        // The case the module doc calls out by name: a directory-level mtime
        // check alone would miss this, because the record's directory entry
        // never changes, only its content.
        let dir = repo_with_pact();
        std::fs::create_dir_all(dir.path().join(".pact").join(activity::ACTIVITY_DIR)).unwrap();
        let record = dir
            .path()
            .join(".pact")
            .join(activity::ACTIVITY_DIR)
            .join("agent-a");
        std::fs::write(&record, "2020-01-01T00:00:00Z").unwrap();
        let prior = newest_source_mtime(dir.path()).unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(&record, "2020-01-01T00:00:01Z").unwrap();
        assert!(!unchanged(dir.path(), prior));
    }

    #[test]
    fn a_repository_that_lost_pact_entirely_is_never_pruned() {
        // Nothing left to stat reads as "cannot compare", not as "quiet forever".
        let dir = repo_with_pact();
        std::fs::remove_dir_all(dir.path().join(".pact")).unwrap();
        assert!(!unchanged(dir.path(), Utc::now()));
    }
}
