//! The per-agent state machine, and the thresholds that decide it.
//!
//! Every transition is driven by elapsed time or by new evidence and by nothing
//! else — there is no state that depends on how the previous tick was computed.
//! That is what makes a tick a pure function of (ledger, clock, thresholds), and
//! therefore what makes the goldens in `tests/goldens.rs` mean anything. See
//! `docs/spec.md#the-state-machine`.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::Serialize;

/// One state per agent per tick.
///
/// **Declaration order is severity order.** `Ord` is derived from it, so
/// `agents.iter().map(state).max()` is the tile's `worst` — there is no second
/// list of severities anywhere, which matters because nothing in the tile's JSON
/// shape encodes this ordering. That is exactly why moving it is invisible in a
/// diff and has to be a deliberate, breaking decision
/// (`docs/tile-contract.md`).
///
/// The `ValueEnum` derive is load-bearing beyond `--exit-on`: it makes clap
/// render the accepted values from this enum, so `--help` cannot offer a state
/// the parser rejects or omit one it accepts. A hand-written list in a doc
/// comment is the drift the `cli-surface-auditor` role exists to catch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lowercase")]
pub enum State {
    /// Evidence within `--active-window`: working.
    Active,
    /// Quiet, but recently loud: thinking, compiling, or between beads. Normal.
    Idle,
    /// Quiet longer than an agent usually is. Worth a look.
    Stale,
    /// Quiet past `--dead-window`, or holding an expired lease with nothing
    /// since. Gone — and if it holds a lease, that lease is blocking somebody.
    Dead,
}

impl State {
    /// The name used in the tile, in `--exit-on`, and in `worst`. One spelling,
    /// in one place: a renderer keys colour off these strings.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Active => "active",
            State::Idle => "idle",
            State::Stale => "stale",
            State::Dead => "dead",
        }
    }

    /// The single letter the text tile counts with: `5A 2I 1S 0D`.
    pub fn initial(self) -> char {
        match self {
            State::Active => 'A',
            State::Idle => 'I',
            State::Stale => 'S',
            State::Dead => 'D',
        }
    }

    /// A lease held by one of these is reported as blocking: the holder is not
    /// coming back to release it on its own.
    pub fn is_blocking(self) -> bool {
        matches!(self, State::Stale | State::Dead)
    }
}

/// The three windows, plus the sweep.
///
/// Defaults are a starting point and nothing more. `Dead` is the one state
/// anybody acts on, so the window behind it is genuinely the user's business: a
/// fleet whose beads take an hour needs a different `--dead-window` than one
/// whose beads take a minute, and no default is right for both.
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub active: Duration,
    pub idle: Duration,
    pub dead: Duration,
    /// An agent nobody has heard from in this long stops occupying space in the
    /// tile. Bookkeeping, not a state: without it a week-old repository renders
    /// forty dead names on a bar with room for one line.
    pub forget: Duration,
}

/// The default windows, as the strings a user would type.
///
/// **These four consts are the only place a default lives.** `clap` renders them
/// into `--help` via `default_value`, and [`Thresholds::default`] parses them —
/// so the help text, the parser and the library cannot disagree.
///
/// They were two sets of literals in the first draft: `Duration::from_secs(60)`
/// here and `default_value = "60s"` in `cli.rs`. Nothing connected them, so
/// changing one would have left the other quietly describing the old behaviour —
/// which is precisely the drift `.claude/agents/cli-surface-auditor.md` exists to
/// hunt. Writing that role down is what exposed it.
pub const ACTIVE_DEFAULT: &str = "60s";
pub const IDLE_DEFAULT: &str = "5m";
pub const DEAD_DEFAULT: &str = "30m";
pub const FORGET_DEFAULT: &str = "1h";

impl Default for Thresholds {
    /// Parsed from the consts above. The `expect`s are unreachable for any value
    /// that compiles, and `defaults_parse_and_are_ordered` is what keeps that
    /// true — a const edited to something `dur::parse` rejects fails that test
    /// rather than panicking in front of a user.
    fn default() -> Self {
        let p = |s: &str| crate::dur::parse(s).expect("a default window must parse");
        Self {
            active: p(ACTIVE_DEFAULT),
            idle: p(IDLE_DEFAULT),
            dead: p(DEAD_DEFAULT),
            forget: p(FORGET_DEFAULT),
        }
    }
}

