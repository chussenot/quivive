//! `quivive why <repo> [--json]`: the attention-worthy items for one repo, each
//! with the evidence line(s) that produced it (S21 of `docs/spec.md`).
//!
//! This is the first real caller of `state::assess` outside its own unit tests
//! (see the handoff on quivive-ykn): [`build`] runs the single-repo pipeline —
//! read, fold into a [`state::RepoSnapshot`], judge with [`state::assess`] —
//! and then does the one thing `state.rs` deliberately does not: point at the
//! file (and, where one exists, the line) that produced each
//! [`state::AttentionItem`]. `state.rs`'s own doc comments name three gaps left
//! for whoever builds the first real `RepoSnapshot`: no plan-plus-events-tail
//! reader exists yet, gate `closed` status is undefined, and a `GateOrderViolation`
//! carries no event-line number. This file settles all three, locally, because
//! nothing else in the tree has yet:
//!
//! * **Gate `closed`.** A gate is a bead id. The only file-based signal that a
//!   bead closed is bd's committed sidecar: the newest `field="status"` row for
//!   that id, if its `new_value` is `"closed"`. Nothing under `.pact/` records
//!   bead status — pact's own event kinds (`acquired`/`released`/`watched`/
//!   `context`/`expired`/`displaced`/`annotation`) know nothing about beads
//!   closing.
//! * **A wave's `started` ids.** The events tail's `bead` field (present on
//!   `acquired` rows when the caller passed `--bead`, e.g.
//!   `{"kind":"acquired",...,"bead":"quivive-ipc"}` — see a real
//!   `.pact/events.jsonl`) is the only file-based signal that work under a given
//!   id has begun. `reader::ledger::Row` does not carry it (deliberately narrow,
//!   per its own doc comment), so this file re-reads `events.jsonl` itself for
//!   just this one field, tolerant of garbage the same way `ledger::read` is.
//! * **The event-line number.** Carried alongside `bead` in the same scan, so a
//!   `GateOrderViolation`'s follow-up can be the real `recount explain
//!   --event-line N` S20 promises, not the bead-id fallback.
//!
//! `build` reads every source cold, with no cursor: `why` is a one-shot answer
//! to "what needs a human", not a repeated tick, so there is nothing to resume
//! and nothing to persist — unlike `main.rs`'s `tick_once`, this never calls
//! `reader::commit`.
//!
//! ## The `--json` shape
//!
//! One object, matching [`WhyOutput`]'s field order:
//!
//! ```json
//! {
//!   "repo": "/abs/path/to/repo",
//!   "status": "human-needed",
//!   "items": [
//!     {
//!       "kind": "dead_holding_paths",
//!       "summary": "agent-3 is dead and holds 1 path(s): src/a.rs (lease TTL remaining 0s)",
//!       "follow_up": "pact lease ls",
//!       "evidence": [
//!         { "file": ".pact/leases/....lock" },
//!         { "file": ".pact/activity/agent-3" }
//!       ]
//!     }
//!   ]
//! }
//! ```
//!
//! `status` is one of S8's five spellings (the same `RepoStatus` the tile
//! emits). `items` is `[]`, never omitted, for both "nothing attention-worthy"
//! and "no pact here" — the caller does not need a second shape to handle the
//! quiet case. `kind` is one of `dead_holding_paths` / `needs_decision` /
//! `gate_order_violation`, the same spellings `state::AttentionItem`'s own
//! `#[serde(tag = "kind")]` would produce, kept in sync by hand in
//! [`item_kind`] since this file builds its own `Item` rather than serializing
//! `AttentionItem` directly (a JSON consumer wants `summary`/`follow_up`
//! alongside the payload, not the raw enum). `evidence[].line` is 1-based and
//! omitted (not `null`) when the citation is a whole file rather than one line
//! of it — S21 accepts "file plus line **or** path".

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{dur, reader, state};

/// One citation: a real file under the inspected repo, and the line inside it
/// when the evidence is line-shaped (a ledger or sidecar row) rather than
/// file-shaped (a lock file, an activity record, `plan.json` whole).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    /// Repo-relative when the evidence lives under the repo (the normal case);
    /// absolute if `PACT_STATE_DIR` has moved `.pact` outside it — either way, a
    /// path this process actually read.
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

impl Evidence {
    fn file(file: String) -> Self {
        Self { file, line: None }
    }
}

/// One attention item, rendered for a human or for `--json`.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub kind: &'static str,
    pub summary: String,
    pub follow_up: String,
    pub evidence: Vec<Evidence>,
}

/// `quivive why`'s whole answer. See the module doc for the `--json` shape this
/// serializes to.
#[derive(Debug, Clone, Serialize)]
pub struct WhyOutput {
    pub repo: String,
    pub status: state::RepoStatus,
    pub items: Vec<Item>,
}

