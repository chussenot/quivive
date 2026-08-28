---
title: Decision records
status: active
date: 2026-08-28
description: Index of vigil's architecture decision records — what was decided, when, and which record supersedes which.
---

# Decision records

Every decision that would be expensive to reverse, or that a future reader would
otherwise undo for looking pointless, is recorded here. The format is fixed
(`docs/adr/NNNN-<slug>.md`, four sections, front matter naming the
decision-makers and what it supersedes) and `mise run check-docs` enforces it —
including the rule that **alternatives are priced**, because a decision whose
alternatives were never costed cannot be re-opened by anyone who was not in the
room.

A record is never edited to say something else. It is superseded by a new record,
which names it in `supersedes:`; the old one changes only its `status` to
`superseded` and gains a link forward.

| # | Decision | Status |
|---|---|---|
| 0001 | [The tile is a fold over a streamed ledger, not a query against an index](0001-stream-first-tile.md) | active |
| 0002 | [No daemon, and vigil does not draw](0002-no-daemon-renderer-boundary.md) | active |
| 0003 | [The YAGNI deferral register](0003-yagni-deferral-register.md) | active |

## Reading order

0001 and 0002 are one decision seen from two sides, and neither stands alone: the
streamed fold is what makes a one-shot process fast enough to be a tile, and the
absence of a daemon is what forbids hiding a slow fold behind a cache. 0003 is
the boundary the other two imply, written down with reversal conditions so it can
be argued with.

Start with 0002 if you want to know **what vigil is**; start with 0001 if you
want to know **why it can be that**.