impl Thresholds {
    /// Ascending, or the machine is nonsense: an `--idle-window` shorter than
    /// `--active-window` describes a state that cannot be entered, and silently
    /// producing a tile from an impossible configuration is worse than refusing.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.active >= self.idle {
            anyhow::bail!(
                "--active-window ({:?}) must be shorter than --idle-window ({:?})",
                self.active,
                self.idle
            );
        }
        if self.idle >= self.dead {
            anyhow::bail!(
                "--idle-window ({:?}) must be shorter than --dead-window ({:?})",
                self.idle,
                self.dead
            );
        }
        Ok(())
    }

    /// Classify by the age of the newest evidence, in seconds.
    ///
    /// Recovery is always direct to `Active`: an agent that leaves evidence is
    /// alive, and there is no convalescent state requiring two ticks to leave.
    /// Hysteresis would be steadier on a flapping fleet and would also make the
    /// tile depend on tick *history*, which
    /// `docs/adr/0001-stream-first-tile.md` forbids.
    pub fn classify(&self, age_secs: i64) -> State {
        let age = age_secs.max(0) as u64;
        if age < self.active.as_secs() {
            State::Active
        } else if age < self.idle.as_secs() {
            State::Idle
        } else if age < self.dead.as_secs() {
            State::Stale
        } else {
            State::Dead
        }
    }
}

// ---------------------------------------------------------------------------
// The seam: S7's per-agent machine above feeds S8's per-repo status and
// S14-S20's transition events. Everything below is pure — data structures in,
// a judgment out, no disk read anywhere in this file. Readers (`src/reader/`)
// build a `RepoSnapshot` from what they read; `assess` turns it into a
// `RepoAssessment`; `transitions` diffs two assessments. Nothing here knows
// how a `RepoSnapshot` was gathered, and nothing that gathers one needs to
// know how it is judged.
// ---------------------------------------------------------------------------

/// A mirror of `reader::lease::Lease`, narrowed to the fields this file's
/// logic uses.
///
/// This is a *mirror*, not the same type, and that is deliberate rather than
/// an oversight: `reader` already depends on nothing in `state` today, and
/// this file's whole value is being disk-I/O-free and independently testable
/// (see the module doc above `State`). Importing `reader::lease::Lease` here
/// would point that edge the wrong way — the first pull toward a cycle the
/// moment a reader ever wants a judgment type back — in exchange for not
/// retyping four field names. The duplication is the cheaper of the two
/// costs.
#[derive(Debug, Clone)]
pub struct LeaseSnapshot {
    pub agent: String,
    /// Repo-relative, as the lock file itself recorded it.
    pub path: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// One declared gate in `.pact/plan.json`: a wave does not begin for real
/// until every gate on the wave before it has closed.
#[derive(Debug, Clone)]
pub struct GateSnapshot {
    /// The gate's own id — usually a bead id — carried verbatim into
    /// [`AttentionItem::GateOrderViolation::open_gate_id`] so `bd show
    /// <id>` (S20) is the follow-up command without any further lookup.
    pub id: String,
    pub closed: bool,
}

/// One wave of `.pact/plan.json`, plus what the events tail says has already
/// started in it. The gates and the "started" list live on the same struct
/// because S18's violation is a relationship between the two, and a caller
/// building one without the other has not finished reading the plan.
#[derive(Debug, Clone)]
pub struct WaveSnapshot {
    pub wave: u32,
    /// Declared gates for this wave. Empty means the wave has none — not the
    /// same as "closed", which is why this is a list of [`GateSnapshot`] and
    /// not a bool.
    pub gates: Vec<GateSnapshot>,
    /// Ids (bead ids, in practice) of work the events tail shows as started
    /// in this wave, regardless of whether that start was in order.
    pub started: Vec<String>,
}

/// `.pact/plan.json`, narrowed to what S18 needs: the waves, in any order,
/// each with its gates and what has started in it.
#[derive(Debug, Clone, Default)]
pub struct PlanSnapshot {
    pub waves: Vec<WaveSnapshot>,
}

/// Everything one tick learned about one repo: the three readers' output,
/// merged, plus the plan and the sidecar. Pure data — [`assess`] is the only
/// thing in this file that interprets it, and this struct is buildable by a
/// reader that has never heard of `assess`.
///
/// `agents` matches `reader::Readings::agents` in type exactly (both a
/// `BTreeMap<String, DateTime<Utc>>` of newest evidence per agent), so
/// building one from the other is a move, not a translation. The lease list
/// is a mirror rather than a shared type; see [`LeaseSnapshot`] for why that
/// split is not the same call both times.
#[derive(Debug, Clone, Default)]
pub struct RepoSnapshot {
    /// Newest evidence per agent, every source already merged.
    pub agents: BTreeMap<String, DateTime<Utc>>,
    pub leases: Vec<LeaseSnapshot>,
    /// `None` when there is no `.pact/plan.json` for this repo. A fleet with
    /// no plan is not a broken one — just an undirected one.
    pub plan: Option<PlanSnapshot>,
    /// Bead ids the committed sidecar (`.beads/interactions.jsonl`) flags as
    /// needing a human decision (S17). Deciding *which* sidecar rows mean
    /// that is the sidecar reader's job; by the time it reaches here it is
    /// just ids.
    pub needs_decision: Vec<String>,
    /// Whether `.pact/` exists for this repo at all. Kept separate from
    /// `agents` being empty: a freshly-initialised pact with nobody having
    /// leased anything yet is `pact_present` and reads as `all-quiet`, not
    /// `no-fleet` — S8 draws that line and this field is what lets it.
    pub pact_present: bool,
    /// Readers that could not read this tick, named exactly as the tile's own
    /// `degraded` field wants them (`reader::Readings::degraded`). Carried
    /// here, rather than threaded past this struct separately, so that
    /// "everything one tick learned about one repo" — this struct's own
    /// promise — is actually everything.
    pub degraded: Vec<String>,
}

/// S8's derived per-repo status, in the precedence order [`assess`] applies.
///
/// Declaration order is **not** severity here, unlike [`State`]: nothing in
/// this crate sorts repos by `RepoStatus`, so there is no `Ord` above to keep
/// honest and none is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepoStatus {
    HumanNeeded,
    Active,
    Drained,
    AllQuiet,
    NoFleet,
}