impl WhyOutput {
    /// The text form: one block per item, its evidence lines indented under it.
    /// Always ends in a single trailing newline, matching `Tile::text`'s "one
    /// line, always" discipline extended to "one block, always" here.
    pub fn text(&self) -> String {
        let mut out = format!("repo: {}\nstatus: {}\n", self.repo, status_str(self.status));
        if self.items.is_empty() {
            out.push_str("nothing attention-worthy\n");
            return out;
        }
        for item in &self.items {
            out.push_str(&format!("\n- {}: {}\n", item.kind, item.summary));
            out.push_str(&format!("    follow-up: {}\n", item.follow_up));
            for ev in &item.evidence {
                match ev.line {
                    Some(l) => out.push_str(&format!("    evidence: {}:{l}\n", ev.file)),
                    None => out.push_str(&format!("    evidence: {}\n", ev.file)),
                }
            }
        }
        out
    }
}

/// `RepoStatus`'s kebab-case spelling, without hand-duplicating the five
/// strings: `RepoStatus` already derives `Serialize` with exactly the spelling
/// S8 wants (see `state.rs`), so asking serde for it once is the one place that
/// spelling lives. `unreachable!` is safe here — every variant serializes to a
/// JSON string, never to an object or array.
fn status_str(status: state::RepoStatus) -> String {
    match serde_json::to_value(status) {
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("RepoStatus always serializes to a bare string"),
    }
}

/// `state::AttentionItem`'s own `#[serde(tag = "kind", rename_all = "snake_case")]`
/// spelling, kept by hand because this file's `Item` carries fields
/// `AttentionItem` does not (`summary`, `follow_up`) and so cannot just
/// `#[serde(flatten)]` the enum in. Three variants, exhaustively matched, so
/// `state.rs` growing a fourth is a compile error here rather than a silent gap.
fn item_kind(item: &state::AttentionItem) -> &'static str {
    match item {
        state::AttentionItem::DeadHoldingPaths { .. } => "dead_holding_paths",
        state::AttentionItem::NeedsDecision { .. } => "needs_decision",
        state::AttentionItem::GateOrderViolation { .. } => "gate_order_violation",
    }
}

/// A path under `repo_root`, rendered relative to it when it actually is one —
/// the normal case — and absolute otherwise (`PACT_STATE_DIR` can move `.pact`
/// outside the repo entirely; see `reader::state_dir`). Either way the string
/// names a path this process opened, never a guess at one.
fn rel_or_abs(repo_root: &Path, p: &Path) -> String {
    match p.strip_prefix(repo_root) {
        Ok(rel) => rel.display().to_string(),
        Err(_) => p.display().to_string(),
    }
}

/// One `.pact/leases/*.lock` file's `path` field, matched against its own
/// filesystem path. A deliberately narrow re-read of the same directory
/// `reader::lease::read` already parsed: that reader turns each lock into a
/// judgment-ready `Lease` and discards the lock file's own path once parsed,
/// because nothing before this bead ever needed to cite the file back. Reading
/// the `path` field a second time, rather than teaching `reader::lease` to keep
/// its source path, keeps this bead inside its own lease (`src/why.rs`) instead
/// of reopening a file the seam and the tile bead both depend on.
fn scan_lock_files(state_dir: &Path, repo_root: &Path) -> BTreeMap<String, String> {
    #[derive(Deserialize)]
    struct PathOnly {
        path: String,
    }

    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(state_dir.join(reader::lease::LEASES_DIR)) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        // `.lock` only — a staging sibling mid-acquire is not a real lease to
        // cite, matching `reader::lease::read`'s own filter.
        if p.extension().is_none_or(|e| e != "lock") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<PathOnly>(&raw) else {
            continue;
        };
        out.insert(parsed.path, rel_or_abs(repo_root, &p));
    }
    out
}

/// One `bead`-tagged `acquired` row from the raw events tail, plus the 1-based
/// line it came from — the two things `S18`'s `GateOrderViolation` needs that
/// `reader::ledger::Row` deliberately does not carry (see the module doc).
/// `acquired` specifically: it is the one event kind that fires the moment an
/// agent starts real work under a bead, where `released`/`watched`/`context`
/// either come later or never name work beginning at all.
///
/// Tolerant of garbage lines the same way `reader::ledger::read` is (`S4`): a
/// line that is not valid JSON, or has no `bead` field, contributes nothing and
/// is not an error — most rows have no `bead` at all, and that is normal.
fn scan_started(state_dir: &Path) -> BTreeMap<String, usize> {
    #[derive(Deserialize)]
    struct Raw {
        kind: String,
        #[serde(default)]
        bead: Option<String>,
    }

    let mut first_line: BTreeMap<String, usize> = BTreeMap::new();
    let Ok(contents) = std::fs::read_to_string(state_dir.join(reader::ledger::LEDGER_FILE)) else {
        return first_line;
    };
    for (i, line) in contents.lines().enumerate() {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let Ok(raw) = serde_json::from_str::<Raw>(text) else {
            continue;
        };
        if raw.kind != "acquired" {
            continue;
        }
        if let Some(bead) = raw.bead {
            // Earliest line wins: that is the line that actually produced the
            // "started" fact, not a later renewal of the same claim.
            first_line.entry(bead).or_insert(i + 1);
        }
    }
    first_line
}

