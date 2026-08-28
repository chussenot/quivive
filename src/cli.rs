//! The command surface.
//!
//! Every value list here is rendered by clap from the type that defines it —
//! `--exit-on` from [`State`], the shells from `clap_complete::Shell` — so
//! `--help` cannot offer something the parser rejects, or omit something it
//! accepts. That is the permanent fix for a drifting list, and it is the drift
//! the `cli-surface-auditor` role exists to catch elsewhere.
//!
//! Defaults are NOT written here either. The four window defaults live as string
//! consts in [`crate::state`], where `Thresholds::default()` parses the same
//! consts clap renders into `--help` — so the help text, the parser and the
//! library cannot disagree about what a default is. A help string that restated
//! one in prose would be the first thing to go stale.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

use crate::state::{ACTIVE_DEFAULT, DEAD_DEFAULT, FORGET_DEFAULT, IDLE_DEFAULT, State};

#[derive(Parser, Debug)]
#[command(
    name = "vigil",
    about = "Is the fleet alive, right now?",
    long_about = "Folds pact's lease ledger into one line describing the fleet now.\n\n\
                  Designed to be called on a timer by a status bar: `vigil tile` reads, \
                  prints one tile and exits. There is no daemon, and vigil does not draw — \
                  it emits the contract in docs/tile-contract.md and the bar renders it.",
    after_long_help = "EXIT CODES\n  \
        0  a tile was produced, including a quiet or degraded one\n  \
        1  no tile could be produced (bad flags, unreadable --repo)\n  \
        2  --exit-on was given and the tile met or exceeded that state\n\n\
        EXAMPLES\n  \
        vigil tile                         one line, this repository\n  \
        vigil tile --json                  the full contract\n  \
        vigil tile --no-cursor             force a full re-read; the fastest cursor diagnostic\n  \
        vigil watch --interval 2s          one tile per tick on stdout\n  \
        vigil tile --exit-on dead || notify-send 'fleet down'\n  \
        vigil tile --dead-window 2h        a fleet whose beads take an hour",
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
    },
    /// Print one tile per tick, on stdout, until interrupted.
    ///
    /// Not a daemon: foreground, owned by whoever started it, binds no socket and
    /// serves no second consumer. When it dies the pipe closes and its consumer
    /// finds out immediately, which is precisely what a daemon does not do. See
    /// docs/adr/0002-no-daemon-renderer-boundary.md.
    Watch {
        #[command(flatten)]
        common: Common,
        /// How long to wait between ticks.
        #[arg(long, default_value = "1s", value_name = "DURATION")]
        interval: String,
    },
    /// Write a shell completion script to stdout.
    Completion {
        #[arg(value_name = "SHELL")]
        shell: Shell,
    },
}

#[derive(clap::Args, Debug, Clone)]
pub struct Common {
    /// The repository to describe.
    #[arg(long, default_value = ".", value_name = "PATH", global = true)]
    pub repo: PathBuf,

    /// Emit the full tile contract as JSON instead of the one-line text form.
    #[arg(long, global = true)]
    pub json: bool,

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

    /// Exit 2 if the tile reaches this state or worse. The cheap 90% of an alert,
    /// using whatever notifier the caller already has configured.
    #[arg(long, value_name = "STATE")]
    pub exit_on: Option<State>,

    /// Ignore and do not write the resume cursor: read the whole ledger.
    ///
    /// The fastest diagnostic here. If the tile changes when you pass this, the
    /// cursor is wrong; if it does not, the bug is in the fold or the readers.
    /// Deleting .pact/vigil-cursor.json by hand does the same thing and must
    /// always be safe.
    #[arg(long)]
    pub no_cursor: bool,
}
