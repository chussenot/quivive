//! The resume cursor: the only thing that survives a tick.
//!
//! **It must be correct to throw away.** Deleting this file and re-running must
//! produce a byte-identical tile — that is the invariant of
//! `docs/adr/0001-stream-first-tile.md`, it is what makes this a cache rather
//! than a second source of truth, and it is what `--no-cursor` and
//! `scripts/fleet-sim.sh` exist to keep honest.
//!
//! Two consequences run through every line below:
//!
//! * **A cursor is never trusted, only verified.** Garbage, a stale offset, a
//!   file rewritten under us — all of it degrades to a full re-read, silently and
//!   correctly, because a full re-read is always right and only ever slower.
//! * **A cursor failure is never an error.** There is no path here that can fail
//!   a tick. A status bar going dark because a cache file was corrupt would be
//!   the cache costing more than it saves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Inside the pact state directory, beside the ledger it points into. It lives
/// there rather than in a directory of vigil's own because `.pact/` is already
/// gitignored and already the place this repository's coordination state lives;
/// a second dot-directory for one cache file is not worth the clutter.
///
/// Named in `docs/tile-contract.md`, which makes it part of the contract: a
/// consumer is entitled to delete this file, and deleting it must only cost time.
pub const CURSOR_FILE: &str = "vigil-cursor.json";

/// Bumped only if this file's shape changes incompatibly. An older cursor is not
/// migrated, it is discarded — see [`load`]. Migration code for a cache is code
/// written to preserve something that is correct to throw away.
const CURSOR_V: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    pub v: u32,
    /// Byte offset into the ledger, always immediately after a newline.
    pub offset: u64,
    /// Length in bytes of the last line consumed, and its hash. Together these
    /// are what let [`Cursor::still_describes`] tell "the ledger grew" from "the
    /// ledger was rewritten and then grew past where we were", which a bare
    /// offset cannot — and which pact's ledger really does do: it rewrites the
    /// file down to its newest 4000 lines once it passes 5000, so truncation is
    /// a *routine* event here, not a disaster case.
    pub tail_len: u64,
    pub tail_hash: u64,
    /// The folded accumulator: agent -> newest evidence found **in the ledger**.
    ///
    /// Ledger-only, deliberately. Lease and worktree evidence is re-read whole
    /// every tick, so folding it in here would persist a fact that its own
    /// source could contradict a second later — and the throw-away invariant
    /// would then be false in a way no test could easily see.
    pub agents: BTreeMap<String, DateTime<Utc>>,
}

impl Cursor {
    pub fn empty() -> Self {
        Self {
            v: CURSOR_V,
            offset: 0,
            tail_len: 0,
            tail_hash: 0,
            agents: BTreeMap::new(),
        }
    }

    /// Does this cursor still describe the file in front of us?
    ///
    /// Three ways it can be stale, in the order they are cheap to check:
    ///
    /// 1. It points past the end — the file shrank. pact's own rewrite does
    ///    exactly this.
    /// 2. It points at offset 0 with nothing consumed — nothing to resume.
    /// 3. The bytes immediately before the offset are no longer the line we
    ///    consumed. This is the one an offset alone cannot catch: after a rewrite
    ///    the file can grow back past the old offset, and resuming there would
    ///    skip real events and silently under-report the fleet.
    pub fn still_describes(&self, file_len: u64, tail_bytes: &[u8]) -> bool {
        self.v == CURSOR_V
            && self.offset > 0
            && self.tail_len > 0
            && self.offset <= file_len
            && tail_bytes.len() as u64 == self.tail_len
            && hash(tail_bytes) == self.tail_hash
    }
}

/// FNV-1a, 64-bit, hand-rolled.
///
/// Not a dependency and not a cryptographic hash, and neither matters: this
/// compares one short line against the same short line, and the cost of a
/// collision is a tile computed from a resumed read that should have been cold —
/// which the next tick corrects. pact's ledger has its own real chain hash for
/// the job that needs one (forgery), and vigil deliberately does not duplicate it.
pub fn hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

pub fn path_in(state_dir: &Path) -> PathBuf {
    state_dir.join(CURSOR_FILE)
}