/// Whether the sidecar shows `gate_id` as closed: the newest `field="status"`
/// row for that bead id has `new_value == "closed"`. `rows` is oldest-first
/// (`reader::sidecar::read`'s own order), so the newest is the last match.
///
/// This is a real design decision, not a mirror of an existing contract: no
/// signal under `.pact/` says a bead closed (pact's event kinds do not know
/// beads have status), so bd's own sidecar is the only file `S18` can read this
/// from. See the module doc.
fn gate_closed(rows: &[reader::sidecar::Row], gate_id: &str) -> bool {
    rows.iter()
        .rfind(|r| r.issue_id == gate_id && r.field.as_deref() == Some("status"))
        .is_some_and(|r| r.new_value.as_deref() == Some("closed"))
}

/// Bead ids the sidecar flags as needing a human decision (`S17`), each with
/// the line of the row that first flagged it. A `field="type"` row whose
/// `new_value` is `"needs-decision"` is the signal — see the handoff on
/// quivive-o3j: "bd only ever emits kind=field_change... likely a
/// field=type/new_value=needs-decision row", which is what every fixture and
/// this reader agree on. Deduplicated by id: a bead flagged twice must produce
/// one attention item, not two indistinguishable ones (`state::assess` does
/// not dedupe `RepoSnapshot.needs_decision` itself — filtering the sidecar's
/// own shape, including this, is documented as this layer's job).
fn needs_decision(rows: &[reader::sidecar::Row]) -> (Vec<String>, BTreeMap<String, usize>) {
    let mut ids = Vec::new();
    let mut line_of = BTreeMap::new();
    for row in rows {
        if row.field.as_deref() == Some("type")
            && row.new_value.as_deref() == Some("needs-decision")
        {
            if !line_of.contains_key(&row.issue_id) {
                ids.push(row.issue_id.clone());
            }
            line_of.entry(row.issue_id.clone()).or_insert(row.line);
        }
    }
    (ids, line_of)
}

/// `.pact/plan.json` plus the events tail, narrowed to `state::PlanSnapshot`'s
/// shape (`S18`). One `WaveSnapshot` per distinct wave number the plan
/// declares; a wave with no gates and nothing started still gets an entry so a
/// later gate declared in it is not silently absent from the judgment.
fn build_plan_snapshot(
    plan: &reader::plan::Snapshot,
    sidecar_rows: &[reader::sidecar::Row],
    started: &BTreeMap<String, usize>,
) -> state::PlanSnapshot {
    let mut wave_numbers: Vec<i64> = plan.waves.values().copied().collect();
    wave_numbers.sort_unstable();
    wave_numbers.dedup();

    let waves = wave_numbers
        .into_iter()
        .map(|w| {
            let gates = plan
                .gates
                .iter()
                .filter(|id| plan.waves.get(id.as_str()) == Some(&w))
                .map(|id| state::GateSnapshot {
                    id: id.clone(),
                    closed: gate_closed(sidecar_rows, id),
                })
                .collect();
            // `BTreeMap::iter` is id-sorted, so this is deterministic without a
            // second sort — matters because two `why` runs over the same
            // evidence must agree byte-for-byte under `--json`.
            let started_here = plan
                .waves
                .iter()
                .filter(|&(_, &wave)| wave == w)
                .filter(|(id, _)| started.contains_key(id.as_str()))
                .map(|(id, _)| id.clone())
                .collect();
            state::WaveSnapshot {
                wave: w.max(0) as u32,
                gates,
                started: started_here,
            }
        })
        .collect();

    state::PlanSnapshot { waves }
}

/// One agent's newest evidence, plus where it came from — the citation `S21`
/// wants alongside a `DeadHoldingPaths` item's lock files.
struct AgentSeen {
    seen: DateTime<Utc>,
    source: Evidence,
}

