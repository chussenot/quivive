---
title: Focus is emphasis, not filtering
status: active
date: 2026-08-29
decision-makers: [chussenot]
supersedes: none
---

# ADR-0004 — Focus is emphasis, not filtering

## Context

pwetty's bundled `claude` tile and quivive's tile share a bar and know nothing
about each other. The `claude` tile knows which niri desktop is focused and which
Claude session sits on it; the `quivive` tile renders every registered repository
identically no matter where the human is looking.

The obvious improvement is to join them: make the quivive tile show the fleet for
the repository the focused session is working in. [The measurements](../studies/focus-aware-tile.md)
say this is possible — the focus signal is published as a plain file
(`~/.local/state/claude-status/tiles.json`, `active: true` on the focused
desktop), costs 0.033 ms to parse, and fits the streaming tick with no structural
change.

They also say the obvious version of it is wrong, for two reasons that are
independent of each other.

**Most focus targets are not fleets quivive watches.** Three of five desktops on
the machine this was measured on resolve to nothing in the registry — two are
real repositories deliberately not registered, one is an ordinary window with no
session at all. A tile that filters to the focused repository has no answer 60%
of the time, and "no answer" is the state the tile is supposed to *report about
the fleet*, not enter itself.

**Filtering answers a question the tool already answers.** quivive's stated
question is whether *the fleet* is alive right now. `quivive tile --repo <path>`
already renders exactly one repository, and anyone who wants a single-repo tile
can run one today. Making the shared tile follow focus does not add a capability;
it trades a fleet view for a single-repo view that already exists, on a bar where
there is room for both.

There is a third constraint that shapes the form of the answer rather than the
answer itself. [ADR-0002](0002-no-daemon-renderer-boundary.md) draws a hard line:
quivive emits data and does not draw. Which repository is accented, whether the
accent is a colour or a glyph or a leading marker, is a rendering decision that
belongs to pwetty's template — not to the producer.

## Decision

**quivive may report which watched repository is focused. It will not filter to
it, and it will not decide how focus looks.**

Concretely:

1. An optional sixth reader resolves the focused repository from
   `tiles.json` — a file read, so [S3](../spec.md#tick) holds — and sets
   `focused: true` on **at most one** entry of the existing `repos` array. Every
   repository stays in the payload, in registry order, exactly as today.
2. It is **additive** under the [tile contract](../tile-contract.md#changing-the-contract):
   a new field, so `v` does not move and every existing consumer keeps working
   without knowing the field exists.
3. Absence is never an error, mirroring [S2](../spec.md#registry)'s treatment of a
   missing registry. No `tiles.json`, an unparseable one, a focused desktop with
   no session, or a session in an unregistered repository all produce the same
   result: no entry carries `focused`, and the tile renders as it does today.
4. **Resolution is by absolute path, or it does not happen.** The current file
   publishes only a basename, which cannot distinguish `~/work/pact` from
   `~/Documents/pact`. Until the producer publishes `cwd`, this reader stays
   dark. Guessing from a basename is the inference over ambient state that
   [D10](0003-yagni-deferral-register.md) refuses, and a confidently wrong fleet
   is worse than no accent.
5. The rendering — accent, marker, ordering, or ignoring the field entirely — is
   pwetty's, per ADR-0002.

Point 4 makes this decision **conditional**: it authorises the shape, and the
work does not start until the upstream file carries a path. That is deliberate.
The alternative is shipping a heuristic now and removing it later, and this
project's register exists to stop exactly that trade.

## Alternatives considered

**Filter the tile to the focused repository.** The version originally asked for.
Cost: no answer for 60% of focus targets as measured, and the failure is silent —
a blank or single-row tile looks like a quiet fleet rather than an unresolved
lookup. Also destroys the fleet-wide question the tile exists to answer, in
exchange for a single-repo view `--repo` already provides. Price: one flag's
worth of code, and the tile's reason to exist. Rejected.

**Read `claude.sqlite` directly** and get `sessions.cwd` and `repos.root_path`
with no upstream change, resolving 100% of focus targets immediately. Cost: S3
forbids a database on the tick path in as many words, and this would be the first
violation — on the hot path, against a WAL-mode database written by a live daemon,
which also makes the tick's cost depend on another process's write pattern. Price:
one query, and the spec boundary that keeps a tick honest. Rejected; the boundary
is worth more than the 60%.

**Match on the basename anyway**, accepting the ambiguity because this machine has
no collision today. Cost: correct until someone clones a second `pact`, then
silently renders the wrong fleet with no signal that it did. Price: zero code
today, an unfalsifiable bug later. Rejected — D10, and the fact that "correct on
one machine on one day" is the shape of every measurement this project has had to
retract.

**Publish focus from quivive to pwetty via a new channel** (socket, IPC, a status
file quivive writes). Cost: a daemon or a second writer, both refused outright by
ADR-0002 and D2, to move a fact that already travels in a file both processes can
read. Price: the entire no-daemon boundary. Rejected without further costing.

**Do nothing.** Cost: zero. The tile keeps answering the fleet question; anyone
wanting single-repo focus runs a second module with `--repo`. This remains the
honest fallback if the upstream change in point 4 never happens, and the register
records no reversal condition for it because it *is* the resting state.

## Consequences

The tile gains an optional field and loses nothing. Consumers that ignore
`focused` — including today's pwetty template — are unaffected, which is what
makes this additive rather than a `v` bump.

**The work is blocked on a producer this repository does not own.** `tiles.json`
must carry the session's `cwd` alongside `folder` before the reader can be
written. That is a change to `claude-status`, filed separately; until it lands,
this ADR describes a decision taken and not a feature shipped, and the tile
behaves exactly as it does today.

quivive acquires a soft dependency on another tool's file format the first time
this reader exists. It is soft by construction — absence is a no-op, not an
error — but it is real, and it is the first time quivive reads anything a
non-pact tool writes. The reader count going from five to six is itself
unremarkable; [D7](0003-yagni-deferral-register.md) already notes that count has
moved once and declines to build an SPI for it.

The two failure modes that remain are both benign and both silent: a focused
repository outside the registry, and a focused desktop with no session. Neither
produces an error, and neither is distinguishable in the payload from the other.
If that ambiguity ever needs resolving, it needs a second field, not a different
value in this one.
