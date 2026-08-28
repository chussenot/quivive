#!/usr/bin/env bash
# Run a command only if this repository has a crate yet; otherwise say so and
# succeed.
#
# This repository's conventions, documentation and decision records landed before
# its Rust crate did — that ordering was the point of the first run (see
# docs/studies/conventions-run.md). Every cargo task in mise.toml is therefore
# written exactly as it will run forever, and routed through here so that
# `mise run check` is green today on what exists rather than red on what does
# not. A gate that is red for reasons unrelated to the change under test is a
# gate people learn to ignore.
#
# The guard removes itself: the commit that adds Cargo.toml turns every one of
# those tasks on, with no edit to mise.toml and nothing for anyone to remember.
# It is loud on purpose — a silent skip is how a gate stops guarding.
#
# Usage: scripts/with-crate.sh <command> [args...]
set -euo pipefail

cd "$(dirname "$0")/.." || exit 1

if [ ! -f Cargo.toml ]; then
	printf 'with-crate: SKIPPED (no Cargo.toml in this repository yet): %s\n' "$*" >&2
	exit 0
fi

exec "$@"