/// Newer strictly wins, keeping the earlier source on a tie — mirrors
/// `reader::mod::read`'s own merge (`if at > *seen`) exactly, because
/// `RepoSnapshot.agents` here must classify agents identically to what
/// `quivive tile` would report for the same repo at the same instant; a
/// `why` that disagreed with the `tile` that pointed at it would be its own
/// defect.
fn bump(out: &mut BTreeMap<String, AgentSeen>, agent: &str, at: DateTime<Utc>, source: Evidence) {
    match out.get_mut(agent) {
        Some(existing) if at > existing.seen => {
            existing.seen = at;
            existing.source = source;
        }
        Some(_) => {}
        None => {
            out.insert(agent.to_string(), AgentSeen { seen: at, source });
        }
    }
}

/// Newest evidence per agent, same fold `reader::mod::read` performs (ledger,
/// then leases, then activity — newest wins), reimplemented here rather than
/// called through `reader::read` because that function discards provenance:
/// it hands back one merged timestamp per agent and nothing about which
/// reader supplied it, which is exactly the fact `S21` needs cited for
/// `DeadHoldingPaths`.
fn merge_agents(
    repo_root: &Path,
    state_dir: &Path,
    ledger_agents: &BTreeMap<String, DateTime<Utc>>,
    leases: &[reader::lease::Lease],
    activity: &[(String, DateTime<Utc>)],
    lock_files: &BTreeMap<String, String>,
) -> BTreeMap<String, AgentSeen> {
    let events_file = rel_or_abs(repo_root, &state_dir.join(reader::ledger::LEDGER_FILE));
    let mut out = BTreeMap::new();
    for (agent, seen) in ledger_agents {
        out.insert(
            agent.clone(),
            AgentSeen {
                seen: *seen,
                source: Evidence::file(events_file.clone()),
            },
        );
    }
    for lease in leases {
        let file = lock_files
            .get(&lease.path)
            .cloned()
            .unwrap_or_else(|| events_file.clone());
        bump(
            &mut out,
            &lease.agent,
            lease.acquired_at,
            Evidence::file(file),
        );
    }
    for (agent, at) in activity {
        let file = rel_or_abs(
            repo_root,
            &state_dir.join(reader::activity::ACTIVITY_DIR).join(agent),
        );
        bump(&mut out, agent, *at, Evidence::file(file));
    }
    out
}

/// Every citation source [`render_item`] needs, bundled into one borrow so
/// the function itself takes two arguments instead of eight —
/// `clippy::too_many_arguments`, and also just easier to read at the call
/// site than seven positional strings and maps in a row.
struct EvidenceSources<'a> {
    agent_seen: &'a BTreeMap<String, AgentSeen>,
    lock_files: &'a BTreeMap<String, String>,
    needs_decision_lines: &'a BTreeMap<String, usize>,
    sidecar_file: &'a str,
    plan_file: &'a str,
    events_file: &'a str,
    started: &'a BTreeMap<String, usize>,
}

/// One `AttentionItem` -> one rendered `Item`, with real evidence attached.
fn render_item(item: &state::AttentionItem, sources: &EvidenceSources<'_>) -> Item {
    match item {
        state::AttentionItem::DeadHoldingPaths {
            agent,
            paths,
            remaining_ttl,
        } => {
            let mut evidence: Vec<Evidence> = paths
                .iter()
                .map(|p| {
                    Evidence::file(
                        sources
                            .lock_files
                            .get(p)
                            .cloned()
                            // A lock this agent's own snapshot named but this
                            // second scan could not find (raced away between
                            // the two reads) still cites the leased path itself
                            // — real, even if the lock beside it is gone.
                            .unwrap_or_else(|| p.clone()),
                    )
                })
                .collect();
            if let Some(seen) = sources.agent_seen.get(agent)
                && !evidence.iter().any(|e| e.file == seen.source.file)
            {
                evidence.push(seen.source.clone());
            }
            Item {
                kind: item_kind(item),
                summary: format!(
                    "{agent} is dead and holds {} path(s): {} (lease TTL remaining {})",
                    paths.len(),
                    paths.join(", "),
                    dur::human(*remaining_ttl)
                ),
                follow_up: "pact lease ls".to_string(),
                evidence,
            }
        }
        state::AttentionItem::NeedsDecision { bead_id } => Item {
            kind: item_kind(item),
            summary: format!("{bead_id} needs a human decision"),
            follow_up: format!("bd show {bead_id}"),
            evidence: vec![Evidence {
                file: sources.sidecar_file.to_string(),
                line: sources.needs_decision_lines.get(bead_id).copied(),
            }],
        },
        state::AttentionItem::GateOrderViolation {
            started_id,
            started_wave,
            open_gate_id,
            gate_wave,
        } => {
            let mut evidence = vec![Evidence::file(sources.plan_file.to_string())];
            // S20: `recount explain --event-line N` when the events tail
            // actually names the line that started the work; `bd show` as the
            // honest fallback when it does not (a plan can flag a violation
            // for an id whose own start this file never located an `acquired`
            // row for).
            let follow_up = match sources.started.get(started_id) {
                Some(line) => {
                    evidence.push(Evidence {
                        file: sources.events_file.to_string(),
                        line: Some(*line),
                    });
                    format!("recount explain --event-line {line}")
                }
                None => format!("bd show {started_id}"),
            };
            Item {
                kind: item_kind(item),
                summary: format!(
                    "{started_id} (wave {started_wave}) started before gate {open_gate_id} (wave {gate_wave}) closed"
                ),
                follow_up,
                evidence,
            }
        }
    }
}

