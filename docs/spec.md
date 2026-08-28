---
title: quivive v0.1 specification
status: active
date: 2026-08-28
description: The exact v0.1 scope — registry, tick, tile (stream-first), watch, why — with the deferrals pointed at the README.
---

# quivive v0.1

*Qui-vive*: the sentry's challenge. quivive is the fleet-presence layer of the
family — [pact](https://github.com/chussenot/pact) records what happened,
[recount](https://github.com/chussenot/recount) explains why,
[agentic-db](https://github.com/chussenot/agentic-db) watches the session,
**quivive tells the human WHEN TO LOOK**. Read-only, compositor-agnostic, no
daemon-as-coordinator, no hooks, no SQLite, nothing agentic-db already does.
Each numbered line below is quotable; a bead's acceptance criteria quote the
lines they satisfy.

## Registry

- S1. The registry is plain text at `~/.config/quivive/repos` (honouring
  `XDG_CONFIG_HOME`), one repository path per line, hand-edited; blank lines and
  `#` comments are ignored, `~` expands.
- S2. There are no registry subcommands; a missing registry file means an empty
  registry, not an error.

## Tick

- S3. A tick is file reads only — no subprocess, no network, no database.
- S4. Per repo the tick reads: `.pact/leases/*` and `.pact/activity/*`, the tail
  of `.pact/events.jsonl` (tolerant of garbage lines), `.pact/plan.json`, and
  bd's committed sidecar `.beads/interactions.jsonl` when present.
- S5. The tick is mtime-pruned: a repo none of whose source files changed since
  the last tick is skipped without being re-read.
- S6. Steady-state tick cost is under 10 ms per repo, release profile, enforced
  by `mise run bench`.
- S7. Per-agent state is the four-state machine ACTIVE/IDLE/STALE/DEAD of
  [ADR-0001](adr/0001-stream-first-tile.md), fed by newest evidence across
  leases, activity records and the events tail.
- S8. Per-repo status is derived, in precedence order: `human-needed` (any
  attention item, S16–S19), else `active` (any ACTIVE/IDLE agent), else
  `drained` (a plan or recent fleet evidence exists but no live agent remains),
  else `all-quiet` (pact present, nothing moving), else `no-fleet` (no pact
  state at all).

## Tile (stream-first)

- S9. `quivive tile --stream` follows the pwetty push contract: spawn once,
  emit exactly one JSON line per CHANGE in the payload, stay alive between
  changes, exit cleanly on stdout EOF; pwetty keeps the last content and
  respawns after ~1 s.
- S10. `quivive tile` (one-shot) prints the same payload once and exits 0.
- S11. The payload is one JSON object: overall `status` (one of S8's five),
  per-repo entries with agent counts and attention items, `v` for the contract.
- S12. The tile ships as a contribution in waybar-pwetty-box:
  `tiles/quivive/{schema.json, tile.json, samples/}` in the claude tile's exact
  house style, every schema property annotated REAL vs MOCK.
- S13. The samples are exactly: `all-quiet`, `active`, `human-needed`,
  `drained`, `no-fleet` — and golden tests verify them in BOTH repos: quivive
  asserts it can emit each sample byte-for-byte from a fixture, pwetty asserts
  the samples validate against `schema.json`.

## Watch

- S14. `quivive watch` sends `notify-send` notifications on TRANSITIONS only —
  an event fires when it becomes true, not while it stays true.
- S15. Notifications are debounced per (repo, event).
- S16. Event: a DEAD agent holds paths — the notification names the agent, the
  paths, and the remaining lease TTL.
- S17. Event: a needs-decision bead is filed (from the committed sidecar).
- S18. Event: a gate-order violation — work in wave N+1 started before wave N's
  declared gates closed, derived from `.pact/plan.json` plus the events tail.
- S19. Event: the fleet drained (S8's `drained` became true).
- S20. Every notification carries its follow-up command — `pact lease ls`,
  `bd show <id>`, or `recount explain --event-line N` — quivive points, the
  family answers.

## Why

- S21. `quivive why <repo> [--json]` lists the attention-worthy items for one
  repo, each with the evidence line(s) that produced it (file plus line or
  path).
- S22. The whole CLI is: `tile`, `watch`, `why`.

## Deferred

Each deferral and its reversal trigger lives in the README and the
[deferral register](adr/0003-yagni-deferral-register.md): plain-waybar output,
install/config generators, quiet hours, master-red detection, multi-machine
anything, and registry subcommands (S2's "iff hand-editing bites").
