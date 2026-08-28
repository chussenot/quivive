//! The tile: quivive's API.
//!
//! Not the CLI flags and not the text layout — the payload. Everything in this
//! file is the contract specified in `docs/tile-contract.md`, and the goldens
//! in `tests/goldens.rs` are what stop it moving under a consumer.
//!
//! S10-S11 of `docs/spec.md`, verbatim: `quivive tile` (one-shot) prints one
//! JSON object once and exits 0. The object is: overall `status` (one of S8's
//! five), per-repo entries with agent counts and attention items, `v` for the
//! contract. Everything below is that sentence, plus the plumbing that gets a
//! [`Payload`] out of a registry and onto stdout.
//!
//! This is a **breaking reshape** of the old single-repo vigil tile (`repo`,
//! `fleet`, `worst`, `agents[]`, `blocked_leases`, `degraded`) into the
//! multi-repo shape S11 actually specifies. `v` does not move for it: no
//! release of this crate has ever shipped, under either name, so there is no
//! consumer for the old shape to break out from under. See
//! `docs/tile-contract.md`'s "Changing the contract" section.
//!
//! **Not the CLI**, deliberately: [`build`] takes an already-resolved list of
//! repo roots rather than reading `~/.config/quivive/repos` itself or knowing
//! about `clap::Args`. Resolving "no `--repo`" to the registry (S1-S2),
//! choosing JSON vs the one-line [`Payload::text`] form, and mapping
//! `--exit-on` through [`severity`] are `src/main.rs`'s job, the same way they
//! were for the single-repo tile this replaces — see its `tick_once`. That
//! keeps this file testable as a pure function of (repo roots, clock,
//! thresholds) with no `crate::cli::Common` in its signature to go stale
//! against.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;

use crate::reader::{self, Readings, plan, sidecar};
use crate::state::{
    self, AttentionItem, GateSnapshot, LeaseSnapshot, PlanSnapshot, RepoSnapshot, RepoStatus,
    State, Thresholds, WaveSnapshot,
};

/// Contract version. See the module docs and `docs/tile-contract.md`.
pub const TILE_V: u32 = 1;

/// One repo's agents, counted by [`State`]. Deliberately just counts — S11
/// asks for "agent counts", not a per-agent list; a bar has room for four
/// numbers, not forty names, and `quivive why` (S21) is where a human goes for
/// the names.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct AgentCounts {
    pub active: usize,
    pub idle: usize,
    pub stale: usize,
    pub dead: usize,
}

/// One repo's entry in the payload.
#[derive(Debug, Serialize)]
pub struct RepoEntry {
    /// The directory basename — what a bar has room to print.
    pub name: String,
    /// The full, canonicalized path — what a human needs to go find the
    /// repository `name` alone cannot disambiguate (two checkouts of the same
    /// project, named identically, is not a hypothetical in this fleet's own
    /// registry).
    pub path: String,
    pub status: RepoStatus,
    pub agents: AgentCounts,
    /// S16-S18 items, sorted by [`AttentionItem`]'s own identity — see
    /// `state::assess`. Empty is the normal case.
    pub attention: Vec<AttentionItem>,
}

/// `quivive tile`'s whole output: one JSON object (S11).
#[derive(Debug, Serialize)]
pub struct Payload {
    pub v: u32,
    /// The clock this tick was computed against — not "now" at read time, and
    /// read exactly once for every repo in this payload. See `crate::now`.
    pub at: String,
    /// The worst status across every repo — see [`worst_status`] for the
    /// precedence and why it is this one.
    pub status: RepoStatus,
    pub repos: Vec<RepoEntry>,
}

