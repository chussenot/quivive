//! Fixture builder for the integration tests.
//!
//! Two rules this module exists to enforce:
//!
//! * **The clock is a parameter, never the wall clock.** Every fixture is built
//!   relative to [`NOW`], and every tile is computed against it. A test that
//!   needed `sleep` would be a report that a tick has stopped being a pure
//!   function of (ledger, clock, thresholds) —
//!   `docs/adr/0001-stream-first-tile.md` — and the fix would be in the fold, not
//!   in the test.
//! * **Fixtures are written as pact writes them.** The rows below carry pact's
//!   real field names, including the ones vigil ignores, because a fixture that
//!   only contains the fields the reader happens to look at asserts what its
//!   author already believed.
//!
//! `PACT_STATE_DIR` must not be set while these run: it redirects
//! `reader::state_dir` away from the fixture. CI does not set it; a developer who
//! has it exported will see every fixture read as an absent ledger.

#![allow(dead_code)] // each test binary uses a different subset

use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use tempfile::TempDir;
use vigil::reader::{self, Readings};
use vigil::state::Thresholds;
use vigil::tile::Tile;

/// The frozen clock every fixture and every golden is relative to.
pub const NOW: &str = "2026-08-28T09:00:00Z";

pub fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(NOW).unwrap().to_utc()
}

pub struct Fixture {
    dir: TempDir,
    ledger: PathBuf,
    has_pact: bool,
}

impl Fixture {
    /// A repository with a `.pact/` directory and an empty ledger.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join(".pact");
        std::fs::create_dir_all(&state).unwrap();
        let ledger = state.join("events.jsonl");
        std::fs::write(&ledger, "").unwrap();
        Self {
            dir,
            ledger,
            has_pact: true,
        }
    }

    /// A repository with no pact in it at all — a normal repository, and the one
    /// case where the text tile must say `unreadable` rather than report zeros.
    pub fn bare() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join(".pact").join("events.jsonl");
        Self {
            dir,
            ledger,
            has_pact: false,
        }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    fn stamp(&self, secs_ago: i64) -> String {
        (now() - TimeDelta::seconds(secs_ago)).to_rfc3339()
    }

    /// One lease event, in pact's shape.
    pub fn event(&self, agent: &str, kind: &str, secs_ago: i64) -> &Self {
        self.raw(&format!(
            r#"{{"at":"{}","agent":"{}","kind":"{}","path":"src/{}.rs","detail":null,"ttl_secs":900,"chain_hash":"deadbeef"}}"#,
            self.stamp(secs_ago),
            agent,
            kind,
            agent
        ))
    }

    /// A line exactly as given — for garbage, blanks and partial writes.
    pub fn raw(&self, line: &str) -> &Self {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.ledger)
            .unwrap();
        writeln!(f, "{line}").unwrap();
        self
    }

    /// Append without a trailing newline: what a tick sees when it lands
    /// mid-append. The most likely bug in the crate has this shape.
    pub fn partial(&self, text: &str) -> &Self {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.ledger)
            .unwrap();
        write!(f, "{text}").unwrap();
        self
    }

    /// Rewrite the ledger with only these lines — what pact itself does once the
    /// file passes 5000 lines, keeping the newest 4000.
    pub fn rewrite(&self, lines: &[&str]) -> &Self {
        let mut body = String::new();
        for l in lines {
            body.push_str(l);
            body.push('\n');
        }
        std::fs::write(&self.ledger, body).unwrap();
        self
    }

    /// One lock file, in pact's shape. `file_exists` controls whether the leased
    /// path is on disk, which is what the worktree reader keys off.
    pub fn lease(
        &self,
        agent: &str,
        path: &str,
        acquired_secs_ago: i64,
        ttl_secs: u64,
        file_exists: bool,
    ) -> &Self {
        let leases = self.dir.path().join(".pact").join("leases");
        std::fs::create_dir_all(&leases).unwrap();
        let lock = format!(
            r#"{{"agent":"{}","path":"{}","acquired_at":"{}","ttl_secs":{},"note":"a note","branch":"main"}}"#,
            agent,
            path,
            self.stamp(acquired_secs_ago),
            ttl_secs
        );
        std::fs::write(
            leases.join(format!("{}.lock", path.replace('/', "%2F"))),
            lock,
        )
        .unwrap();
        if file_exists {
            let target = self.dir.path().join(path);
            if let Some(p) = target.parent() {
                std::fs::create_dir_all(p).unwrap();
            }
            std::fs::write(&target, "content").unwrap();
        }
        self
    }

    /// A file beside the locks that is not a `.lock` — pact's staging sibling,
    /// which a tick landing mid-acquire will see.
    pub fn lease_staging_file(&self, name: &str) -> &Self {
        let leases = self.dir.path().join(".pact").join("leases");
        std::fs::create_dir_all(&leases).unwrap();
        std::fs::write(leases.join(name), "half a lock").unwrap();
        self
    }

    pub fn read(&self, use_cursor: bool) -> Readings {
        reader::read(&reader::Options {
            repo_root: self.dir.path().to_path_buf(),
            use_cursor,
        })
        .expect("a fixture repository is always readable")
    }

    pub fn commit(&self, readings: &Readings) {
        reader::commit(self.dir.path(), readings);
    }

    /// The tile, with the default thresholds unless overridden.
    pub fn tile(&self, use_cursor: bool, thresholds: &Thresholds) -> Tile {
        let readings = self.read(use_cursor);
        let tile = Tile::build(&readings, "REPO", now(), thresholds);
        self.commit(&readings);
        tile
    }

    pub fn has_cursor(&self) -> bool {
        vigil::cursor::load(&self.dir.path().join(".pact")).is_some()
    }

    pub fn delete_cursor(&self) {
        let _ = std::fs::remove_file(vigil::cursor::path_in(&self.dir.path().join(".pact")));
    }

    pub fn is_bare(&self) -> bool {
        !self.has_pact
    }
}
