---
title: The tile contract
status: active
date: 2026-08-28
description: The versioned shape quivive emits — fields, text form, exit codes, and the rules for changing any of it.
---

# The tile contract

The tile is **quivive's API**. Not the CLI flags, not the text layout — the
payload. [ADR-0002](adr/0002-no-daemon-renderer-boundary.md) is why: quivive
emits a shape and somebody else draws it, so the shape is the thing consumers
depend on and the thing that must not move under them.

## This is a breaking reshape, and `v` does not move

The tile used to describe **one** repository (`repo`, `fleet`, `worst`,
`agents[]`, `blocked_leases`, `degraded`). It now describes the whole fleet —
S11: overall `status`, one entry per repository, agent counts and attention
items per entry. That is a removal, a rename and a meaning change all at once
— ordinarily the textbook case for moving `v`.

It stays `1` here for one reason: **no release of this crate has ever
shipped**, under either name (`vigil` or `quivive`), so there is no consumer
of the old shape for a version bump to protect. The old shape is not kept
below either, for the same reason a changelog does not carry a version that
was never released. The next reshape after this one, if there is ever a
consumer depending on this page, is a real `v: 2` and a real "previous shape"
section — this paragraph is what that decision will look like.

## The shape

Every example on this page is real `quivive tile` output, piped through
nothing — never hand-typed, never adjusted from memory after the surrounding
prose changed. That discipline is the point: a sibling repository has two
version examples that went stale exactly the other way.

This is `quivive tile` with a two-repository registry — one repo with a live
agent, one with a dead agent still holding a lease:

```json
{
  "v": 1,
  "at": "2026-08-28T09:00:00Z",
  "status": "human-needed",
  "repos": [
    {
      "name": "active",
      "path": "/home/user/repos/active",
      "status": "active",
      "agents": {
        "active": 1,
        "idle": 0,
        "stale": 0,
        "dead": 0
      },
      "attention": []
    },
    {
      "name": "humanneeded",
      "path": "/home/user/repos/humanneeded",
      "status": "human-needed",
      "agents": {
        "active": 0,
        "idle": 0,
        "stale": 0,
        "dead": 1
      },
      "attention": [
        {
          "kind": "dead_holding_paths",
          "agent": "ghost",
          "paths": [
            "src/fold.rs"
          ],
          "remaining_ttl": 0
        }
      ]
    }
  ]
}
```

(`path` above is shortened for the page; a real tile carries the absolute,
canonicalized path quivive actually read.)

