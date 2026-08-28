//! `quivive why <repo> [--json]`: the attention-worthy items for one repo, each
//! with the evidence line(s) that produced it (S21 of `docs/spec.md`).
//!
//! Implemented in quivive-15r. Stubbed here so the CLI surface (S22: "the whole
//! CLI is tile, watch, why") is complete in this bead and `main.rs`'s wiring
//! never needs to change shape when the real command lands.

use std::path::Path;

/// List the attention items for `repo`, in JSON if `json` is set.
///
/// Not implemented yet: see quivive-15r.
pub fn run(_repo: &Path, _json: bool) -> anyhow::Result<()> {
    anyhow::bail!("not implemented yet (bead quivive-15r)")
}
