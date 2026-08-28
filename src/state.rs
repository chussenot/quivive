//! The per-agent state machine, and the thresholds that decide it.
//!
//! Every transition is driven by elapsed time or by new evidence and by nothing
//! else — there is no state that depends on how the previous tick was computed.
//! That is what makes a tick a pure function of (ledger, clock, thresholds), and
//! therefore what makes the goldens in `tests/goldens.rs` mean anything. See
//! `docs/spec.md#the-state-machine`.

use std::time::Duration;

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
}
