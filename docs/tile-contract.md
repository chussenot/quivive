---
title: The tile contract
status: active
date: 2026-08-28
description: The versioned shape vigil emits — fields, text form, exit codes, and the rules for changing any of it.
---

# The tile contract

Written before the implementation, and implemented unchanged — which was the
point of writing it first: the goldens had something to be golden against.

The tile is **vigil's API**. Not the CLI flags, not the text layout — the tile.
[ADR-0002](adr/0002-no-daemon-renderer-boundary.md) is why: vigil emits a shape
and somebody else draws it, so the shape is the thing consumers depend on and the
thing that must not move under them.

## The shape

Both examples on this page are the `mixed` golden verbatim
(`tests/goldens/mixed.json` and `.txt`), not prose written to look like output.
`REPO` is that fixture's placeholder for the repository path; a real tile carries
an absolute path there. Sample output on this page is never hand-edited — a
sibling repository has two version examples that went stale exactly that way,
because somebody updated the surrounding prose and adjusted the sample from
memory.

```json
{
  "v": 1,
  "at": "2026-08-28T09:00:00Z",
  "repo": "REPO",
  "fleet": {
    "active": 3,
    "idle": 3,
    "stale": 1,
    "dead": 1,
    "total": 8
  },
  "worst": "dead",
  "agents": [
    {
      "id": "agent-6",
      "state": "dead",
      "age_s": 2400,
      "leases": []
    },
    {
      "id": "agent-3",
      "state": "stale",
      "age_s": 412,
      "leases": [
        "src/fold.rs"
      ]
    },
    {
      "id": "agent-5",
      "state": "idle",
      "age_s": 250,
      "leases": []
    },
    {
      "id": "agent-4",
      "state": "idle",
      "age_s": 120,
      "leases": []
    },
    {
      "id": "agent-8",
      "state": "idle",
      "age_s": 60,
      "leases": []
    },
    {
      "id": "agent-7",
      "state": "active",
      "age_s": 59,
      "leases": []
    },
    {
      "id": "agent-2",
      "state": "active",
      "age_s": 30,
      "leases": []
    },
    {
      "id": "agent-1",
      "state": "active",
      "age_s": 5,
      "leases": [
        "src/tile.rs"
      ]
    }
  ],
  "blocked_leases": [
    {
      "path": "src/fold.rs",
      "held_by": "agent-3",
      "expired_s": 112
    }
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
| `degraded` | array of strings | readers that could not read, and decline counts, named. Empty is the normal case |

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

### What `degraded` can say

Two shapes, both plain strings, and both additive — a new reason does not move
`v`, so a consumer must not enumerate them exhaustively:

* `"ledger"` — the reader could not read at all. There is no `"lease"` equivalent:
  a missing leases directory is a repository's resting state, not a fault.
* `"<reader>: N unparsable line(s)"` / `"... lock(s)"` — a decline count. Declines
  are counted rather than swallowed because a decline count nobody knows about is
  this reader's most likely undetected defect. A blank line is **not** a decline
  (pact's compaction can leave one), and neither is pact's staging sibling beside
  the lock files (a tick landing mid-acquire will see it).

### Determinism, and the clock

A tile carries the instant it was computed at, so two invocations a millisecond
apart are legitimately different tiles. `VIGIL_NOW` (RFC3339) freezes the clock,
and with it frozen **two invocations over the same ledger produce byte-identical
output** — including one that resumed a cursor and one given `--no-cursor`.

That is not a convenience for tests. It is the only way to state the purity claim
of [ADR-0001](adr/0001-stream-first-tile.md) about the *binary* somebody runs
rather than about the library, and it is what `scripts/fleet-sim.sh` asserts. A
malformed `VIGIL_NOW` is an error rather than a silent fall back to the wall
clock: a seam that quietly ignores a bad value makes every comparison pass for the
wrong reason.

## The text form

`vigil tile` without `--json` prints exactly one line, no trailing decoration:

```
3A 3I 1S 1D  worst=dead  agent-6 dead 40m0s
```

The counts come first because they are what a glance is for. The detail after
`worst=` names the single worst agent, **and only when somebody should look at
it** — naming the worst agent on a healthy fleet would spend the one line of room
on the least interesting fact in the tile. `(holds <path>)` is appended when that
agent is sitting on a lease, with `+N` for the rest.

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

The resume cursor is `.pact/vigil-cursor.json`, and its path is part of this
contract for one reason: **a consumer is entitled to delete it, and deleting it
must only ever cost time.** Its contents are not part of the contract and may
change shape without notice, because it is a cache — see
[ADR-0001](adr/0001-stream-first-tile.md). `--no-cursor` does the same thing
without touching the file.

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
