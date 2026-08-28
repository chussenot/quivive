//! The activity reader: pact's `.pact/activity/*`, one file per agent.
//!
//! pact writes one of these on *every* invocation, from the identity resolution
//! its own `main` performs before any subcommand runs — so `pact msg inbox` or
//! `pact lease ls`, which write no ledger event, still leave a trace here. This
//! is the reader S7 means by "activity records": a second, cheaper vote on
//! whether an agent is still around, for the read-only half of a fleet's work
//! that `ledger` alone cannot see.
//!
//! ## Two kinds of evidence, not one
//!
//! pact's own record is the RFC3339 timestamp inside the file, written as
//! *content* rather than trusted to the filesystem's mtime (pact's own
//! `src/activity.rs` explains why: mtime granularity varies by filesystem and a
//! copy or archive can reset it). quivive reads both anyway and merges
//! newest-wins, the same rule the ledger, the leases and this reader all feed
//! into one accumulator with:
//!
//! * The **content**, when it parses, is pact's own answer and is exact to the
//!   microsecond.
//! * The **mtime** is evidence too, and it is what survives a record this
//!   version of quivive cannot parse — a future pact writing a format this reader
//!   has not been taught, or a half-written file caught mid-write. Dropping
//!   evidence just because we could not read the format around it would make an
//!   upgrade on pact's side look like an agent going quiet on quivive's.
//!
//! In the ordinary case the two nearly coincide — content is generated and
//! written in the same syscall's neighbourhood, measured a few milliseconds
//! apart either way on this machine's own `.pact/activity/` — so taking the
//! larger of the two costs nothing when they agree and only pays off when they
//! do not.
//!
//! ## Not the same reader as the retired `worktree`
//!
//! This says "an agent ran a pact command recently" — participation, not
//! progress. It does not say an agent is mid-edit on a file it has not yet run
//! any command about. See `docs/adr/0003-yagni-deferral-register.md` and the
//! handoff on this bead for why that gap is accepted rather than patched with a
//! reader `S4` does not name.

use std::path::Path;

use chrono::{DateTime, Utc};

pub const ACTIVITY_DIR: &str = "activity";

pub struct Reading {
    /// Newest evidence per agent: `max(content timestamp, file mtime)`.
    pub agents: Vec<(String, DateTime<Utc>)>,
    /// A record whose content did not parse as RFC3339. Its mtime still counts
    /// as evidence above; this is reported in `degraded` only, matching the
    /// ledger and lease readers' "count declines, never drop the file" rule.
    pub declined: usize,
    /// False when there is no activity directory: a repository on a pact
    /// older than pact-mqw.6 (no such record ever written), or a fleet that has
    /// not run a single command yet this checkout. Normal, not a fault.
    pub present: bool,
}

/// Read every per-agent record whole. Small — a fleet is tens of agents, not
/// thousands — so no cursor here, matching the lease reader's reasoning: a
/// record dropped between two ticks (an agent's directory pruned, or the state
/// dir moved) must vanish from the tile, and an accumulator would contradict
/// its own source.
pub fn read(state_dir: &Path) -> Reading {
    let dir = state_dir.join(ACTIVITY_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Reading {
            agents: Vec::new(),
            declined: 0,
            present: false,
        };
    };

    let mut agents = Vec::new();
    let mut declined = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        // pact's `touch()` writes with a plain create-and-truncate, never a
        // staging sibling (unlike a lock file, a record is written unconditionally
        // by whoever ran the command, so there is no acquire race to protect
        // against) — so, unlike `lease::read`, there is no sibling shape to skip
        // here. Every entry is a record.
        let Ok(meta) = entry.metadata() else {
            declined += 1;
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Some(agent) = path.file_name().and_then(|n| n.to_str()) else {
            declined += 1;
            continue;
        };
        let mtime: Option<DateTime<Utc>> = meta.modified().ok().map(DateTime::<Utc>::from);

        let content = std::fs::read_to_string(&path).ok().and_then(|raw| {
            DateTime::parse_from_rfc3339(raw.trim())
                .ok()
                .map(|t| t.to_utc())
        });
        if content.is_none() {
            declined += 1;
        }

        match (content, mtime) {
            (Some(c), Some(m)) => agents.push((agent.to_string(), c.max(m))),
            (Some(c), None) => agents.push((agent.to_string(), c)),
            (None, Some(m)) => agents.push((agent.to_string(), m)),
            // Neither readable: the entry vanished between read_dir and here, or
            // this filesystem reports no mtime at all. Nothing to contribute.
            (None, None) => {}
        }
    }
    agents.sort();

    Reading {
        agents,
        declined,
        present: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_record(dir: &Path, agent: &str, content: &str) {
        let d = dir.join(ACTIVITY_DIR);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(agent), content).unwrap();
    }

    #[test]
    fn a_missing_activity_dir_is_absent_not_declined() {
        let tmp = state();
        let r = read(tmp.path());
        assert!(!r.present);
        assert_eq!(r.declined, 0);
        assert!(r.agents.is_empty());
    }

    #[test]
    fn a_well_formed_record_is_read_from_its_content() {
        // Far enough in the future that no test machine's mtime could exceed it,
        // so this exercises the content path specifically rather than racing the
        // filesystem clock — `the_newer_of_content_and_mtime_wins` below covers
        // the merge itself.
        let tmp = state();
        write_record(tmp.path(), "readers", "2099-01-01T00:00:00Z");
        let r = read(tmp.path());
        assert!(r.present);
        assert_eq!(r.declined, 0);
        assert_eq!(r.agents.len(), 1);
        assert_eq!(r.agents[0].0, "readers");
        assert_eq!(
            r.agents[0].1,
            DateTime::parse_from_rfc3339("2099-01-01T00:00:00Z")
                .unwrap()
                .to_utc()
        );
    }

    #[test]
    fn unparsable_content_declines_but_the_mtime_still_counts() {
        let tmp = state();
        write_record(tmp.path(), "flaky", "not a timestamp");
        let r = read(tmp.path());
        assert_eq!(r.declined, 1);
        assert_eq!(r.agents.len(), 1, "the mtime is still evidence of life");
        assert_eq!(r.agents[0].0, "flaky");
    }

    #[test]
    fn the_newer_of_content_and_mtime_wins() {
        // A record whose content is stamped well in the past but whose mtime is
        // fresh — a clock skew between the writer and this reader's own view of
        // the filesystem must not make live evidence look stale.
        let tmp = state();
        write_record(tmp.path(), "skewed", "2020-01-01T00:00:00Z");
        let r = read(tmp.path());
        let now = Utc::now();
        assert!(
            now.signed_duration_since(r.agents[0].1).num_seconds() < 5,
            "mtime (now) should have won over the stale content: {:?}",
            r.agents[0].1
        );
    }

    #[test]
    fn a_subdirectory_is_neither_a_decline_nor_evidence() {
        let tmp = state();
        write_record(tmp.path(), "good", "2026-08-28T10:46:03Z");
        let dir = tmp.path().join(ACTIVITY_DIR);
        std::fs::create_dir_all(dir.join("subdir-not-a-record")).unwrap();
        let r = read(tmp.path());
        assert_eq!(r.declined, 0);
        assert_eq!(r.agents.len(), 1);
        assert_eq!(r.agents[0].0, "good");
    }
}
