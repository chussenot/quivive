//! The worktree reader: the modification time of each leased path.
//!
//! ## Why this reader exists
//!
//! An agent can be very much alive and write nothing to the ledger for minutes at
//! a stretch, because it is thinking or compiling. Without this, a compile longer
//! than `--active-window` reports IDLE, and one longer than `--idle-window`
//! reports STALE — the tile crying wolf on the most normal thing an agent does.
//!
//! ## Why this is not the deferred "guessing at liveness"
//!
//! `docs/adr/0003-yagni-deferral-register.md` (D10) refuses heuristics over
//! ambient machine state: process tables, tty activity, editor state. This is a
//! near neighbour of that refusal and stays on the right side of it for two
//! reasons worth stating rather than assuming:
//!
//! * The mtime is a trace **the agent wrote**, not an inference about a process.
//! * It is scoped to a path the agent **explicitly claimed** with a lease. vigil
//!   never stats a file nobody has taken responsibility for, so there is no
//!   possibility of crediting one agent with another's work, or with a `git
//!   checkout`.
//!
//! ## Why it was called the "git reader" in the first draft of the spec
//!
//! Because the surface it reads sounded like git's. Implementing it showed the
//! liveness evidence is the filesystem's mtime and nothing else: no ref, no
//! index, no HEAD. The spec was renamed to match the code rather than the code
//! bent to match the spec — see `docs/studies/conventions-run.md`.

use std::path::Path;

use chrono::{DateTime, Utc};

use super::lease::Lease;

/// `(agent, mtime)` for every leased path that exists and can be stat'd.
///
/// A path that does not exist is not an error and not a decline: pact leases
/// paths that have not been created yet, which is the normal way to claim a file
/// you are about to write.
pub fn read(repo_root: &Path, leases: &[Lease]) -> Vec<(String, DateTime<Utc>)> {
    let mut out = Vec::new();
    for lease in leases {
        // Repo-relative, and it must stay that way: a lease path is data from a
        // file on disk, and joining an absolute path onto the root would let a
        // lock file point vigil anywhere on the filesystem.
        let rel = Path::new(&lease.path);
        if rel.is_absolute() || rel.components().any(|c| c.as_os_str() == "..") {
            continue;
        }
        let Ok(meta) = std::fs::metadata(repo_root.join(rel)) else {
            continue;
        };
        let Ok(mtime) = meta.modified() else {
            continue;
        };
        out.push((lease.agent.clone(), DateTime::<Utc>::from(mtime)));
    }
    out
}