/// S8's precedence, restated as a total order so the payload's overall
/// `status` can take a max over repos.
///
/// `human-needed` outranks everything because it is the one status that
/// actually asks a human to act right now — a bar showing anything else while
/// one repo needs a decision or is sitting on a dead agent's lease has hidden
/// the one fact that mattered. `active` is next: among the ones nobody needs
/// to act on, a fleet still working is more interesting than one that is not.
/// `drained` outranks `all-quiet` because it names a repo that WAS being
/// worked and stopped — evidence of a fleet, versus a repo nobody has ever
/// pointed one at — and a human skimming a multi-repo tile is more likely to
/// wonder "did that one finish?" than to wonder about a repo with no history
/// at all. `no-fleet` is the floor: nothing to report.
///
/// This mirrors `state::derive_status`'s own comment that nothing in that
/// module sorts by `RepoStatus` — this is the first thing that needs to, and
/// it lives here rather than as a derived `Ord` on the type itself so that
/// state.rs stays free of a concern (severity, across repos) it has never
/// needed before this file existed.
///
/// `pub`: `src/main.rs` needs this same ordering to implement `--exit-on`
/// (compare the payload's overall `status` against the threshold the CLI was
/// given), and a second copy of this ordering is exactly the drift this
/// function exists to prevent.
pub fn severity(status: RepoStatus) -> u8 {
    match status {
        RepoStatus::HumanNeeded => 4,
        RepoStatus::Active => 3,
        RepoStatus::Drained => 2,
        RepoStatus::AllQuiet => 1,
        RepoStatus::NoFleet => 0,
    }
}

/// The worst status across every repo entry, by [`severity`]. An empty list —
/// an empty registry, S1-S2 — has no repo to take the max of, and falls
/// through to `no-fleet`: the same "nothing to see" reading a single repo with
/// no pact in it gets.
fn worst_status(entries: &[RepoEntry]) -> RepoStatus {
    entries
        .iter()
        .map(|e| e.status)
        .max_by_key(|s| severity(*s))
        .unwrap_or(RepoStatus::NoFleet)
}

/// The forget sweep: agents quiet longer than `thresholds.forget` are dropped
/// from the count, UNLESS they are still holding a lease — a holder is the one
/// fact in a stale entry that is still actionable, however long ago it went
/// quiet, and `state::assess`'s `DeadHoldingPaths` needs it present to report
/// on. Without this sweep a week-old repository's `dead` count grows without
/// bound and never says anything new.
///
/// Lives here rather than in `state.rs`: forgetting is bookkeeping for a
/// *report*, not a fact about an agent's current state, and `RepoSnapshot` is
/// specified (see its handoff) as "everything one tick learned" — a sweep
/// applied before that struct is built keeps that promise about what
/// `RepoSnapshot` contains, not about what a repo actually has evidence of.
fn forget_sweep(
    readings: &Readings,
    now: DateTime<Utc>,
    thresholds: &Thresholds,
) -> BTreeMap<String, DateTime<Utc>> {
    let holders: std::collections::HashSet<&str> =
        readings.leases.iter().map(|l| l.agent.as_str()).collect();
    readings
        .agents
        .iter()
        .filter(|(id, seen)| {
            let age_s = (now.timestamp() - seen.timestamp()).max(0) as u64;
            age_s < thresholds.forget.as_secs() || holders.contains(id.as_str())
        })
        .map(|(id, seen)| (id.clone(), *seen))
        .collect()
}

