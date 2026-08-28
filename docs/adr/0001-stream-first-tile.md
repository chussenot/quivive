---
title: The tile is a fold over a streamed ledger, not a query against an index
status: active
date: 2026-08-28
decision-makers: [chussenot]
supersedes: none
---

# ADR-0001 — The tile is a fold over a streamed ledger, not a query against an index

## Context

quivive answers one question — *is the fleet alive right now?* — and it answers it
on somebody's status bar, which means it is invoked on a timer, roughly once a
second, forever. The input is [pact](https://github.com/chussenot/pact)'s
`.pact/events.jsonl`: append-only, one JSON object per line, unbounded. A fleet
of eight agents working for a day produces tens of thousands of lines; a
long-lived repository produces hundreds of thousands.

Two properties of the question shape everything else:

- **It is about recency, not history.** Every state in
  [the state machine](../spec.md#tick) is decided by *how old* an
  agent's newest event is. Nothing in the tile needs an event from last Tuesday.
- **It is a reduction.** The answer is a fixed-size summary — a handful of
  per-agent states — no matter how many events produced it.

A reduction over an append-only log with a recency-only question is the exact
shape a streaming fold fits. The temptation is to reach for storage anyway,
because storage is what one reaches for when a file gets big.

## Decision

**The tile is computed by folding events forward from a resumable cursor. quivive
holds no index, no schema, and no copy of the ledger.**

Concretely:

- Each tick opens the ledger, seeks to a saved byte offset, reads only the bytes
  appended since the last tick, and folds each new event into the accumulator —
  the per-agent state described in [docs/spec.md](../spec.md).
- The cursor and the folded accumulator are the only things quivive persists,
  in one small file under `.pact/` (the exact path is part of
  [the tile contract](../tile-contract.md)).
- **The cursor must be correct to throw away.** Deleting it and re-reading the
  whole ledger must produce a byte-identical tile. That is not an aspiration; it
  is the invariant `mise run fleet` exists to measure, by ticking a control fleet
  with the cursor deleted before every tick and asserting the two runs agree.
- Rotation and truncation are detected structurally — the file shrank, or its
  inode changed — and fall back to a full re-read rather than to a guess.
- A tick is a **pure function of (ledger bytes, clock, thresholds)**. No sleeps,
  no ambient time other than the clock passed in, no network.

## Alternatives considered

**Read the whole ledger every tick.** The simplest thing that works, and it does
work — for a while. Cost is O(file) per tick: at 1 Hz over a 100k-event ledger
that is re-parsing tens of megabytes a second to answer a question about the last
sixty seconds of it. It also scales the wrong way against the thing that makes
quivive useful, since the fleets worth watching are the busy ones. Rejected on
cost, not on correctness — and kept as the *fallback* path, which is why the
invariant above is expressible at all.

**Index the ledger in SQLite (or in
[agentic-db](https://github.com/chussenot/agentic-db)).** Fast queries, and it
would make history free. It also buys a schema, migrations, a writer that can
crash halfway, and — the real cost — **a second source of truth that can
disagree with the ledger**. Every bug class in that list is a bug class quivive
would then own forever, in service of a question that never needs a query. And
agentic-db already owns durable history for this family: a second index here
would duplicate it while being worse at it. Rejected; the boundary is recorded
in [ADR-0002](0002-no-daemon-renderer-boundary.md) and
[ADR-0003](0003-yagni-deferral-register.md).

**Read the tail only — the last N lines, or the last N kilobytes.** Cheap, and
almost right, which is the dangerous kind of wrong. A tail cannot see an agent
whose newest event fell off the window, so that agent silently disappears from
the tile rather than being reported DEAD — the single most important thing quivive
has to say. Rejected on correctness.

**Have pact maintain a status file quivive reads.** Cheapest of all at tick time,
and it puts a rendering concern inside the coordinator, which then has to know
what a status bar wants. It also means the answer is only as fresh as pact's last
write, and pact deliberately has no daemon to keep it fresh. Rejected; it moves
the problem rather than solving it, and it couples two tools that are better kept
apart.

## Consequences

- **Ticking at 1 Hz is affordable, and that is what makes quivive a tile at
  all.** Per-tick cost is O(events appended since the last tick), which on an
  idle fleet is zero events and one `stat` — now paid once **per repository**
  the registry names, not once per invocation: `quivive tile`'s fold is `S1-S2`'s
  fan-out over one cursor per repo, and an idle fleet's registry entry costs the
  same one `stat` whether it is the only one registered or the twentieth.
- **The cursor is a cache, never a record.** Any bug that can only be fixed by
  keeping more state in it should be read as evidence the fold is wrong.
  `mise run fleet` is the standing proof, and `mise run bench` measures the
  ceiling in [docs/spec.md](../spec.md#tick). The cursor file is
  `.pact/quivive-cursor.json` per repository — renamed from the pre-rename
  crate's `.pact/vigil-cursor.json` (quivive-qeg) — and the rename needed no
  migration: an old file under the old name simply reads as no cursor, which is
  exactly what deleting it by hand already produces, and this ADR's own
  invariant is the proof that outcome is always correct.
- **The started-bead set (S18) is folded through the same cursor, not
  re-derived.** `quivive-374` added `Cursor.started`: a `BTreeSet` of bead ids
  seen naming an `acquired` row anywhere in the ledger, folded incrementally by
  the same resumed read that already folds per-agent evidence, rather than
  re-scanning `events.jsonl` for it on every tick. It is a plain union across
  resumed reads the same way the agent accumulator already is, so it costs
  nothing extra to keep correct to throw away.
- **Goldens are possible.** Because a tick is pure, the tile for a frozen ledger
  at a frozen clock is a fixed string, so the contract can be pinned by
  `mise run tile-goldens` instead of by prose. A golden test that needs a sleep
  is a report that this decision has been violated.
- **quivive cannot answer a question about the past.** "How long was agent-3
  stale yesterday" has no cursor position that answers it. That is deliberate,
  it is agentic-db's question, and the deferral is registered with its reversal
  condition in [ADR-0003](0003-yagni-deferral-register.md).
- **A reader that is not append-only does not fit this design.** Lease, activity,
  plan and sidecar state are read whole each tick because they are small and
  mutable; the ledger is the only streamed input. A future reader over another
  growing log must bring its own cursor, and must satisfy the same
  throw-it-away invariant.
- **The worktree reader this ADR's first draft assumed was retired, not
  built.** Inferring liveness from the mtime of a leased path turned out to
  credit an agent with a `git checkout` it never did; `docs/spec.md`'s
  `activity` reader replaced it with pact's own per-invocation trace — evidence
  the agent wrote, not evidence inferred about it. The point above about "a
  reader that is not append-only" still holds; only the identity of that third
  reader changed.