/// The single-repo pipeline: read, fold, judge, cite evidence. Pure aside from
/// the filesystem reads — `now` is a parameter rather than read internally via
/// `crate::now()`, the same choice `Tile::build` makes and for the same two
/// reasons: a tick (or here, one `why` answer) is a function of `(evidence,
/// clock)` and nothing else, and a unit test that wants a frozen clock can just
/// pass one instead of mutating the process-global `QUIVIVE_NOW` env var —
/// which, read internally, would make two of THIS FILE's own tests racy against
/// each other the moment `cargo test` runs them on different threads of the
/// same process (an earlier draft did exactly that and flaked). `run` below is
/// the one place that reads `crate::now()`, exactly once, matching
/// `main.rs::tick_once`.
pub fn build(repo: &Path, now: DateTime<Utc>) -> anyhow::Result<WhyOutput> {
    let repo_root = repo
        .canonicalize()
        .with_context(|| format!("--repo {}", repo.display()))?;
    anyhow::ensure!(
        repo_root.is_dir(),
        "{} is not a directory",
        repo_root.display()
    );
    let repo_display = repo_root.display().to_string();

    let state_dir = reader::state_dir(&repo_root);
    if !state_dir.is_dir() {
        // No pact here at all: `why` answers questions, it does not scold — S8's
        // own precedence chain already says this is `no-fleet` for an empty
        // snapshot, so ask it rather than hand-coding the status here too.
        let assessment = state::assess(
            &state::RepoSnapshot::default(),
            now,
            &state::Thresholds::default(),
        );
        return Ok(WhyOutput {
            repo: repo_display,
            status: assessment.status,
            items: Vec::new(),
        });
    }

    let ledger = reader::ledger::read(&state_dir, None);
    let leases = reader::lease::read(&state_dir);
    let activity = reader::activity::read(&state_dir);
    let plan = reader::plan::read(&state_dir);
    let sidecar = reader::sidecar::read(&repo_root);

    let lock_files = scan_lock_files(&state_dir, &repo_root);
    let agent_seen = merge_agents(
        &repo_root,
        &state_dir,
        &ledger.agents,
        &leases.leases,
        &activity.agents,
        &lock_files,
    );
    let started = scan_started(&state_dir);
    let (needs_decision_ids, needs_decision_lines) = needs_decision(&sidecar.rows);

    let snapshot = state::RepoSnapshot {
        agents: agent_seen
            .iter()
            .map(|(a, s)| (a.clone(), s.seen))
            .collect(),
        leases: leases
            .leases
            .iter()
            .map(|l| state::LeaseSnapshot {
                agent: l.agent.clone(),
                path: l.path.clone(),
                acquired_at: l.acquired_at,
                expires_at: l.expires_at,
            })
            .collect(),
        plan: plan
            .snapshot
            .as_ref()
            .map(|p| build_plan_snapshot(p, &sidecar.rows, &started)),
        needs_decision: needs_decision_ids,
        pact_present: true,
        // `why` cites the readers' evidence directly; whether a reader itself
        // degraded is `tile`'s field to report, not a fact this file's own
        // items need repeated in each citation.
        degraded: Vec::new(),
    };

    let thresholds = state::Thresholds::default();
    let assessment = state::assess(&snapshot, now, &thresholds);

    let sidecar_file = rel_or_abs(
        &repo_root,
        &repo_root.join(".beads").join(reader::sidecar::SIDECAR_FILE),
    );
    let plan_file = rel_or_abs(&repo_root, &state_dir.join(reader::plan::PLAN_FILE));
    let events_file = rel_or_abs(&repo_root, &state_dir.join(reader::ledger::LEDGER_FILE));

    let sources = EvidenceSources {
        agent_seen: &agent_seen,
        lock_files: &lock_files,
        needs_decision_lines: &needs_decision_lines,
        sidecar_file: &sidecar_file,
        plan_file: &plan_file,
        events_file: &events_file,
        started: &started,
    };
    let items = assessment
        .attention
        .iter()
        .map(|item| render_item(item, &sources))
        .collect();

    Ok(WhyOutput {
        repo: repo_display,
        status: assessment.status,
        items,
    })
}