/// S17: which bead ids the committed sidecar flags as needing a human
/// decision.
///
/// bd's audit export carries every field change as `kind: "field_change"` —
/// there is no `created` or `filed` kind to key off (see `reader::sidecar`'s
/// module doc) — so "filed as needing a decision" is read as: this issue has a
/// row changing `field: "type"` to `new_value: "needs-decision"`. That is the
/// exact shape `reader::sidecar`'s own doctest fixture uses, which is the
/// closest thing to a specification this reader ships with. Deduped and
/// sorted so two assessments over the same evidence agree on order, matching
/// `AttentionItem`'s own sort.
fn needs_decision_ids(rows: &[sidecar::Row]) -> Vec<String> {
    let mut ids: Vec<String> = rows
        .iter()
        .filter(|r| {
            r.field.as_deref() == Some("type") && r.new_value.as_deref() == Some("needs-decision")
        })
        .map(|r| r.issue_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Narrow pact's own `.pact/plan.json` snapshot, plus the sidecar's status
/// history, into the [`PlanSnapshot`] shape `state::assess` needs for S18.
///
/// Two judgment calls, both made here because no reader owns them (see the
/// handoff on `quivive-ykn`: "no plan reader exists yet ... state.rs only
/// defines what a parser must produce"):
///
/// * **A gate is `closed`** when the sidecar's newest `field: "status"` row
///   for that id has `new_value == "closed"` — bd's own close operation, read
///   the same way [`needs_decision_ids`] reads a decision flag. No such row
///   means open, which is the conservative default: an id this reader has no
///   evidence closed it is treated as still blocking, not silently cleared.
/// * **An id has `started`** in its wave when the sidecar shows its status
///   left `"open"` at all (claimed, in progress, closed — anything but the
///   filed state). That is the cheapest signal available that says "someone
///   began this" without needing to know bd's whole status vocabulary.
///
/// Both readings come from the SAME newest-status-per-id pass over
/// `interactions`, taken in the sidecar's own append order (oldest first —
/// see `reader::sidecar`), so "newest" here means "last row for that id",
/// with no separate timestamp sort needed.
fn build_plan_snapshot(raw: &plan::Snapshot, interactions: &[sidecar::Row]) -> PlanSnapshot {
    let mut latest_status: BTreeMap<&str, &str> = BTreeMap::new();
    for row in interactions {
        if row.field.as_deref() == Some("status")
            && let Some(v) = row.new_value.as_deref()
        {
            latest_status.insert(row.issue_id.as_str(), v);
        }
    }

    let waves_present: std::collections::BTreeSet<i64> = raw.waves.values().copied().collect();
    let mut waves: Vec<WaveSnapshot> = waves_present
        .into_iter()
        .map(|w| {
            let gates: Vec<GateSnapshot> = raw
                .gates
                .iter()
                .filter(|g| raw.waves.get(g.as_str()).copied() == Some(w))
                .map(|g| GateSnapshot {
                    id: g.clone(),
                    closed: latest_status.get(g.as_str()) == Some(&"closed"),
                })
                .collect();
            let started: Vec<String> = raw
                .waves
                .iter()
                .filter(|&(_, &wave)| wave == w)
                .filter(|(id, _)| latest_status.get(id.as_str()).is_some_and(|s| *s != "open"))
                .map(|(id, _)| id.clone())
                .collect();
            WaveSnapshot {
                // Waves are declared as small non-negative integers in every
                // real plan; a plan somehow declaring a negative one clamps to
                // 0 rather than panicking on the cast — a malformed plan is a
                // reason to degrade, never a reason to crash a tile.
                wave: w.max(0) as u32,
                gates,
                started,
            }
        })
        .collect();
    waves.sort_by_key(|w| w.wave);
    PlanSnapshot { waves }
}

/// Read one repository and build its [`RepoEntry`], committing the cursor
/// (`docs/adr/0001-stream-first-tile.md`) as the last step so a panic while
/// judging cannot leave a cursor advanced past evidence never reported.
///
/// Returns `Err` only for the one genuinely fatal read condition
/// (`reader::read`'s own: the path does not canonicalize or is not a
/// directory) — everything else this repo's readers could not parse is
/// already folded into `state::assess`'s judgment, never into an error here.
/// Callers decide what an unreadable repo means for THEM: `run` below
/// propagates it for an explicit `--repo`, because a human typed a bad path
/// and should hear about it, but degrades it to a quiet `no-fleet` entry when
/// the path came from the registry, because S1-S2's own reasoning applies
/// just as much to a repo that moved or was deleted as to a malformed line —
/// one bad entry must not take the whole fleet's tile down.
pub fn tick(
    root: &Path,
    now: DateTime<Utc>,
    thresholds: &Thresholds,
    use_cursor: bool,
) -> Result<RepoEntry> {
    let opts = reader::Options {
        repo_root: root.to_path_buf(),
        use_cursor,
    };
    let readings = reader::read(&opts)?;

    // reader::read already canonicalized and validated this path; re-deriving
    // it here (rather than threading it back out of Readings) keeps Readings
    // free of a field only this one caller wants.
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let name = canon
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| canon.display().to_string());

    // `.pact/` existing at all is a fact `Readings` does not carry (it is
    // about what the readers found, not about the directory's presence), and
    // `derive_status` needs it to tell `all-quiet` (a pact nobody has worked
    // in) from `no-fleet` (no pact at all) — S8's own line.
    let pact_present = reader::state_dir(&canon).is_dir();

    let agents = forget_sweep(&readings, now, thresholds);
    let leases: Vec<LeaseSnapshot> = readings
        .leases
        .iter()
        .map(|l| LeaseSnapshot {
            agent: l.agent.clone(),
            path: l.path.clone(),
            acquired_at: l.acquired_at,
            expires_at: l.expires_at,
        })
        .collect();
    let plan = readings
        .plan
        .as_ref()
        .map(|p| build_plan_snapshot(p, &readings.interactions));
    let needs_decision = needs_decision_ids(&readings.interactions);

    let snapshot = RepoSnapshot {
        agents,
        leases,
        plan,
        needs_decision,
        pact_present,
        degraded: readings.degraded.clone(),
    };
    let assessment = state::assess(&snapshot, now, thresholds);

    // Committed after judgment is complete, not before — see the doc comment
    // above.
    reader::commit(&canon, &readings);

    let mut counts = AgentCounts::default();
    for s in assessment.agents.values() {
        match s {
            State::Active => counts.active += 1,
            State::Idle => counts.idle += 1,
            State::Stale => counts.stale += 1,
            State::Dead => counts.dead += 1,
        }
    }

    Ok(RepoEntry {
        name,
        path: canon.display().to_string(),
        status: assessment.status,
        agents: counts,
        attention: assessment.attention,
    })
}

/// A repo entry for a path this tick could not read at all: the registry-entry
/// degradation path described on [`tick`]. Reads as `no-fleet` — the same
/// "nothing to report" a repo with no pact at all gets — rather than
/// inventing a sixth status or a per-repo error field S11 does not specify.
fn unreadable_entry(root: &Path) -> RepoEntry {
    RepoEntry {
        name: root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string()),
        path: root.display().to_string(),
        status: RepoStatus::NoFleet,
        agents: AgentCounts::default(),
        attention: Vec::new(),
    }
}

