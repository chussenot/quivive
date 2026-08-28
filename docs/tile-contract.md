---
title: The tile contract
status: draft
date: 2026-08-28
description: The versioned shape vigil emits — fields, text form, exit codes, and the rules for changing any of it.
---

# The tile contract

`status: draft`: this is the contract the implementation is written against, not
a transcript of a binary you can run. Everything here is intended to become
`active` unchanged, because the point of writing it first is that the goldens
have something to be golden against.

The tile is **vigil's API**. Not the CLI flags, not the text layout — the tile.
[ADR-0002](adr/0002-no-daemon-renderer-boundary.md) is why: vigil emits a shape
and somebody else draws it, so the shape is the thing consumers depend on and the
thing that must not move under them.

## The shape

```json
{
  "v": 1,
  "at": "2026-08-28T09:41:07Z",
  "repo": "/home/user/quivive",
  "fleet": { "active": 5, "idle": 2, "stale": 1, "dead": 0, "total": 8 },
  "worst": "stale",
  "agents": [
    { "id": "agent-3", "state": "stale", "age_s": 412, "leases": ["src/fold.rs"] }
  ],
  "blocked_leases": [
    { "path": "src/fold.rs", "held_by": "agent-3", "expired_s": 0 }
  ],
  "degraded": []
}
```

| Field | Type | Meaning |
|---|---|---|
| `v` | integer | contract version. See [Changing the contract](#changing-the-contract) |
| `at` | RFC 3339, UTC | the clock this tick was computed against — **not** "now" at read time |
| `repo` | absolute path | which repository this tile describes |
| `fleet` | counts | one count per state, plus `total`. Always all five keys, zeros included |
| `worst` | state name | the highest-severity state present, or `"quiet"` when `total` is 0 |
| `agents` | array | one entry per remembered agent, ordered by severity then by age descending |
| `blocked_leases` | array | leases held by a `STALE` or `DEAD` agent — the actionable subset |
| `degraded` | array of strings | readers that could not read, named. Empty is the normal case |

Two field choices carry most of the weight:

**`worst` exists so that a renderer never has to reimplement severity ordering.**
A bar wants one colour, and every bar deriving "which colour" from four counts is
four chances to disagree with vigil about what matters. Severity order is
`dead > stale > idle > active > quiet`, decided here, once.

**`degraded` is a field, not a log line, and never an error.** A repository with
no `.pact/` is not broken, and a tile that exits non-zero because one reader found
nothing would take a status bar down over a normal condition. Naming the reader in
the tile lets a renderer show a dimmed tile instead of an empty one, which is the
difference between "nothing is running" and "I cannot see".

`age_s` is seconds, integer, relative to `at` — not a formatted string. A
formatted age is a rendering decision, and a bar that wants `6m` can compute it;
a bar that gets `6m` and wants seconds cannot.

## The text form

`vigil tile` without `--json` prints exactly one line, no trailing decoration:

```
5A 2I 1S 0D  worst=stale  agent-3 stale 6m52s (holds src/fold.rs)
```

The text form is **also** a contract, for the reason
[ADR-0002](adr/0002-no-daemon-renderer-boundary.md#alternatives-considered)
gives: consumers parse it whether or not they are invited to, so it is cheaper to
declare a shape than to pretend one does not exist. It is pinned by the same
goldens as the JSON.

It is one line, always, including when nothing is running (`quiet`) and when every
reader failed (`unreadable: ledger`). A status bar has one line of room; a tool
that sometimes needs two has no way to say so.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | a tile was produced — including a `quiet` or `degraded` one |
| 1 | vigil could not produce a tile at all (bad flags, unreadable repo path) |
| 2 | reserved for `--exit-on <state>`: a tile was produced *and* it met or exceeded that state |

Code 2 is what makes [D3](adr/0003-yagni-deferral-register.md) — no
notifications — an honest deferral rather than a gap: `vigil tile --exit-on dead
|| notify-send "fleet down"` is the 90% case in one line, using the caller's own
notifier, which is already configured the way the caller likes.

## Changing the contract

`v` is an integer and it moves only for a **breaking** change. Two rules decide
which kind a change is:

- **Additive** — a new field, a new `degraded` reason, a new entry in an array.
  `v` does not move. Every consumer is required to ignore fields it does not know,
  and this is the sentence that requires it.
- **Breaking** — removing a field, renaming one, changing a type, changing the
  *meaning* of a value, or changing severity order. `v` moves, and the previous
  shape is documented on this page rather than deleted from it.

`mise run tile-goldens` is the gate. A golden diff is not a failure to be
silenced; it is the prompt to decide which of the two kinds of change you just
made. Any commit that moves `v` must move this page in the same commit — that is
a review rule, and the reason it can be one is that a golden diff makes the
question unmissable.

The goldens live in `tests/goldens/` and are only possible because a tick is a
pure function of (ledger, clock, thresholds) —
[ADR-0001](adr/0001-stream-first-tile.md). Each golden is a frozen ledger plus a
frozen `at`, so a golden that needs a sleep, a real clock, or a retry is evidence
that purity has been lost, and the right fix is in the fold, not in the golden.