/// The four watch events of S16-S19, minus S19: see the comment on
/// [`derive_status`] for why `FleetDrained` is not a fourth variant here and
/// lives on [`TransitionEvent`] instead.
///
/// Each variant carries exactly what S20's follow-up command needs and
/// nothing else — `DeadHoldingPaths` names the agent, the paths and the
/// remaining TTL (S16 verbatim), `NeedsDecision` names the bead for `bd show`,
/// `GateOrderViolation` names the started work and the specific gate still
/// blocking it. Formatting the command itself is `watch`'s job (quivive-8mq);
/// this file only guarantees the payload is there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttentionItem {
    /// S16. Grouped by holder, not one item per lease: a dead agent sitting
    /// on three files is one fact for a human to act on, not three.
    DeadHoldingPaths {
        agent: String,
        /// Repo-relative, sorted, so two assessments over the same evidence
        /// agree byte-for-byte regardless of read order.
        paths: Vec<String>,
        /// Seconds until pact's sweeper reclaims the *most urgent* of the
        /// held leases — the minimum across them, clamped at 0 rather than
        /// left negative. "How overdue" is a different question, already
        /// answered by `tile::BlockedLease::expired_s`; this field answers
        /// "how long until this becomes worse."
        remaining_ttl: i64,
    },
    /// S17. A bead the committed sidecar flags as needing a human decision.
    NeedsDecision { bead_id: String },
    /// S18. `started_id` (in `started_wave`) began before `open_gate_id` (in
    /// the earlier `gate_wave`) closed. When more than one earlier gate is
    /// still open, only the earliest-wave one is reported — the one actually
    /// blocking `started_id` right now — so closing gates one at a time does
    /// not make the *set* of violations change shape and refire on every
    /// close.
    GateOrderViolation {
        started_id: String,
        started_wave: u32,
        open_gate_id: String,
        gate_wave: u32,
    },
}

impl AttentionItem {
    /// The identity of the *condition*, deliberately narrower than the full
    /// payload: `remaining_ttl` counts down every tick a dead agent stays
    /// dead, so comparing whole values would make [`transitions`] see a
    /// "new" `DeadHoldingPaths` every single tick and violate S14's "becomes
    /// true, not while it stays true." Two items with the same identity
    /// describe the same standing fact regardless of which tick produced
    /// them.
    fn identity(&self) -> (u8, &str, &str) {
        match self {
            AttentionItem::DeadHoldingPaths { agent, .. } => (0, agent.as_str(), ""),
            AttentionItem::NeedsDecision { bead_id } => (1, bead_id.as_str(), ""),
            AttentionItem::GateOrderViolation {
                started_id,
                open_gate_id,
                ..
            } => (2, started_id.as_str(), open_gate_id.as_str()),
        }
    }
}

/// One tick's judgment for one repo: the per-agent states, the S8 status they
/// (and the plan, and the sidecar) derive, and the S16-S18 items that make up
/// "human-needed." Built by [`assess`] from a [`RepoSnapshot`]; two of these
/// are what [`transitions`] compares.
#[derive(Debug, Clone)]
pub struct RepoAssessment {
    pub agents: BTreeMap<String, State>,
    pub status: RepoStatus,
    /// Sorted by [`AttentionItem::identity`], so two assessments built from
    /// equivalent evidence agree on order and `transitions` never has to
    /// search unordered data.
    pub attention: Vec<AttentionItem>,
}