/// Build the whole payload: one clock read (`now`, taken by the caller and
/// passed down — see `crate::now`), one [`tick`] per repo, one overall status.
///
/// `degrade_unreadable` is true for the registry path and false for an
/// explicit `--repo` — see [`tick`]'s doc comment for why the two cases answer
/// "the path was bad" differently.
pub fn build(
    repo_roots: &[PathBuf],
    now: DateTime<Utc>,
    thresholds: &Thresholds,
    use_cursor: bool,
    degrade_unreadable: bool,
) -> Result<Payload> {
    let mut repos = Vec::with_capacity(repo_roots.len());
    for root in repo_roots {
        match tick(root, now, thresholds, use_cursor) {
            Ok(entry) => repos.push(entry),
            Err(e) if degrade_unreadable => repos.push(unreadable_entry_named(root, &e)),
            Err(e) => return Err(e),
        }
    }
    let status = worst_status(&repos);
    Ok(Payload {
        v: TILE_V,
        at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
        status,
        repos,
    })
}

/// [`unreadable_entry`], but keeping the reason on stderr rather than
/// swallowing it entirely — a registry entry that stopped resolving is worth a
/// line for whoever is watching the terminal, even though the JSON payload
/// itself (S11) has nowhere to carry it.
fn unreadable_entry_named(root: &Path, err: &anyhow::Error) -> RepoEntry {
    eprintln!("quivive: {} (from the registry): {err:#}", root.display());
    unreadable_entry(root)
}

impl Payload {
    /// A compact one-line summary — NOT part of the contract S11 pins (the
    /// payload IS the JSON object; pwetty and every other consumer reads
    /// that). Offered because it costs nothing on top of a `Payload` that
    /// already exists: one pass counting repos by status. Useful at a
    /// terminal; not goldened or documented as a stable shape the way the old
    /// single-repo text tile was.
    pub fn text(&self) -> String {
        if self.repos.is_empty() {
            return format!("{}  no repos registered", status_str(self.status));
        }
        let mut by_status: Vec<(String, usize)> = Vec::new();
        for r in &self.repos {
            let s = status_str(r.status);
            match by_status.iter_mut().find(|(k, _)| *k == s) {
                Some((_, n)) => *n += 1,
                None => by_status.push((s, 1)),
            }
        }
        let parts: Vec<String> = by_status.iter().map(|(s, n)| format!("{n} {s}")).collect();
        format!(
            "{}  {} repo{}: {}",
            status_str(self.status),
            self.repos.len(),
            if self.repos.len() == 1 { "" } else { "s" },
            parts.join(", ")
        )
    }
}

/// `RepoStatus`'s S8 spelling, by asking serde rather than hand-copying its
/// `#[serde(rename_all = "kebab-case")]` list a second time — the JSON payload
/// and this text form are then structurally unable to disagree about a status
/// name. Only used for the bonus text form above; the JSON payload serializes
/// `RepoStatus` directly and never goes through this.
fn status_str(status: RepoStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}