/// List the attention items for `repo`, in JSON if `json` is set.
pub fn run(repo: &Path, json: bool) -> anyhow::Result<()> {
    let output = build(repo, crate::now()?)?;
    let mut out = std::io::stdout().lock();
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&output)?)?;
    } else {
        write!(out, "{}", output.text())?;
    }
    // Flush explicitly, matching `tick_once`: an ignored flush error on a
    // closed pipe is a broken-pipe panic on exit, not a silent no-op.
    let _ = out.flush();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// The frozen clock every test below passes to `why::build` directly —
    /// never the wall clock, or a dead-agent fixture would flap with wherever
    /// CI happens to be running relative to `NOW`.
    const NOW: &str = "2026-08-28T12:00:00Z";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(NOW).unwrap().to_utc()
    }

    fn stamp(secs_ago: i64) -> String {
        (now() - chrono::TimeDelta::seconds(secs_ago)).to_rfc3339()
    }

    /// A minimal repo fixture, written as pact and bd write their own files —
    /// this module's own tempdir builder, per quivive-15r's brief (not
    /// `tests/support::Fixture`, leased elsewhere this wave).
    struct Repo {
        dir: TempDir,
    }

    impl Repo {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join(".pact")).unwrap();
            std::fs::write(dir.path().join(".pact").join("events.jsonl"), "").unwrap();
            Self { dir }
        }

        fn bare() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn event_line(&self, kind: &str, agent: &str, bead: Option<&str>, secs_ago: i64) -> String {
            match bead {
                Some(b) => format!(
                    r#"{{"at":"{}","agent":"{agent}","kind":"{kind}","bead":"{b}"}}"#,
                    stamp(secs_ago)
                ),
                None => format!(
                    r#"{{"at":"{}","agent":"{agent}","kind":"{kind}"}}"#,
                    stamp(secs_ago)
                ),
            }
        }

        fn append_event(&self, line: &str) -> &Self {
            use std::io::Write as _;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.path().join(".pact").join("events.jsonl"))
                .unwrap();
            writeln!(f, "{line}").unwrap();
            self
        }

        fn lease(&self, agent: &str, path: &str, acquired_secs_ago: i64, ttl_secs: u64) -> &Self {
            let leases_dir = self.dir.path().join(".pact").join("leases");
            std::fs::create_dir_all(&leases_dir).unwrap();
            let lock = format!(
                r#"{{"agent":"{agent}","path":"{path}","acquired_at":"{}","ttl_secs":{ttl_secs}}}"#,
                stamp(acquired_secs_ago)
            );
            let name = format!("{}.lock", path.replace('/', "_"));
            std::fs::write(leases_dir.join(name), lock).unwrap();
            self
        }

        fn activity(&self, agent: &str, secs_ago: i64) -> &Self {
            let dir = self.dir.path().join(".pact").join("activity");
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(agent);
            std::fs::write(&path, stamp(secs_ago)).unwrap();
            // `reader::activity::read` takes `max(content, file mtime)` (see its
            // module doc). Left alone, the file's mtime is the REAL filesystem
            // clock at write time, not the frozen `now()` this fixture pretends
            // the record was written under — so the moment wall-clock drifts
            // past `NOW`, the real mtime silently outvotes a deliberately-stale
            // content timestamp and the agent reads as ACTIVE instead of DEAD
            // (quivive-jwp: this is exactly what made this file's tests
            // time-dependent). Pinning the mtime to the same instant the
            // content claims removes the real clock from the experiment
            // entirely: the two readings agree, matching the reader's own
            // documented "ordinary case" where they are written together.
            let target: std::time::SystemTime =
                (now() - chrono::TimeDelta::seconds(secs_ago)).into();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(target)
                .unwrap();
            self
        }

        fn plan(&self, json: &str) -> &Self {
            std::fs::write(self.dir.path().join(".pact").join("plan.json"), json).unwrap();
            self
        }

        fn sidecar_line(&self, line: &str) -> &Self {
            use std::io::Write as _;
            let dir = self.dir.path().join(".beads");
            std::fs::create_dir_all(&dir).unwrap();
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("interactions.jsonl"))
                .unwrap();
            writeln!(f, "{line}").unwrap();
            self
        }

        fn sidecar_row(
            &self,
            issue_id: &str,
            field: &str,
            new_value: &str,
            secs_ago: i64,
        ) -> &Self {
            self.sidecar_line(&format!(
                r#"{{"issue_id":"{issue_id}","kind":"field_change","actor":"human","created_at":"{}","extra":{{"field":"{field}","new_value":"{new_value}"}}}}"#,
                stamp(secs_ago)
            ))
        }
    }

    fn dead_age() -> i64 {
        state::Thresholds::default().dead.as_secs() as i64 + 1
    }

    /// `build` against the frozen clock, directly — `now` is a parameter, not
    /// an env var this test would have to mutate process-globally (see the doc
    /// comment on `build` for why that split exists).
    fn run_with_clock(repo: &Path) -> WhyOutput {
        build(repo, now()).expect("build must not fail on a well-formed fixture")
    }

    fn exists(root: &Path, evidence: &Evidence) -> bool {
        let p = PathBuf::from(&evidence.file);
        if p.is_absolute() {
            p.is_file() || p.is_dir()
        } else {
            let joined = root.join(&p);
            joined.is_file() || joined.is_dir()
        }
    }

    // --- no pact, and nothing attention-worthy -----------------------------

    #[test]
    fn a_repo_with_no_pact_reports_no_fleet_and_exits_clean() {
        let repo = Repo::bare();
        let out = run_with_clock(repo.root());
        assert_eq!(out.status, state::RepoStatus::NoFleet);
        assert!(out.items.is_empty());
        assert!(out.text().contains("status: no-fleet"));
        assert!(out.text().contains("nothing attention-worthy"));
    }

    #[test]
    fn a_quiet_pact_repo_has_nothing_attention_worthy() {
        let repo = Repo::new();
        let out = run_with_clock(repo.root());
        assert_eq!(out.status, state::RepoStatus::AllQuiet);
        assert!(out.items.is_empty());
        assert!(out.text().contains("nothing attention-worthy"));
    }

    // --- DeadHoldingPaths (S16) ---------------------------------------------

    #[test]
    fn a_dead_agent_holding_a_lease_cites_the_lock_file_and_its_newest_evidence() {
        let repo = Repo::new();
        repo.lease("agent-3", "src/a.rs", dead_age(), 300);
        repo.append_event(&repo.event_line("acquired", "agent-3", None, dead_age()));

        let out = run_with_clock(repo.root());
        assert_eq!(out.status, state::RepoStatus::HumanNeeded);
        assert_eq!(out.items.len(), 1);
        let item = &out.items[0];
        assert_eq!(item.kind, "dead_holding_paths");
        assert_eq!(item.follow_up, "pact lease ls");
        assert!(!item.evidence.is_empty());
        for ev in &item.evidence {
            assert!(
                exists(repo.root(), ev),
                "evidence {ev:?} must point at a real file"
            );
        }
        // The lock file itself is one of the citations.
        assert!(item.evidence.iter().any(|e| e.file.ends_with(".lock")));
    }

    #[test]
    fn a_dead_agents_newest_evidence_can_be_an_activity_record() {
        // A lease acquired long ago, and an activity record newer than it but
        // still past the dead window: the activity record must win the merge
        // and be the second citation, not the stale lease acquisition.
        let repo = Repo::new();
        repo.lease("agent-7", "src/b.rs", dead_age() + 500, 300);
        repo.activity("agent-7", dead_age());

        let out = run_with_clock(repo.root());
        assert_eq!(out.items.len(), 1);
        let item = &out.items[0];
        assert!(
            item.evidence.iter().any(|e| e.file.contains("activity")),
            "expected an activity citation in {:?}",
            item.evidence
        );
        for ev in &item.evidence {
            assert!(exists(repo.root(), ev));
        }
    }

    // --- NeedsDecision (S17) -------------------------------------------------

    #[test]
    fn a_needs_decision_bead_cites_its_sidecar_line() {
        let repo = Repo::new();
        repo.sidecar_row("proj-9", "type", "needs-decision", 60);

        let out = run_with_clock(repo.root());
        assert_eq!(out.status, state::RepoStatus::HumanNeeded);
        assert_eq!(out.items.len(), 1);
        let item = &out.items[0];
        assert_eq!(item.kind, "needs_decision");
        assert_eq!(item.follow_up, "bd show proj-9");
        assert_eq!(item.evidence.len(), 1);
        assert_eq!(item.evidence[0].line, Some(1));
        assert!(exists(repo.root(), &item.evidence[0]));
    }

    #[test]
    fn a_bead_flagged_twice_is_one_item_not_two() {
        let repo = Repo::new();
        repo.sidecar_row("proj-9", "type", "needs-decision", 120);
        repo.sidecar_row("proj-9", "type", "needs-decision", 30);

        let out = run_with_clock(repo.root());
        assert_eq!(out.items.len(), 1);
        // The FIRST row is the one that produced the fact.
        assert_eq!(out.items[0].evidence[0].line, Some(1));
    }

    // --- GateOrderViolation (S18) --------------------------------------------

    #[test]
    fn work_started_before_an_earlier_gate_closed_cites_plan_and_events() {
        let repo = Repo::new();
        repo.plan(
            r#"{"at":"2026-08-28T00:00:00Z",
                "edges":{"gate-1":[],"bead-9":["gate-1"]},
                "waves":{"gate-1":1,"bead-9":2},
                "gates":["gate-1"]}"#,
        );
        repo.append_event(&repo.event_line("acquired", "agent-1", Some("bead-9"), 10));

        let out = run_with_clock(repo.root());
        assert_eq!(out.status, state::RepoStatus::HumanNeeded);
        assert_eq!(out.items.len(), 1);
        let item = &out.items[0];
        assert_eq!(item.kind, "gate_order_violation");
        assert_eq!(item.follow_up, "recount explain --event-line 1");
        assert!(item.evidence.iter().any(|e| e.file.ends_with("plan.json")));
        assert!(
            item.evidence
                .iter()
                .any(|e| e.file.ends_with("events.jsonl") && e.line == Some(1))
        );
        for ev in &item.evidence {
            assert!(exists(repo.root(), ev));
        }
    }

    #[test]
    fn a_closed_gate_is_not_a_violation() {
        let repo = Repo::new();
        repo.plan(
            r#"{"at":"2026-08-28T00:00:00Z",
                "edges":{"gate-1":[],"bead-9":["gate-1"]},
                "waves":{"gate-1":1,"bead-9":2},
                "gates":["gate-1"]}"#,
        );
        repo.sidecar_row("gate-1", "status", "closed", 500);
        repo.append_event(&repo.event_line("acquired", "agent-1", Some("bead-9"), 10));

        let out = run_with_clock(repo.root());
        assert!(
            out.items.is_empty(),
            "closed gate must not violate: {out:?}"
        );
    }

    #[test]
    fn a_started_id_with_no_located_acquire_falls_back_to_bd_show() {
        let repo = Repo::new();
        // The plan alone can flag a violation-shaped wave arrangement without
        // this file ever finding an `acquired` row naming it — the fallback
        // path, exercised on its own.
        repo.plan(
            r#"{"at":"2026-08-28T00:00:00Z",
                "edges":{"gate-1":[]},
                "waves":{"gate-1":1},
                "gates":["gate-1"]}"#,
        );
        // No plan-declared "wave 2" id exists here, so assert the fallback
        // shape directly against `render_item` instead of round-tripping
        // through a fixture that has no way to leave `started` populated
        // without also being locatable.
        let no_agents: BTreeMap<String, AgentSeen> = BTreeMap::new();
        let no_locks: BTreeMap<String, String> = BTreeMap::new();
        let no_lines: BTreeMap<String, usize> = BTreeMap::new();
        let no_starts: BTreeMap<String, usize> = BTreeMap::new();
        let sources = EvidenceSources {
            agent_seen: &no_agents,
            lock_files: &no_locks,
            needs_decision_lines: &no_lines,
            sidecar_file: ".beads/interactions.jsonl",
            plan_file: ".pact/plan.json",
            events_file: ".pact/events.jsonl",
            started: &no_starts,
        };
        let item = render_item(
            &state::AttentionItem::GateOrderViolation {
                started_id: "ghost-9".to_string(),
                started_wave: 2,
                open_gate_id: "gate-1".to_string(),
                gate_wave: 1,
            },
            &sources,
        );
        assert_eq!(item.follow_up, "bd show ghost-9");
        assert_eq!(item.evidence.len(), 1);
        assert!(item.evidence[0].file.ends_with("plan.json"));
    }

    // --- --json stability ----------------------------------------------------

    #[test]
    fn json_output_is_stable_across_two_runs_under_a_frozen_clock() {
        let repo = Repo::new();
        repo.lease("agent-3", "src/a.rs", dead_age(), 300);
        repo.sidecar_row("proj-1", "type", "needs-decision", 60);
        repo.plan(
            r#"{"at":"2026-08-28T00:00:00Z",
                "edges":{"gate-1":[],"bead-9":["gate-1"]},
                "waves":{"gate-1":1,"bead-9":2},
                "gates":["gate-1"]}"#,
        );
        repo.append_event(&repo.event_line("acquired", "agent-1", Some("bead-9"), 10));

        let first = serde_json::to_string_pretty(&run_with_clock(repo.root())).unwrap();
        let second = serde_json::to_string_pretty(&run_with_clock(repo.root())).unwrap();
        assert_eq!(
            first, second,
            "two runs over unchanged evidence must agree byte-for-byte"
        );

        // And it actually parses back into the documented shape.
        let value: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert!(value["repo"].is_string());
        assert!(value["status"].is_string());
        assert!(value["items"].is_array());
        assert_eq!(value["items"].as_array().unwrap().len(), 3);
        for item in value["items"].as_array().unwrap() {
            assert!(item["kind"].is_string());
            assert!(item["summary"].is_string());
            assert!(item["follow_up"].is_string());
            assert!(item["evidence"].is_array());
            assert!(!item["evidence"].as_array().unwrap().is_empty());
        }
    }
}