/// S16: fold `snapshot.leases` by holder and flag every holder this tick
/// classifies as [`State::Dead`].
///
/// A holder absent from `agents` entirely is also treated as dead — not
/// skipped — because the only way a lock file names an agent no reader
/// produced evidence for is that agent going quieter than merely stale.
/// `tile::Tile::build`'s `blocked_leases` makes the identical defensive call
/// for the identical reason.
fn dead_holding_paths(
    snapshot: &RepoSnapshot,
    agents: &BTreeMap<String, State>,
    now: DateTime<Utc>,
) -> Vec<AttentionItem> {
    let mut by_agent: BTreeMap<&str, Vec<&LeaseSnapshot>> = BTreeMap::new();
    for lease in &snapshot.leases {
        by_agent
            .entry(lease.agent.as_str())
            .or_default()
            .push(lease);
    }

    by_agent
        .into_iter()
        .filter(|(agent, _)| agents.get(*agent).copied().unwrap_or(State::Dead) == State::Dead)
        .map(|(agent, leases)| {
            let mut paths: Vec<String> = leases.iter().map(|l| l.path.clone()).collect();
            paths.sort();
            let remaining_ttl = leases
                .iter()
                .map(|l| (l.expires_at.timestamp() - now.timestamp()).max(0))
                .min()
                .unwrap_or(0);
            AttentionItem::DeadHoldingPaths {
                agent: agent.to_string(),
                paths,
                remaining_ttl,
            }
        })
        .collect()
}

/// S18: for every id the events tail shows as started in some wave, find the
/// earliest-wave gate still open among all waves declared *before* it, and
/// report exactly that one — see [`AttentionItem::GateOrderViolation`] for
/// why only the earliest.
fn gate_order_violations(snapshot: &RepoSnapshot) -> Vec<AttentionItem> {
    let Some(plan) = &snapshot.plan else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for wave in &plan.waves {
        for started_id in &wave.started {
            let blocking = plan
                .waves
                .iter()
                .filter(|earlier| earlier.wave < wave.wave)
                .flat_map(|earlier| earlier.gates.iter().map(move |g| (earlier.wave, g)))
                .filter(|(_, g)| !g.closed)
                .min_by_key(|(w, _)| *w);
            if let Some((gate_wave, gate)) = blocking {
                out.push(AttentionItem::GateOrderViolation {
                    started_id: started_id.clone(),
                    started_wave: wave.wave,
                    open_gate_id: gate.id.clone(),
                    gate_wave,
                });
            }
        }
    }
    out
}

/// S8, as one linear precedence chain and nothing else: no intermediate
/// boolean stored anywhere else in this file, no second place this order
/// could quietly be re-derived differently. `status_precedence_is_total_*`
/// below is what keeps this exhaustive as the type it returns grows.
///
/// S19's "fleet drained" is deliberately **not** folded into `attention`,
/// even though S8's own prose cites "S16-S19" for `human-needed`: S19
/// defines the event as "S8's `drained` became true" — a transition, not a
/// level — while `attention` here is a fresh read of the *current* tick.
/// Counting a drained fleet as a standing attention item would make the
/// `RepoStatus::Drained` arm below unreachable, which contradicts S8's own
/// fifth line naming it as a real, distinct outcome. `transitions` fires
/// [`TransitionEvent::FleetDrained`] off `status` becoming `Drained`
/// directly instead of off anything in `attention` — see there.
fn derive_status(
    snapshot: &RepoSnapshot,
    agents: &BTreeMap<String, State>,
    attention: &[AttentionItem],
) -> RepoStatus {
    let any_live = agents
        .values()
        .any(|s| matches!(s, State::Active | State::Idle));
    let fleet_evidence = snapshot.plan.is_some() || !agents.is_empty();

    if !attention.is_empty() {
        RepoStatus::HumanNeeded
    } else if any_live {
        RepoStatus::Active
    } else if fleet_evidence {
        RepoStatus::Drained
    } else if snapshot.pact_present {
        RepoStatus::AllQuiet
    } else {
        RepoStatus::NoFleet
    }
}

/// Classify every agent, derive every S16-S18 attention item, and derive S8's
/// status from both. The only place a `RepoSnapshot` becomes a judgment.
pub fn assess(
    snapshot: &RepoSnapshot,
    now: DateTime<Utc>,
    thresholds: &Thresholds,
) -> RepoAssessment {
    let agents: BTreeMap<String, State> = snapshot
        .agents
        .iter()
        .map(|(id, seen)| {
            let age_s = now.timestamp() - seen.timestamp();
            (id.clone(), thresholds.classify(age_s))
        })
        .collect();

    let mut attention = dead_holding_paths(snapshot, &agents, now);
    attention.extend(
        snapshot
            .needs_decision
            .iter()
            .cloned()
            .map(|bead_id| AttentionItem::NeedsDecision { bead_id }),
    );
    attention.extend(gate_order_violations(snapshot));
    attention.sort_by(|a, b| a.identity().cmp(&b.identity()));

    let status = derive_status(snapshot, &agents, &attention);

    RepoAssessment {
        agents,
        status,
        attention,
    }
}

