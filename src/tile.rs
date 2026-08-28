//! The tile: vigil's API.
//!
//! Not the CLI flags and not the text layout — the tile. Everything in this file
//! is the contract specified in `docs/tile-contract.md`, and the goldens in
//! `tests/goldens.rs` are what stop it moving under a consumer.
//!
//! Adding a field is additive and does not move `v`; removing or renaming one,
//! changing a type, changing a *meaning*, or changing severity order is breaking
//! and does. Severity order is not encoded anywhere in the shape, which is
//! exactly why moving it is invisible in a diff and has to be a deliberate call.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;

use crate::dur;
use crate::reader::Readings;
use crate::state::{State, Thresholds};

/// Contract version. See the module docs and `docs/tile-contract.md`.
pub const TILE_V: u32 = 1;

/// `worst` when no agent is known at all. Not a [`State`]: it is the absence of
/// agents rather than a state one could be in, and giving it a variant would put
/// it in the severity ordering, where it does not belong.
pub const QUIET: &str = "quiet";

#[derive(Debug, Serialize)]
pub struct Fleet {
    pub active: usize,
    pub idle: usize,
    pub stale: usize,
    pub dead: usize,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct AgentTile {
    pub id: String,
    pub state: State,
    /// Seconds, integer, relative to `at` — never a formatted string. A bar that
    /// wants `6m52s` can compute it; a bar handed `6m52s` and wanting seconds
    /// cannot get back.
    pub age_s: i64,
    pub leases: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BlockedLease {
    pub path: String,
    pub held_by: String,
    /// Seconds past the lease's TTL, or 0 while it is still live. A lease held by
    /// a stale agent is already worth reporting before it expires — that is the
    /// window in which somebody can still ask the holder to release it.
    pub expired_s: i64,
}

#[derive(Debug, Serialize)]
pub struct Tile {
    pub v: u32,
    /// The clock this tick was computed against — not "now" at read time.
    pub at: String,
    pub repo: String,
    pub fleet: Fleet,
    pub worst: String,
    pub agents: Vec<AgentTile>,
    /// Leases held by a stale or dead agent: the actionable subset, not every
    /// lease. A live agent holding a lease is the system working.
    pub blocked_leases: Vec<BlockedLease>,
    /// Readers that could not read, named. Empty is the normal case, and this is
    /// a field rather than a log line or an error precisely so that a renderer
    /// can show a dimmed tile instead of an empty one — the difference between
    /// "nothing is running" and "I cannot see".
    pub degraded: Vec<String>,
}

impl Tile {
    pub fn build(
        readings: &Readings,
        repo: &str,
        now: DateTime<Utc>,
        thresholds: &Thresholds,
    ) -> Self {
        let holders: Vec<&str> = readings.leases.iter().map(|l| l.agent.as_str()).collect();

        let mut agents: Vec<AgentTile> = readings
            .agents
            .iter()
            .map(|(id, seen)| {
                // Epoch seconds, not `now - *seen`: subtracting two `DateTime`s
                // panics when the span exceeds i64 milliseconds, and two
                // timestamps chrono will parse can be 500,000 years apart. See
                // `Lease::expired_for` for the same reasoning.
                let age_s = now.timestamp() - seen.timestamp();
                AgentTile {
                    id: id.clone(),
                    state: thresholds.classify(age_s),
                    age_s: age_s.max(0),
                    leases: readings
                        .leases
                        .iter()
                        .filter(|l| &l.agent == id)
                        .map(|l| l.path.clone())
                        .collect(),
                }
            })
            // The forget sweep. Bookkeeping, not a state: without it a week-old
            // repository renders forty dead names on a bar with room for one line.
            //
            // An agent holding a lease is never forgotten, however long it has
            // been quiet. Forgetting is for names that no longer matter, and an
            // agent whose expired claim is blocking somebody is the single thing
            // in this tile most worth acting on.
            .filter(|a| {
                (a.age_s as u64) < thresholds.forget.as_secs() || holders.contains(&a.id.as_str())
            })
            .collect();

        // Severity descending, then age descending, then id. The last key is not
        // decoration: read_dir order and BTreeMap order are different things, and
        // two agents with the same state and the same age must not swap places
        // between ticks or the goldens flap.
        agents.sort_by(|a, b| {
            b.state
                .cmp(&a.state)
                .then(b.age_s.cmp(&a.age_s))
                .then(a.id.cmp(&b.id))
        });

        let mut fleet = Fleet {
            active: 0,
            idle: 0,
            stale: 0,
            dead: 0,
            total: agents.len(),
        };
        for a in &agents {
            match a.state {
                State::Active => fleet.active += 1,
                State::Idle => fleet.idle += 1,
                State::Stale => fleet.stale += 1,
                State::Dead => fleet.dead += 1,
            }
        }

        let worst = agents
            .iter()
            .map(|a| a.state)
            .max()
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| QUIET.to_string());

        // A lease whose holder is not in the agent list at all is treated as
        // blocking: the only way that happens is a lock file naming an agent the
        // ledger has never seen, which is a lease nobody is coming back for.
        let blocked_leases = readings
            .leases
            .iter()
            .filter(|l| {
                agents
                    .iter()
                    .find(|a| a.id == l.agent)
                    .is_none_or(|a| a.state.is_blocking())
            })
            .map(|l| BlockedLease {
                path: l.path.clone(),
                held_by: l.agent.clone(),
                expired_s: l.expired_for(now),
            })
            .collect();

        Tile {
            v: TILE_V,
            at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
            repo: repo.to_string(),
            fleet,
            worst,
            agents,
            blocked_leases,
            degraded: readings.degraded.clone(),
        }
    }

    /// The one-line text form. Also a contract — consumers parse it whether or
    /// not they are invited to, so declaring a shape is cheaper than pretending
    /// there is not one.
    ///
    /// One line, always: including `quiet`, including fully degraded. A status bar
    /// has one line of room and no way to be told otherwise.
    pub fn text(&self) -> String {
        // An unreadable ledger is the one case where the counts would be a lie
        // rather than a fact: `0A 0I 0S 0D` says "nothing is running" and the
        // truth is "I cannot see".
        if self.degraded.iter().any(|d| d == "ledger") {
            return format!("unreadable: {}", self.degraded.join(", "));
        }

        let counts = [State::Active, State::Idle, State::Stale, State::Dead]
            .iter()
            .map(|s| {
                let n = match s {
                    State::Active => self.fleet.active,
                    State::Idle => self.fleet.idle,
                    State::Stale => self.fleet.stale,
                    State::Dead => self.fleet.dead,
                };
                format!("{n}{}", s.initial())
            })
            .collect::<Vec<_>>()
            .join(" ");

        let mut line = format!("{counts}  worst={}", self.worst);

        // The detail names the single worst agent, and only when somebody should
        // look at it. Naming the worst agent on a healthy fleet would spend the
        // one line of room on the least interesting fact in the tile.
        if let Some(a) = self.agents.first().filter(|a| a.state.is_blocking()) {
            line.push_str(&format!(
                "  {} {} {}",
                a.id,
                a.state.as_str(),
                dur::human(a.age_s)
            ));
            if let Some(p) = a.leases.first() {
                let more = a.leases.len() - 1;
                line.push_str(&format!(" (holds {p}"));
                if more > 0 {
                    line.push_str(&format!(" +{more}"));
                }
                line.push(')');
            }
        }

        if !self.degraded.is_empty() {
            line.push_str(&format!("  [{}]", self.degraded.join(", ")));
        }
        line
    }

    /// The severity the tile reached, for `--exit-on`. `None` when quiet.
    pub fn worst_state(&self) -> Option<State> {
        self.agents.iter().map(|a| a.state).max()
    }
}
