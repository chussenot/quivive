//! Thin: parse, dispatch, choose an exit code. Everything that decides anything
//! lives in the library, so the tests can reach it without spawning a process.

use std::io::Write;

use anyhow::Result;
use clap::{CommandFactory, Parser};

use vigil::cli::{Cli, Command, Common};
use vigil::state::Thresholds;
use vigil::tile::Tile;
use vigil::{EXIT_FAIL, EXIT_OK, EXIT_TRIGGERED, dur, reader};

fn main() {
    // clap's own exit code for a usage error is 2, and docs/tile-contract.md
    // reserves 2 for --exit-on. A documented exit code the binary does not
    // actually use is worse than no documentation, so the code is overridden
    // here rather than the contract bent to match the library.
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            // --help and --version arrive here too, and they are a success.
            std::process::exit(if e.use_stderr() { EXIT_FAIL } else { EXIT_OK });
        }
    };

    match run(cli) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("vigil: {e:#}");
            std::process::exit(EXIT_FAIL);
        }
    }
}

fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::Completion { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(EXIT_OK)
        }
        Command::Tile { common } => tick_once(&common),
        Command::Watch { common, interval } => {
            let interval = dur::parse(&interval)?;
            // One tick before the first sleep: a bar that starts vigil wants a
            // tile now, not in a second.
            loop {
                let code = tick_once(&common)?;
                // --exit-on ends a watch as well as a tile. A watch that kept
                // printing after the condition it was asked to watch for would
                // make the flag mean something different in the two commands.
                if code == EXIT_TRIGGERED {
                    return Ok(code);
                }
                std::thread::sleep(interval);
            }
        }
    }
}

fn tick_once(common: &Common) -> Result<i32> {
    let thresholds = Thresholds {
        active: dur::parse(&common.active_window)?,
        idle: dur::parse(&common.idle_window)?,
        dead: dur::parse(&common.dead_window)?,
        forget: dur::parse(&common.forget)?,
    };
    thresholds.validate()?;

    let opts = reader::Options {
        repo_root: common.repo.clone(),
        use_cursor: !common.no_cursor,
    };
    let readings = reader::read(&opts)?;

    // One read of the clock per tick, taken here and passed down. See
    // `vigil::now` for why the seam exists and what it is for.
    let now = vigil::now()?;
    let repo = opts
        .repo_root
        .canonicalize()
        .unwrap_or_else(|_| opts.repo_root.clone());
    let tile = Tile::build(&readings, &repo.display().to_string(), now, &thresholds);

    // The cursor is committed AFTER the tile is built, so a panic while rendering
    // cannot leave a cursor advanced past events that were never reported.
    reader::commit(&repo, &readings);

    let mut out = std::io::stdout().lock();
    if common.json {
        // Pretty, and deliberately: a tile is read by people at least as often as
        // by bars, `jq` does not care, and a diff of two pretty tiles is legible
        // where a diff of two long lines is not — which the goldens depend on.
        writeln!(out, "{}", serde_json::to_string_pretty(&tile)?)?;
    } else {
        writeln!(out, "{}", tile.text())?;
    }
    // Flush explicitly: `vigil watch | head -1` closes the pipe, and an ignored
    // flush error there is a broken-pipe panic on exit.
    let _ = out.flush();

    if let Some(threshold) = common.exit_on
        && tile.worst_state().is_some_and(|w| w >= threshold)
    {
        return Ok(EXIT_TRIGGERED);
    }
    Ok(EXIT_OK)
}