/// The four watch events of S14-S20. `Attention` covers S16-S18 by wrapping
/// whichever [`AttentionItem`] just became true; `FleetDrained` is S19 and
/// stands alone because it is not a member of `attention` — see the comment
/// on [`derive_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionEvent {
    Attention(AttentionItem),
    FleetDrained,
}

/// S14's whole contract, as one pure function: an event fires when it
/// **becomes** true between `prev` and `now`, never while it merely stays
/// true. `watch` (quivive-8mq) calls this once per tick per repo and is a
/// thin shell around it — everything that decides *whether* to notify lives
/// here, where it can be tested without `notify-send`, a clock, or a debounce
/// timer anywhere in sight.
pub fn transitions(prev: &RepoAssessment, now: &RepoAssessment) -> Vec<TransitionEvent> {
    let mut events: Vec<TransitionEvent> = now
        .attention
        .iter()
        .filter(|item| {
            !prev
                .attention
                .iter()
                .any(|p| p.identity() == item.identity())
        })
        .cloned()
        .map(TransitionEvent::Attention)
        .collect();

    if now.status == RepoStatus::Drained && prev.status != RepoStatus::Drained {
        events.push(TransitionEvent::FleetDrained);
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_are_exclusive_at_the_bottom_of_each_state() {
        let t = Thresholds::default();
        assert_eq!(t.classify(0), State::Active);
        assert_eq!(t.classify(59), State::Active);
        assert_eq!(t.classify(60), State::Idle);
        assert_eq!(t.classify(299), State::Idle);
        assert_eq!(t.classify(300), State::Stale);
        assert_eq!(t.classify(1799), State::Stale);
        assert_eq!(t.classify(1800), State::Dead);
    }

    #[test]
    fn a_clock_that_went_backwards_reads_as_active_not_as_dead() {
        // Negative age means the newest evidence is in the future relative to
        // this tick. Clamping to 0 reports ACTIVE, which is the safe direction:
        // the alternative arithmetic makes a live agent look long dead.
        assert_eq!(Thresholds::default().classify(-5000), State::Active);
    }

    #[test]
    fn severity_order_comes_from_the_declaration_and_not_from_a_second_list() {
        assert!(State::Dead > State::Stale);
        assert!(State::Stale > State::Idle);
        assert!(State::Idle > State::Active);
    }

    #[test]
    fn only_stale_and_dead_block_a_lease() {
        assert!(!State::Active.is_blocking());
        assert!(!State::Idle.is_blocking());
        assert!(State::Stale.is_blocking());
        assert!(State::Dead.is_blocking());
    }

    #[test]
    fn defaults_parse_and_are_ordered() {
        // The consts are strings, so a typo in one is a runtime panic rather than
        // a compile error. This is the gate that makes that impossible to ship.
        let t = Thresholds::default();
        assert_eq!(t.active, Duration::from_secs(60));
        assert_eq!(t.idle, Duration::from_secs(300));
        assert_eq!(t.dead, Duration::from_secs(1800));
        assert_eq!(t.forget, Duration::from_secs(3600));
        assert!(t.validate().is_ok());
        // The sweep must be no shorter than the dead window, or an agent would be
        // forgotten before it could ever be reported dead.
        assert!(t.forget >= t.dead);
    }

    #[test]
    fn windows_out_of_order_are_refused_rather_than_silently_folded() {
        let mut t = Thresholds::default();
        t.idle = t.active;
        assert!(t.validate().is_err());
        let mut t = Thresholds::default();
        t.dead = t.idle;
        assert!(t.validate().is_err());
        assert!(Thresholds::default().validate().is_ok());
    }

    // -----------------------------------------------------------------------
    // The seam: RepoSnapshot -> assess -> RepoAssessment, and transitions
    // between two of the latter.
    // -----------------------------------------------------------------------

    /// A frozen tick clock. Every test below is a pure function of (snapshot,
    /// this clock, thresholds) — no `Utc::now()` anywhere, or a flaky test is
    /// the least of what that costs.
    fn clock() -> DateTime<Utc> {
        "2026-08-28T12:00:00Z".parse().unwrap()
    }

    /// Evidence `age_secs` old, relative to [`clock`].
    fn seen(age_secs: i64) -> DateTime<Utc> {
        clock() - chrono::TimeDelta::seconds(age_secs)
    }

    fn dead_age() -> i64 {
        Thresholds::default().dead.as_secs() as i64 + 1
    }

    // --- S8 status precedence ------------------------------------------

    #[test]
    fn status_precedence_is_total_over_every_combination() {
        // The four independent knobs behind `derive_status`, enumerated by
        // hand instead of reached for `proptest` — outside the dependency
        // budget in Cargo.toml. `Agent` has three settings rather than a
        // plain bool because "any live agent" and "some fleet evidence" stop
        // being independent the moment an agent exists at all: a live agent
        // is automatically fleet evidence too, so a boolean would silently
        // skip the (live, no-evidence) combination that cannot occur.
        #[derive(Clone, Copy, Debug)]
        enum Agent {
            None,
            Live,
            DeadOnly,
        }

        let thresholds = Thresholds::default();
        for attention in [false, true] {
            for agent in [Agent::None, Agent::Live, Agent::DeadOnly] {
                for plan in [false, true] {
                    for pact_present in [false, true] {
                        let mut snap = RepoSnapshot {
                            pact_present,
                            ..RepoSnapshot::default()
                        };
                        if attention {
                            snap.needs_decision.push("bd-attn".to_string());
                        }
                        match agent {
                            Agent::None => {}
                            Agent::Live => {
                                snap.agents.insert("a1".to_string(), seen(0));
                            }
                            Agent::DeadOnly => {
                                snap.agents.insert("a1".to_string(), seen(dead_age()));
                            }
                        }
                        if plan {
                            snap.plan = Some(PlanSnapshot::default());
                        }

                        let assessment = assess(&snap, clock(), &thresholds);

                        let any_live = matches!(agent, Agent::Live);
                        let fleet_evidence = plan || !matches!(agent, Agent::None);
                        let expected = if attention {
                            RepoStatus::HumanNeeded
                        } else if any_live {
                            RepoStatus::Active
                        } else if fleet_evidence {
                            RepoStatus::Drained
                        } else if pact_present {
                            RepoStatus::AllQuiet
                        } else {
                            RepoStatus::NoFleet
                        };
                        assert_eq!(
                            assessment.status, expected,
                            "attention={attention} agent={agent:?} plan={plan} pact_present={pact_present}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn human_needed_outranks_an_active_fleet() {
        let mut snap = RepoSnapshot::default();
        snap.agents.insert("a1".to_string(), seen(0));
        snap.needs_decision.push("bd-1".to_string());
        assert_eq!(
            assess(&snap, clock(), &Thresholds::default()).status,
            RepoStatus::HumanNeeded
        );
    }

    #[test]
    fn active_outranks_drained_and_all_quiet() {
        let mut snap = RepoSnapshot {
            pact_present: true,
            ..RepoSnapshot::default()
        };
        snap.agents.insert("a1".to_string(), seen(0));
        snap.plan = Some(PlanSnapshot::default());
        assert_eq!(
            assess(&snap, clock(), &Thresholds::default()).status,
            RepoStatus::Active
        );
    }

    #[test]
    fn drained_is_reached_by_either_a_dead_agent_or_a_plan_with_nobody_ever_seen() {
        let thresholds = Thresholds::default();

        let mut by_dead_agent = RepoSnapshot::default();
        by_dead_agent
            .agents
            .insert("a1".to_string(), seen(dead_age()));
        assert_eq!(
            assess(&by_dead_agent, clock(), &thresholds).status,
            RepoStatus::Drained
        );

        let by_plan = RepoSnapshot {
            plan: Some(PlanSnapshot::default()),
            ..RepoSnapshot::default()
        };
        assert_eq!(
            assess(&by_plan, clock(), &thresholds).status,
            RepoStatus::Drained
        );
    }

    #[test]
    fn all_quiet_is_pact_present_with_no_plan_and_no_agent_ever_seen() {
        let snap = RepoSnapshot {
            pact_present: true,
            ..RepoSnapshot::default()
        };
        assert_eq!(
            assess(&snap, clock(), &Thresholds::default()).status,
            RepoStatus::AllQuiet
        );
    }

    #[test]
    fn no_fleet_is_the_floor() {
        assert_eq!(
            assess(&RepoSnapshot::default(), clock(), &Thresholds::default()).status,
            RepoStatus::NoFleet
        );
    }

    // --- S16: DeadHoldingPaths -------------------------------------------

    #[test]
    fn dead_agent_with_an_expired_lease_and_a_live_lease_reports_the_more_urgent_ttl() {
        let thresholds = Thresholds::default();
        let mut snap = RepoSnapshot::default();
        snap.agents.insert("agent-3".to_string(), seen(dead_age()));
        snap.leases.push(LeaseSnapshot {
            agent: "agent-3".to_string(),
            path: "src/b.rs".to_string(),
            acquired_at: seen(dead_age()),
            // Already 30s past expiry.
            expires_at: clock() - chrono::TimeDelta::seconds(30),
        });
        snap.leases.push(LeaseSnapshot {
            agent: "agent-3".to_string(),
            path: "src/a.rs".to_string(),
            acquired_at: seen(dead_age()),
            // Two minutes of TTL still left.
            expires_at: clock() + chrono::TimeDelta::seconds(120),
        });

        let assessment = assess(&snap, clock(), &thresholds);
        let item = assessment
            .attention
            .iter()
            .find(|a| matches!(a, AttentionItem::DeadHoldingPaths { .. }))
            .expect("a dead agent holding leases must be flagged");
        assert_eq!(
            *item,
            AttentionItem::DeadHoldingPaths {
                agent: "agent-3".to_string(),
                // Sorted, and independent of the order the leases were pushed in.
                paths: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
                // The expired lease is the more urgent of the two, and clamps
                // at 0 rather than reporting a negative TTL.
                remaining_ttl: 0,
            }
        );
    }

    #[test]
    fn dead_agent_holding_only_live_leases_still_flags_with_a_positive_ttl() {
        let thresholds = Thresholds::default();
        let mut snap = RepoSnapshot::default();
        snap.agents.insert("agent-9".to_string(), seen(dead_age()));
        snap.leases.push(LeaseSnapshot {
            agent: "agent-9".to_string(),
            path: "src/c.rs".to_string(),
            acquired_at: seen(dead_age()),
            expires_at: clock() + chrono::TimeDelta::seconds(300),
        });

        let assessment = assess(&snap, clock(), &thresholds);
        assert_eq!(
            assessment.attention,
            vec![AttentionItem::DeadHoldingPaths {
                agent: "agent-9".to_string(),
                paths: vec!["src/c.rs".to_string()],
                remaining_ttl: 300,
            }]
        );
    }

    #[test]
    fn a_live_agents_leases_are_not_dead_holding_paths() {
        let mut snap = RepoSnapshot::default();
        snap.agents.insert("agent-1".to_string(), seen(0));
        snap.leases.push(LeaseSnapshot {
            agent: "agent-1".to_string(),
            path: "src/state.rs".to_string(),
            acquired_at: seen(0),
            expires_at: clock() + chrono::TimeDelta::seconds(600),
        });
        assert!(
            assess(&snap, clock(), &Thresholds::default())
                .attention
                .is_empty()
        );
    }

    #[test]
    fn a_lease_naming_an_agent_absent_from_this_ticks_evidence_still_flags() {
        // The only way a lock file names an agent no reader produced evidence
        // for is that agent having gone quieter than merely stale.
        let mut snap = RepoSnapshot {
            pact_present: true,
            ..RepoSnapshot::default()
        };
        snap.leases.push(LeaseSnapshot {
            agent: "ghost".to_string(),
            path: "src/x.rs".to_string(),
            acquired_at: seen(0),
            expires_at: clock() + chrono::TimeDelta::seconds(60),
        });
        let assessment = assess(&snap, clock(), &Thresholds::default());
        assert!(assessment.attention.iter().any(
            |a| matches!(a, AttentionItem::DeadHoldingPaths { agent, .. } if agent == "ghost")
        ));
    }

    // --- S17: NeedsDecision ------------------------------------------------

    #[test]
    fn needs_decision_bead_ids_become_attention_items() {
        let mut snap = RepoSnapshot::default();
        snap.needs_decision.push("bd-42".to_string());
        let assessment = assess(&snap, clock(), &Thresholds::default());
        assert_eq!(
            assessment.attention,
            vec![AttentionItem::NeedsDecision {
                bead_id: "bd-42".to_string()
            }]
        );
    }

    // --- S18: GateOrderViolation --------------------------------------------

    #[test]
    fn work_started_before_an_earlier_waves_gate_closed_is_a_violation() {
        let snap = RepoSnapshot {
            plan: Some(PlanSnapshot {
                waves: vec![
                    WaveSnapshot {
                        wave: 1,
                        gates: vec![GateSnapshot {
                            id: "gate-1".to_string(),
                            closed: false,
                        }],
                        started: vec![],
                    },
                    WaveSnapshot {
                        wave: 2,
                        gates: vec![],
                        started: vec!["bead-9".to_string()],
                    },
                ],
            }),
            ..RepoSnapshot::default()
        };

        let assessment = assess(&snap, clock(), &Thresholds::default());
        assert_eq!(
            assessment.attention,
            vec![AttentionItem::GateOrderViolation {
                started_id: "bead-9".to_string(),
                started_wave: 2,
                open_gate_id: "gate-1".to_string(),
                gate_wave: 1,
            }]
        );
    }

    #[test]
    fn work_started_after_its_gates_closed_is_not_a_violation() {
        let snap = RepoSnapshot {
            plan: Some(PlanSnapshot {
                waves: vec![
                    WaveSnapshot {
                        wave: 1,
                        gates: vec![GateSnapshot {
                            id: "gate-1".to_string(),
                            closed: true,
                        }],
                        started: vec![],
                    },
                    WaveSnapshot {
                        wave: 2,
                        gates: vec![],
                        started: vec!["bead-9".to_string()],
                    },
                ],
            }),
            ..RepoSnapshot::default()
        };

        assert!(
            assess(&snap, clock(), &Thresholds::default())
                .attention
                .is_empty()
        );
    }

    #[test]
    fn only_the_earliest_open_gate_is_reported_for_one_started_item() {
        let snap = RepoSnapshot {
            plan: Some(PlanSnapshot {
                waves: vec![
                    WaveSnapshot {
                        wave: 1,
                        gates: vec![GateSnapshot {
                            id: "gate-1".to_string(),
                            closed: false,
                        }],
                        started: vec![],
                    },
                    WaveSnapshot {
                        wave: 2,
                        gates: vec![GateSnapshot {
                            id: "gate-2".to_string(),
                            closed: false,
                        }],
                        started: vec![],
                    },
                    WaveSnapshot {
                        wave: 3,
                        gates: vec![],
                        started: vec!["bead-9".to_string()],
                    },
                ],
            }),
            ..RepoSnapshot::default()
        };

        let assessment = assess(&snap, clock(), &Thresholds::default());
        assert_eq!(
            assessment.attention,
            vec![AttentionItem::GateOrderViolation {
                started_id: "bead-9".to_string(),
                started_wave: 3,
                open_gate_id: "gate-1".to_string(),
                gate_wave: 1,
            }]
        );
    }

    // --- S14: becoming-true-only -------------------------------------------

    #[test]
    fn identical_assessments_produce_no_transitions() {
        let mut snap = RepoSnapshot::default();
        snap.needs_decision.push("bd-1".to_string());
        let assessment = assess(&snap, clock(), &Thresholds::default());
        assert!(transitions(&assessment, &assessment).is_empty());
    }

    #[test]
    fn an_attention_item_present_in_both_ticks_does_not_repeat_as_its_ttl_counts_down() {
        let thresholds = Thresholds::default();
        let mut snap = RepoSnapshot::default();
        snap.agents.insert("agent-3".to_string(), seen(dead_age()));
        snap.leases.push(LeaseSnapshot {
            agent: "agent-3".to_string(),
            path: "src/a.rs".to_string(),
            acquired_at: seen(dead_age()),
            expires_at: clock() + chrono::TimeDelta::seconds(300),
        });

        let prev = assess(&snap, clock(), &thresholds);
        // One tick later: the same dead-holding-paths fact, but the payload's
        // own remaining_ttl has counted down — it must not look new.
        let later_clock = clock() + chrono::TimeDelta::seconds(60);
        let later = assess(&snap, later_clock, &thresholds);
        assert_ne!(
            prev.attention, later.attention,
            "the fixture should actually move"
        );
        assert!(
            transitions(&prev, &later).is_empty(),
            "a counting-down TTL on the same condition must not refire it"
        );
    }

    #[test]
    fn a_new_attention_item_fires_exactly_once() {
        let empty = assess(&RepoSnapshot::default(), clock(), &Thresholds::default());
        let mut snap = RepoSnapshot::default();
        snap.needs_decision.push("bd-7".to_string());
        let filed = assess(&snap, clock(), &Thresholds::default());

        let events = transitions(&empty, &filed);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            TransitionEvent::Attention(AttentionItem::NeedsDecision { bead_id }) if bead_id == "bd-7"
        ));

        // The next tick, still filed: must not fire again.
        assert!(transitions(&filed, &filed).is_empty());
    }

    #[test]
    fn fleet_drained_fires_once_on_entry_and_not_again_while_it_stays_drained() {
        let thresholds = Thresholds::default();

        let mut active_snap = RepoSnapshot::default();
        active_snap.agents.insert("a1".to_string(), seen(0));
        let active = assess(&active_snap, clock(), &thresholds);

        let mut drained_snap = RepoSnapshot::default();
        drained_snap
            .agents
            .insert("a1".to_string(), seen(dead_age()));
        let drained = assess(&drained_snap, clock(), &thresholds);
        assert_eq!(drained.status, RepoStatus::Drained);

        let events = transitions(&active, &drained);
        assert!(events.contains(&TransitionEvent::FleetDrained));

        assert!(transitions(&drained, &drained).is_empty());
    }
}
