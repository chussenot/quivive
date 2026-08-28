//! Thin: parse, dispatch, choose an exit code. Everything that decides anything
//! lives in the library, so the tests can reach it without spawning a process.
//!
//! Owns all module wiring (docs/spec.md S22: "the whole CLI is tile, watch,
//! why") so that a bead landing one of `registry`, `stream`, `watch` or `why`
//! never has to touch this file — it edits its own module and the dispatch
//! below keeps working unchanged.

use std::io::Write;

use anyhow::Result;
use clap::Parser;

use quivive::cli::{Cli, Command, Common};
use quivive::state::Thresholds;
use quivive::{EXIT_FAIL, EXIT_OK, EXIT_TRIGGERED, dur, registry, tile};

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
            eprintln!("quivive: {e:#}");
            std::process::exit(EXIT_FAIL);
        }
    }
}

fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::Tile { common, stream } => {
            if stream {
                // Stubbed until quivive-5uv; `?` turns its error into EXIT_FAIL
                // the same way every other unimplemented path does.
                quivive::stream::run(&common)?;
                Ok(EXIT_OK)
            } else {
                tick_once(&common)
            }
        }
        Command::Watch { interval, debounce } => {
            let opts = quivive::watch::WatchOptions {
                interval: dur::parse(&interval)?,
                debounce: dur::parse(&debounce)?,
            };
            quivive::watch::run(&opts)?;
            Ok(EXIT_OK)
        }
        Command::Why { repo, json } => {
            // Stubbed until quivive-15r.
            quivive::why::run(&repo, json)?;
            Ok(EXIT_OK)
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

    // No `--repo`: every path the registry names (S1-S2), and one bad entry
    // degrades to a quiet `no-fleet` row rather than taking the whole payload
    // down. An explicit `--repo`: just that path, and it is a real error if it
    // does not resolve — a human typed it, and there is no registry to degrade
    // around (see `tile::tick`'s doc comment).
    let (repo_roots, degrade_unreadable) = match &common.repo {
        Some(path) => (vec![path.clone()], false),
        None => (registry::read()?, true),
    };

    // One read of the clock per tick, taken here and passed down. See
    // `quivive::now` for why the seam exists and what it is for.
    let now = quivive::now()?;
    let payload = tile::build(
        &repo_roots,
        now,
        &thresholds,
        !common.no_cursor,
        degrade_unreadable,
    )?;

    let mut out = std::io::stdout().lock();
    if common.text {
        writeln!(out, "{}", payload.text())?;
    } else {
        // Pretty, and deliberately: a tile is read by people at least as often as
        // by bars, `jq` does not care, and a diff of two pretty tiles is legible
        // where a diff of two long lines is not — which the goldens depend on.
        writeln!(out, "{}", serde_json::to_string_pretty(&payload)?)?;
    }
    // Flush explicitly: `quivive tile --stream | head -1` closes the pipe, and
    // an ignored flush error there is a broken-pipe panic on exit.
    let _ = out.flush();

    if let Some(threshold) = common.exit_on
        && tile::severity(payload.status) >= tile::severity(threshold)
    {
        return Ok(EXIT_TRIGGERED);
    }
    Ok(EXIT_OK)
}
