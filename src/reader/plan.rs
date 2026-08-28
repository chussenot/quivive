//! The plan reader: `.pact/plan.json`, pact's linted snapshot of a wave plan.
//!
//! Written once, by `pact plan lint`, only when the lint found no *errors* —
//! never by vigil, and never incrementally. A tick either sees the last clean
//! lint's graph whole, or it sees nothing; there is no cursor here because
//! there is nothing to stream, matching the lease and activity readers'
//! reasoning in `docs/adr/0001-stream-first-tile.md`.
//!
//! ## Why the shape mirrors pact's own `Snapshot` field for field
//!
//! This is pact's file, not vigil's, so the struct below is deliberately a
//! narrow copy of `pact::plan::Snapshot` rather than an independent guess at
//! what a plan "should" look like. Divergence here would be silent: pact bumps
//! nothing when it adds a field, because `.pact/plan.json` is a private
//! contract with `pact audit`'s `gate-order` check and `pact handoff`, not a
//! versioned wire format vigil is entitled to assume stability from. Reading
//! only the fields both `waves` and `gates` need keeps this reader working
//! across a pact upgrade the same way `ledger::Row` keeps working across new
//! event kinds — by not caring about the rest.
//!
//! `waves` and `gates` are `#[serde(default)]` for the same reason pact
//! declares them that way: a snapshot written before gates existed carries
//! neither key, and must parse as "a plan with no gates" rather than fail.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

pub const PLAN_FILE: &str = "plan.json";

/// The parts of pact's plan snapshot that stay interesting after the lint ran:
/// what `S18` (gate-order) and `S8` (drained detection) need downstream.
#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    /// RFC3339, when the lint that wrote this ran. Kept as the raw string pact
    /// wrote — nothing here reasons about its age, so parsing it into a
    /// `DateTime` would be a fact this reader asserts and nobody uses.
    pub at: String,
    /// `id -> depends_on`, every entry the manifest declared.
    pub edges: BTreeMap<String, Vec<String>>,
    /// `id -> wave`, for the entries that declared one.
    #[serde(default)]
    pub waves: BTreeMap<String, i64>,
    /// The entries the plan declared as gates (`pact-gyn`).
    #[serde(default)]
    pub gates: Vec<String>,
}

pub struct Reading {
    pub snapshot: Option<Snapshot>,
    /// 1 when the file exists but did not parse as a `Snapshot`, 0 otherwise.
    /// Not a per-line count — this is one JSON object, not a log — but reported
    /// as `declined` for the same reason every other reader counts rather than
    /// swallows: a plan vigil could not read is not silence, and `pact plan
    /// lint` having moved the schema out from under an old vigil is exactly the
    /// case a decline count exists to surface.
    pub declined: usize,
    /// False when there is no plan at all: a repository nobody has run
    /// `pact plan lint` in yet, or one that has never planned a fleet. Normal —
    /// see `docs/spec.md#tick`'s `no-fleet` status.
    pub present: bool,
}

pub fn read(state_dir: &Path) -> Reading {
    let path = state_dir.join(PLAN_FILE);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Reading {
            snapshot: None,
            declined: 0,
            present: false,
        };
    };
    match serde_json::from_str::<Snapshot>(&raw) {
        Ok(snapshot) => Reading {
            snapshot: Some(snapshot),
            declined: 0,
            present: true,
        },
        Err(_) => Reading {
            snapshot: None,
            declined: 1,
            present: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(json: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(PLAN_FILE), json).unwrap();
        dir
    }

    #[test]
    fn a_missing_plan_is_absent_not_declined() {
        let dir = tempfile::tempdir().unwrap();
        let r = read(dir.path());
        assert!(!r.present);
        assert_eq!(r.declined, 0);
        assert!(r.snapshot.is_none());
    }

    #[test]
    fn a_real_snapshot_parses_edges_waves_and_gates() {
        let dir = state(
            r#"{"at":"2026-08-25T00:00:00Z","edges":{"g-tst":[],"m-imp":["g-tst"]},
               "waves":{"g-tst":0,"m-imp":1},"gates":["g-tst"]}"#,
        );
        let r = read(dir.path());
        assert!(r.present);
        assert_eq!(r.declined, 0);
        let s = r.snapshot.expect("parses");
        assert_eq!(s.edges["m-imp"], vec!["g-tst".to_string()]);
        assert_eq!(s.waves["m-imp"], 1);
        assert_eq!(s.gates, vec!["g-tst".to_string()]);
    }

    #[test]
    fn a_pre_gate_snapshot_with_no_waves_or_gates_key_still_parses() {
        // pact's own compatibility case: a snapshot written before gates existed.
        let dir = state(r#"{"at":"2026-08-25T00:00:00Z","edges":{"m-imp":[]}}"#);
        let r = read(dir.path());
        let s = r.snapshot.expect("parses despite missing keys");
        assert!(s.waves.is_empty());
        assert!(s.gates.is_empty());
    }

    #[test]
    fn garbage_is_declined_not_fatal() {
        let dir = state("not json at all");
        let r = read(dir.path());
        assert!(r.present, "the file exists, so this is not `no-fleet`");
        assert_eq!(r.declined, 1);
        assert!(r.snapshot.is_none());
    }
}
