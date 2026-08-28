//! quivive — is the fleet alive, right now?
//!
//! One question, answered by folding [pact](https://github.com/chussenot/pact)'s
//! append-only lease ledger forward from a resumable cursor and reading two small
//! mutable surfaces whole. See `docs/spec.md` for what is computed and
//! `docs/adr/0001-stream-first-tile.md` for why it is computed this way.
//!
//! The load-bearing invariant of the whole crate: **deleting the resume cursor
//! and re-running must produce a byte-identical tile.** `tests/goldens.rs` and
//! `scripts/fleet-sim.sh` are what hold it.

pub mod cli;
pub mod cursor;
pub mod dur;
pub mod reader;
pub mod registry;
pub mod state;
pub mod stream;
pub mod tile;
pub mod watch;
pub mod why;

/// The tile was produced — including a `quiet` or `degraded` one.
pub const EXIT_OK: i32 = 0;
/// No tile could be produced at all: bad flags, unreadable repo path.
///
/// clap's own default for a usage error is 2, which `main` overrides, because
/// `docs/tile-contract.md` reserves 2 for `--exit-on` and a documented exit code
/// that the binary does not use is worse than no documentation.
pub const EXIT_FAIL: i32 = 1;
/// `--exit-on <state>` was given and the tile met or exceeded that state.
pub const EXIT_TRIGGERED: i32 = 2;

/// The clock a tick is computed against.
///
/// `QUIVIVE_NOW` (RFC3339) overrides it. That seam is not a convenience: the
/// whole design rests on a tick being a pure function of (ledger, clock,
/// thresholds) — `docs/adr/0001-stream-first-tile.md` — and a claim of purity
/// that cannot be tested end-to-end is a claim about the library, not about the
/// binary somebody runs. With the clock frozen, two invocations over the same
/// ledger must produce byte-identical output, which is exactly what
/// `scripts/fleet-sim.sh` asserts between a resumed read and a cold one.
///
/// It is read once per tick and passed down. Three reads of the clock in three
/// places is a tick that cannot be golden, and a tile whose `at` disagrees with
/// the ages printed beside it.
pub fn now() -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    match std::env::var("QUIVIVE_NOW") {
        Ok(s) if !s.is_empty() => Ok(chrono::DateTime::parse_from_rfc3339(s.trim())
            .map_err(|e| anyhow::anyhow!("QUIVIVE_NOW=`{s}` is not RFC3339: {e}"))?
            .to_utc()),
        _ => Ok(chrono::Utc::now()),
    }
}
