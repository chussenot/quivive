//! The command surface.
//!
//! docs/spec.md S22 is verbatim: "The whole CLI is: tile, watch, why." Every
//! value list here is rendered by clap from the type that defines it —
//! `--exit-on` from [`RepoStatus`] — so `--help` cannot offer something the parser
//! rejects, or omit something it accepts. That is the permanent fix for a
//! drifting list, and it is the drift the `cli-surface-auditor` role exists to
//! catch elsewhere.
//!
//! Defaults are NOT written here either. The four window defaults live as string
//! consts in [`crate::state`], where `Thresholds::default()` parses the same
//! consts clap renders into `--help` — so the help text, the parser and the
//! library cannot disagree about what a default is. A help string that restated
//! one in prose would be the first thing to go stale.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::state::{ACTIVE_DEFAULT, DEAD_DEFAULT, FORGET_DEFAULT, IDLE_DEFAULT, RepoStatus};
use crate::watch;

#[derive(Parser, Debug)]
#[command(
    name = "quivive",
    about = "Is the fleet alive, right now?",
    long_about = "Folds pact's lease ledger into one line describing the fleet now.\n\n\
                  Designed to be called on a timer by a status bar: `quivive tile` reads, \
                  prints one tile and exits. There is no daemon, and quivive does not draw — \
                  it emits the contract in docs/tile-contract.md and the bar renders it.",
    after_long_help = "EXIT CODES\n  \
        0  a tile was produced, including a quiet or degraded one\n  \
        1  no tile could be produced (bad flags, unreadable --repo)\n  \
        2  --exit-on was given and the tile met or exceeded that status\n\n\
        EXAMPLES\n  \
        quivive tile                         the full contract, every registered repo\n  \
        quivive tile --repo .                just this repository\n  \
        quivive tile --text                  one summary line instead of the JSON payload\n  \
        quivive tile --no-cursor             force a full re-read; the fastest cursor diagnostic\n  \
        quivive why .                        the attention items behind the tile\n  \
        quivive tile --exit-on human-needed || notify-send 'fleet needs you'\n  \
        quivive tile --dead-window 2h        a fleet whose beads take an hour",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Print one tile and exit.
    Tile {
        #[command(flatten)]
        common: Common,
        /// Follow the pwetty push contract instead of printing once: emit one
        /// JSON line per change and stay alive. See docs/spec.md S9.
        #[arg(long)]
        stream: bool,
    },
    /// Send `notify-send` notifications on fleet transitions, until interrupted.
    ///
    /// Not a daemon: foreground, owned by whoever started it, binds no socket and
    /// serves no second consumer. See docs/adr/0002-no-daemon-renderer-boundary.md.
    /// Fires on TRANSITIONS only — an event fires when it becomes true, not while
    /// it stays true (docs/spec.md S14).
    Watch {
        /// How often to re-check the registry for new evidence. Every repo
        /// unchanged since its last read (docs/spec.md S5) is skipped
        /// without a real read, so this mostly governs how quickly a
        /// genuine change is noticed, not how much work each pass does.
        #[arg(long, default_value = watch::INTERVAL_DEFAULT, value_name = "DURATION")]
        interval: String,

        /// Suppress a repeat notification for the same (repo, event) inside
        /// this window, so a condition flapping true/false/true notifies
        /// once (docs/spec.md S15).
        #[arg(long, default_value = watch::DEBOUNCE_DEFAULT, value_name = "DURATION")]
        debounce: String,
    },
    /// List the attention-worthy items for one repo, each with the evidence
    /// line(s) that produced it (docs/spec.md S21).
    Why {
        /// The repository to describe.
        #[arg(value_name = "REPO")]
        repo: PathBuf,
        /// Emit the items as JSON instead of the text form.
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Args, Debug, Clone)]
pub struct Common {
    /// The repository to describe. Omitted: every repository named in the
    /// registry (`~/.config/quivive/repos`, docs/spec.md S1-S2). Given: only
    /// this one, and a path that does not resolve is a real error rather than
    /// a degraded registry entry.
    #[arg(long, value_name = "PATH", global = true)]
    pub repo: Option<PathBuf>,

    /// Print the compact one-line summary instead of the JSON payload.
    /// JSON is the default (S10-S11: the payload IS the contract), so this
    /// flag exists to opt OUT of it, not into it.
    #[arg(long, global = true)]
    pub text: bool,

    /// Newer than this: ACTIVE.
    #[arg(long, default_value = ACTIVE_DEFAULT, value_name = "DURATION")]
    pub active_window: String,

    /// Newer than this, but not ACTIVE: IDLE.
    #[arg(long, default_value = IDLE_DEFAULT, value_name = "DURATION")]
    pub idle_window: String,

    /// Older than this: DEAD. The one state anybody acts on, so this is the knob
    /// that actually matters — a fleet whose beads take an hour needs a different
    /// value from one whose beads take a minute.
    #[arg(long, default_value = DEAD_DEFAULT, value_name = "DURATION")]
    pub dead_window: String,

    /// Quiet this long and an agent leaves the tile altogether — unless it is
    /// holding a lease, in which case it stays however long it has been gone.
    #[arg(long, default_value = FORGET_DEFAULT, value_name = "DURATION")]
    pub forget: String,

    /// Exit 2 if the overall status reaches this or worse. The cheap 90% of an
    /// alert, using whatever notifier the caller already has configured.
    #[arg(long, value_name = "STATUS")]
    pub exit_on: Option<RepoStatus>,

    /// Ignore and do not write the resume cursor: read the whole ledger.
    ///
    /// The fastest diagnostic here. If the tile changes when you pass this, the
    /// cursor is wrong; if it does not, the bug is in the fold or the readers.
    /// Deleting .pact/quivive-cursor.json by hand does the same thing and must
    /// always be safe.
    #[arg(long)]
    pub no_cursor: bool,
}

/// Lets `--exit-on` take one of [`RepoStatus`]'s five S8 spellings and renders
/// them into `--help`, without `state.rs` — which predates `--exit-on`
/// retargeting from `State` to `RepoStatus` — taking a `clap` dependency for a
/// concern that is entirely this CLI's. `RepoStatus` is defined in this crate,
/// so implementing the foreign `ValueEnum` trait for it here is ordinary Rust,
/// not a workaround: only one of the trait or the type needs to be local, and
/// the type is. The spellings mirror `RepoStatus`'s own
/// `#[serde(rename_all = "kebab-case")]` exactly, so the JSON payload and
/// `--exit-on`'s accepted values cannot disagree about a status name.
impl clap::ValueEnum for RepoStatus {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            RepoStatus::HumanNeeded,
            RepoStatus::Active,
            RepoStatus::Drained,
            RepoStatus::AllQuiet,
            RepoStatus::NoFleet,
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(match self {
            RepoStatus::HumanNeeded => "human-needed",
            RepoStatus::Active => "active",
            RepoStatus::Drained => "drained",
            RepoStatus::AllQuiet => "all-quiet",
            RepoStatus::NoFleet => "no-fleet",
        }))
    }
}
