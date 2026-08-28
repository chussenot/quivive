//! The repo registry: `~/.config/quivive/repos` (S1-S2 of `docs/spec.md`).
//!
//! Implemented in quivive-113. Stubbed here so `watch` and `stream` have a
//! module to depend on without waiting on that bead, and so `main.rs`'s wiring
//! never needs to change shape when the real reader lands.

/// Read the registry and return the repository paths it names.
///
/// Not implemented yet: see quivive-113.
pub fn run() -> anyhow::Result<()> {
    anyhow::bail!("not implemented yet (bead quivive-113)")
}