| Field | Type | Meaning |
|---|---|---|
| `v` | integer | contract version. See [Changing the contract](#changing-the-contract) |
| `at` | RFC 3339, UTC | the clock this tick was computed against — **not** "now" at read time, and read exactly once for the whole payload |
| `status` | one of S8's five | the worst status across every repo — see [Overall status](#overall-status-and-severity) |
| `repos` | array | one entry per repository quivive was told about, in registry order. Empty when the registry is empty or missing (S1-S2) |
| `repos[].name` | string | the directory basename — what a bar has room to print |
| `repos[].path` | absolute path | the full, canonicalized path — what a human needs when two checkouts share a `name` |
| `repos[].status` | one of S8's five | this repo's own status, by the same precedence as the overall one |
| `repos[].agents` | counts | `active`/`idle`/`stale`/`dead`. Always all four keys, zeros included. Counts, not a per-agent list — S11 asks for counts, and `quivive why` (S21) is where a human goes for names |
| `repos[].attention` | array | S16-S18 items, empty in the normal case — see [Attention items](#attention-items) |

### Overall status, and severity

S8's five statuses, in precedence order — `human-needed` outranks everything
because it is the one status that actually asks a human to act right now;
`active` is next among the ones nobody needs to act on; `drained` outranks
`all-quiet` because it names a repo that *was* worked and stopped, which a
human skimming a multi-repo tile is more likely to wonder about than a repo
with no history at all; `no-fleet` is the floor.

```
human-needed > active > drained > all-quiet > no-fleet
```

The payload's top-level `status` is the worst of every `repos[].status` by
this order — an empty `repos` array (an empty or missing registry) has
nothing to take the max of and reads as `no-fleet`, the same "nothing to see"
a single repo with no pact in it gets:

```json
{
  "v": 1,
  "at": "2026-08-28T09:00:00Z",
  "status": "no-fleet",
  "repos": []
}
```

That is `quivive tile` run with `XDG_CONFIG_HOME` pointed at a directory with
no `quivive/repos` file in it — S2's "a missing registry file means an empty
registry, not an error" — and it is the shape `quivive-eea`'s acceptance
criteria names verbatim.

Nothing in this JSON shape encodes the severity order above — it is a fact
about the *renderer's* precedence, not a field. That is exactly why moving it
is invisible in a diff of the shape and has to be a deliberate, breaking call;
see [Changing the contract](#changing-the-contract).

### Attention items

S16-S18, one shape per kind, tagged by `kind`:

* **`dead_holding_paths`** (S16) — a DEAD agent holds one or more leases. One
  item per holder, not per lease: `agent`, the sorted `paths` it holds, and
  `remaining_ttl` (seconds, the minimum across those leases, clamped at 0 —
  never negative).
* **`needs_decision`** — a bead the committed sidecar flags as needing a
  human call: `bead_id`.
* **`gate_order_violation`** (S18) — work in a wave started before an earlier
  wave's declared gate closed: `started_id`, `started_wave`, `open_gate_id`,
  `gate_wave`. Only the earliest open gate blocking a given `started_id` is
  reported, so closing gates one at a time does not reshuffle the set.

Any non-empty `attention` array is what makes a repo's status `human-needed`
— S8's own first line. A `STALE` holder does **not** produce
`dead_holding_paths`; only `DEAD` does, which is why an otherwise-busy fleet
with one stale agent still reads as `active`, not `human-needed`.

`age_s`-shaped values are always integers, seconds, relative to `at` — never
a formatted string. A formatted age is a rendering decision: a bar that wants
`6m52s` can compute it from an integer; a bar handed `6m52s` and wanting
seconds cannot get back.

### Determinism, and the clock

A tick reads `at` exactly once for the whole payload, however many
repositories it covers — reading the clock per repo is a tile that cannot be
golden, and a payload whose `at` could disagree with what each repo was
actually judged against. `QUIVIVE_NOW` (RFC3339) freezes that read, and with
it frozen **two invocations over the same evidence produce byte-identical
output** — including one that resumed the cursor and one given
`--no-cursor`. A malformed `QUIVIVE_NOW` is an error, not a silent fall back
to the wall clock: a seam that quietly ignores a bad value would make every
comparison pass for the wrong reason.

The resume cursor lives at `.pact/quivive-cursor.json` per repository (the
crate was renamed from `vigil`; there is no migration from the old
`.pact/vigil-cursor.json` name, because a cache is correct to throw away).
Its path is part of this contract for one reason: **a consumer is entitled
to delete it, and deleting it must only ever cost time, never change the
tile** — the one invariant in [ADR-0001](adr/0001-stream-first-tile.md).
Its *contents* are not part of the contract and may change shape without
notice. `--no-cursor` does the same thing as deleting the file, without
touching it.

## The text form

`--text` prints one line per invocation, no trailing decoration — real
output, over the same two-repository registry as the JSON example above:

```
human-needed  2 repos: 1 active, 1 human-needed
```

The overall status comes first because it is what a glance is for; the
per-status repo counts follow. A single repo reads the same way with
singular grammar:

```
active  1 repo: 1 active
```

This is a bonus rendering on top of the payload — costs nothing extra to
compute — and is **not** part of the versioned contract S11 pins the way the
JSON shape is: it is not goldened field-by-field the way `v`, `status` and
`repos[]` are, and a future bar-focused reshape of it (truncation, a worst-repo
detail) would not by itself move `v`. It is always exactly one line, including
the empty-registry case:

```
no-fleet  no repos registered
```

## Exit codes

| Code | Meaning |
|---|---|
| 0 | a tile was produced — including an `all-quiet`, `no-fleet` or otherwise unremarkable one |
| 1 | quivive could not produce a tile at all (bad flags, an unreadable *explicit* `--repo`) |
| 2 | reserved for `--exit-on <status>`: a tile was produced *and* its overall status met or exceeded that one |

An explicit `--repo` that does not resolve is the one real error and exits 1
— it is not a registry entry, so there is nothing to degrade around. A
registry entry that fails to resolve, by contrast, degrades to a quiet
`no-fleet` entry for that one repo rather than failing the whole payload: one
bad line in `~/.config/quivive/repos` must not take the whole fleet's tile
down.

Code 2 is the alert path for a script that shells out to `quivive tile`
directly instead of running `quivive watch`'s own `notify-send` loop:
`quivive tile --exit-on human-needed || notify-send "fleet needs you"`, using
whatever notifier the caller already has configured. It is also what makes
[D3](adr/0003-yagni-deferral-register.md) — no hooks, no acting on a
transition beyond a one-way push — an honest deferral rather than a gap: both
this exit code and `watch`'s own notifications stop at saying, never doing.
`--exit-on` takes one of S8's five status names — the same
`human-needed`/`active`/`drained`/`all-quiet`/`no-fleet` vocabulary as
`status` above, not the four-state *agent* machine (`active`/`idle`/`stale`/
`dead`) those counts are made of; the two vocabularies are easy to confuse
because both derive from the same evidence, and only one of them is what a
human waiting on the whole fleet actually wants to threshold on.

## Changing the contract

`v` is an integer and it moves only for a **breaking** change. Two rules
decide which kind a change is:

- **Additive** — a new field, a new attention-item `kind`, a new entry in an
  array. `v` does not move. Every consumer is required to ignore fields it
  does not know, and this is the sentence that requires it.
- **Breaking** — removing a field, renaming one, changing a type, changing
  the *meaning* of a value, or changing severity order. `v` moves, and the
  previous shape stays documented on this page rather than being deleted
  from it.

The two that get misfiled most often, both in the breaking direction: a
meaning change that looks additive (the same field, the same type, and a
different number in every consumer — `remaining_ttl` measured from a
different reference point, say), and severity order (nothing in the schema
encodes it, so moving it is invisible in a diff, and every renderer's colour
comes from it).

`mise run tile-goldens` is the gate. A golden diff is not a failure to be
silenced; it is the prompt to decide which of the two kinds of change was
just made — regenerate (`UPDATE_GOLDENS=1 cargo test --test goldens`), read
the diff, and only then decide about `v` and this page. Regenerating a
golden to turn a red gate green without reading the diff first is how a
breaking change ships without anybody deciding to ship it.

The goldens live in `tests/goldens/` and are only possible because a tick is
a pure function of (evidence, clock, thresholds) —
[ADR-0001](adr/0001-stream-first-tile.md). Each golden is frozen evidence
plus a frozen `at`; a golden that needed a sleep, a real clock or a retry
would be evidence that purity has been lost, and the fix belongs in the fold,
never in the golden.

`tests/goldens/` holds two kinds of fixture, and only one is S13. Most of the
files (`declines.json`, `forget.json`, `expired_kinds.json`, and others)
exercise the reader/fold side directly — declines, the forget sweep, the
cursor invariant — and predate S13's canonical set. `tests/goldens/{all-quiet,
active, human-needed, drained, no-fleet}.json` are that canonical set, and the
JSON example above is shaped to match them.

### The sample-sync rule

S13's own sentence — samples verified in *both* repos — is a cross-repository
promise, not just a local one. `tests/goldens/{all-quiet, active, human-needed,
drained, no-fleet}.json` in **this** repo are the single source of truth;
`waybar-pwetty-box/tiles/quivive/samples/<name>.json` must be a byte-identical
copy of each. `tests/goldens.rs`'s own suite asserts that byte-identity
whenever the sibling repo is reachable (`QUIVIVE_PWETTY_SAMPLES_DIR`, or the
two repos checked out side by side as siblings) and skips cleanly, with the
reason printed, when it is not — a lone `quivive` clone has no sibling to
check against, and that absence is not a defect.

To change one of the five: regenerate here first
(`UPDATE_GOLDENS=1 cargo test --test goldens`), read the diff — a question,
never a failure to silence, see [Changing the contract](#changing-the-contract)
above — then copy the regenerated file over pwetty's copy verbatim and run
`pwetty check quivive` there. Never hand-edit either copy out of sync with the
other; the two are the same fact recorded twice on purpose, and a hand-edit is
exactly the drift that makes recording it twice worthless.
