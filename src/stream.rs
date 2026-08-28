//! `quivive tile --stream`: the pwetty push contract — spawn once, emit one
//! JSON line per CHANGE, stay alive, exit cleanly on stdout EOF (S9 of
//! `docs/spec.md`).
//!
//! Implemented in quivive-5uv. Stubbed here so the `--stream` flag exists on
//! `tile` in this bead and `main.rs`'s wiring never needs to change shape when
//! the real long-lived loop lands.

use crate::cli::Common;

/// Stream one tile line per change, following the pwetty push contract.
///
/// Not implemented yet: see quivive-5uv.
pub fn run(_common: &Common) -> anyhow::Result<()> {
    anyhow::bail!("not implemented yet (bead quivive-5uv)")
}