/// Load, or `None` for every possible reason: absent, unreadable, unparsable,
/// written by a future version. Never an error — see the module docs.
pub fn load(state_dir: &Path) -> Option<Cursor> {
    let raw = std::fs::read_to_string(path_in(state_dir)).ok()?;
    let c: Cursor = serde_json::from_str(&raw).ok()?;
    (c.v == CURSOR_V).then_some(c)
}

/// Save atomically — temp sibling, then rename — and swallow every failure.
///
/// Atomically because two consumers can tick at once (two bars, or a bar and a
/// `watch`), and a half-written cursor read by the other one would be a
/// corruption that persists. The rename makes a reader see either the old cursor
/// or the new one, and both are correct.
///
/// The temp name carries the process id so that those two consumers do not
/// stage over each other's file.
pub fn save(state_dir: &Path, cursor: &Cursor) {
    let final_path = path_in(state_dir);
    let tmp = state_dir.join(format!(".{CURSOR_FILE}.{}.tmp", std::process::id()));
    let Ok(json) = serde_json::to_string(cursor) else {
        return;
    };
    if std::fs::write(&tmp, json).is_err() {
        return;
    }
    if std::fs::rename(&tmp, &final_path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor_over(line: &[u8], offset: u64) -> Cursor {
        Cursor {
            v: CURSOR_V,
            offset,
            tail_len: line.len() as u64,
            tail_hash: hash(line),
            agents: BTreeMap::new(),
        }
    }

    #[test]
    fn a_matching_tail_resumes() {
        let line = b"{\"agent\":\"a\"}\n";
        let c = cursor_over(line, 100);
        assert!(c.still_describes(100, line));
        assert!(c.still_describes(4096, line));
    }

    #[test]
    fn a_shrunken_file_does_not_resume() {
        // pact rewrites events.jsonl down to its newest 4000 lines once it passes
        // 5000, so this is the routine case, not the exotic one.
        let line = b"{\"agent\":\"a\"}\n";
        let c = cursor_over(line, 100);
        assert!(!c.still_describes(50, line));
    }

    #[test]
    fn a_rewritten_file_that_grew_back_past_us_does_not_resume() {
        // The case a bare offset cannot catch, and the reason tail_hash exists:
        // the file is LONGER than our offset, so a length check passes, and
        // resuming there would skip every event written since the rewrite.
        let ours = b"{\"agent\":\"a\"}\n";
        let theirs = b"{\"agent\":\"b\"}\n";
        let c = cursor_over(ours, 100);
        assert!(!c.still_describes(9999, theirs));
    }

    #[test]
    fn a_zero_cursor_never_resumes() {
        assert!(!cursor_over(b"", 0).still_describes(100, b""));
    }

    #[test]
    fn a_future_version_is_discarded_rather_than_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Cursor::empty();
        c.v = CURSOR_V + 1;
        std::fs::write(path_in(dir.path()), serde_json::to_string(&c).unwrap()).unwrap();
        assert!(load(dir.path()).is_none());
    }

    #[test]
    fn garbage_loads_as_none_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(path_in(dir.path()), "not json at all").unwrap();
        assert!(load(dir.path()).is_none());
        assert!(load(Path::new("/definitely/not/here")).is_none());
    }

    #[test]
    fn a_save_into_an_unwritable_directory_is_silent() {
        // The whole module promises it cannot fail a tick. This is that promise.
        save(Path::new("/definitely/not/here"), &Cursor::empty());
    }

    #[test]
    fn a_round_trip_preserves_the_accumulator() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Cursor::empty();
        c.offset = 42;
        c.tail_len = 7;
        c.tail_hash = hash(b"abcdefg");
        c.agents.insert(
            "agent-3".into(),
            DateTime::parse_from_rfc3339("2026-08-28T09:41:07Z")
                .unwrap()
                .to_utc(),
        );
        save(dir.path(), &c);
        let back = load(dir.path()).expect("saved cursor should load");
        assert_eq!(back.offset, 42);
        assert_eq!(back.agents.len(), 1);
        assert!(back.still_describes(100, b"abcdefg"));
    }
}
