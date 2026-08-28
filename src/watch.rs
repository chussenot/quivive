//! `quivive watch`: `notify-send` on transitions only, debounced per
//! (repo, event) — S14-S20 of `docs/spec.md`.
//!
//! Implemented in quivive-8mq. Stubbed here so the CLI surface (S22: "the whole
//! CLI is tile, watch, why") is complete in this bead and `main.rs`'s wiring
//! never needs to change shape when the real command lands. The old
//! interval-loop `watch` (print a tile every tick) is gone with it: S14's watch
//! fires on events, not on a timer, so there is nothing in the old behaviour
//! worth keeping.

/// Watch the registry and notify on transitions.
///
/// Not implemented yet: see quivive-8mq.
pub fn run() -> anyhow::Result<()> {
    anyhow::bail!("not implemented yet (bead quivive-8mq)")
}
