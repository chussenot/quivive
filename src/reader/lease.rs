//! The lease reader: pact's `.pact/leases/*.lock`, read whole every tick.
//!
//! Small and mutable, so there is no cursor here and no fold: a lease that was
//! released between two ticks must vanish from the tile, and any accumulated
//! memory of it would be a fact contradicting its own source.

use std::path::Path;

use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;

pub const LEASES_DIR: &str = "leases";

/// pact's lock-file shape, narrowed to what quivive reads. The remaining
/// fields — `note`, `branch`, `worktree`, `invoked_from`, the at-acquire blob id —
/// are informational and ignored.
#[derive(Deserialize)]
struct Lock {
    agent: String,
    path: String,
    acquired_at: String,
    ttl_secs: u64,
}

#[derive(Debug, Clone)]
pub struct Lease {
    pub agent: String,
    /// Repo-relative, as pact recorded it — not decoded from the lock filename.
    /// pact encodes the path into the filename and *also* stores it in the file;
    /// reading the field means quivive does not have to reimplement, and then keep
    /// up with, somebody else's escaping.
    pub path: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl Lease {
    /// Seconds past expiry, or 0 while still live.
    ///
    /// Epoch arithmetic rather than `now - self.expires_at`: subtracting two
    /// `DateTime`s yields a `TimeDelta`, which **panics** when the span exceeds
    /// i64 milliseconds. Two timestamps chrono will happily parse can be 500,000
    /// years apart, and a panic is the one failure mode a status bar cannot
    /// survive. Two `i64` epoch seconds cannot overflow each other.
    pub fn expired_for(&self, now: DateTime<Utc>) -> i64 {
        (now.timestamp() - self.expires_at.timestamp()).max(0)
    }
}

pub struct Reading {
    pub leases: Vec<Lease>,
    pub declined: usize,
    /// False when there is no leases directory: nobody currently holds a path.
    /// A normal state, and not the same thing as an unreadable one.
    pub present: bool,
}

pub fn read(state_dir: &Path) -> Reading {
    let dir = state_dir.join(LEASES_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Reading {
            leases: Vec::new(),
            declined: 0,
            present: false,
        };
    };

    let mut leases = Vec::new();
    let mut declined = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        // `.lock` only. pact stages a lease through a unique temp sibling before
        // renaming it into place, so a tick that lands mid-acquire will see that
        // staging file — counting it as a damaged lease would report corruption
        // during the most normal operation pact has.
        if path.extension().is_none_or(|e| e != "lock") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            declined += 1;
            continue;
        };
        let Ok(lock) = serde_json::from_str::<Lock>(&raw) else {
            declined += 1;
            continue;
        };
        let Ok(acquired) = DateTime::parse_from_rfc3339(&lock.acquired_at) else {
            declined += 1;
            continue;
        };
        let acquired_at = acquired.to_utc();
        // `ttl_secs` is a u64 read from a file on disk, so every step here has to
        // survive a garbage value. `TimeDelta::seconds` PANICS out of range — it
        // is not saturating, which is what a first draft of this assumed and what
        // `a_lock_with_an_absurd_ttl_does_not_panic` caught — so the fallible
        // constructor is used, and the add is checked on top of it. A lease with a
        // nonsense TTL reads as one that never expires, which is the harmless
        // direction: it is reported as blocking only if its holder goes quiet.
        let expires_at = TimeDelta::try_seconds(lock.ttl_secs.min(i64::MAX as u64) as i64)
            .and_then(|d| acquired_at.checked_add_signed(d))
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        leases.push(Lease {
            agent: lock.agent,
            path: lock.path,
            acquired_at,
            expires_at,
        });
    }
    // Sorted by path so the tile's `blocked_leases` is stable across ticks:
    // read_dir order is whatever the filesystem feels like, and an unstable
    // ordering would make the goldens flap and a diff of two tiles unreadable.
    leases.sort_by(|a, b| a.path.cmp(&b.path));

    Reading {
        leases,
        declined,
        present: true,
    }
}
