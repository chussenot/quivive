//! The tick: repo -> readers -> merged evidence -> the tile.
//!
//! See the data-flow diagram in `docs/spec.md#the-tick`. This module is the solid
//! edges of it; `cursor` is the dotted ones.

pub mod lease;
pub mod ledger;
pub mod worktree;

use std::collections::BTreeMap;
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
    let mtimes = worktree::read(&repo_root, &lea.leases);

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
    // A missing leases directory is deliberately NOT degraded. Nobody holding a
    // path is the resting state of a repository, and a bar dimmed for it would be
    // dimmed most of the time.

    // Merge, newest wins. The ledger's fold is the accumulator; lease and
    // worktree evidence is layered on top and never written back into the cursor.
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
    for (agent, at) in &mtimes {
        note(agent, *at);
    }

    Ok(Readings {
        agents,
        leases: lea.leases,
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
