---
title: The YAGNI deferral register
status: active
date: 2026-08-28
decision-makers: [chussenot]
supersedes: none
---

# ADR-0003 — The YAGNI deferral register

## Context

"We are not building that" is the cheapest sentence in software and the least
durable. Six months later nobody remembers whether a feature was *rejected* or
merely *not yet reached*, so it gets built by whoever asks second — or, worse, a
capability the tool genuinely needs is refused out of loyalty to a decision whose
reasoning nobody can reconstruct.

vigil is unusually exposed to this, for two reasons. It is small, so almost any
proposed feature is large relative to it. And it sits next to three tools that
already own adjacent ground —
[pact](https://github.com/chussenot/pact) owns coordination,
[recount](https://github.com/chussenot/recount) owns testimony,
[agentic-db](https://github.com/chussenot/agentic-db) owns durable history — so
most of what vigil could grow into is something one of them is already better
placed to do. A refusal that does not say *who owns this instead* reads as a gap
in vigil rather than as a boundary.

## Decision

**Deferrals are recorded here, in one register, and a row is only valid if it
names the condition that would reverse it.**

A row with no reversal condition is not a deferral, it is a prejudice, and it
must be either given one or deleted. Reversing a row is a normal commit: strike
the row, say the condition was met, and — if the reversal changes how vigil is
shaped — open a new ADR that supersedes the relevant one.

| # | Deferred | Why not now | What reverses it |
|---|---|---|---|
| D1 | **History and queries over it** — "how long was agent-3 stale yesterday" | The tile is a recency question answered by a fold; history needs an index, and an index here is a second source of truth ([ADR-0001](0001-stream-first-tile.md)) | Nothing, in vigil. The reversal is that **agentic-db** grows the query, reading the same ledger. If that is somehow impossible, this row becomes a new ADR, not a feature |
| D2 | **A daemon or a socket API** | [ADR-0002](0002-no-daemon-renderer-boundary.md), in full | A measured tick that cannot meet the [per-tick ceiling](../spec.md#the-tick) on a realistic ledger *after* the fold has been optimised. Latency somebody can perceive on a 1 Hz bar, demonstrated, not predicted |
| D3 | **Notifications, alerts, hooks — anything vigil does rather than says** | vigil answers a question; acting on the answer is the caller's job, and a process that is not running cannot push. Exit codes are the cheap 90%: a script can already branch on "any agent DEAD" | Exit codes proving genuinely insufficient for a real script somebody wrote — with the script attached to the issue. Not "it would be nice if it beeped" |
| D4 | **A TUI** | pact already has one, and a fleet drill-down is *its* surface, next to the leases and messages a drill-down wants. A second TUI over the same repository is two things to keep in step | A drill-down that pact's UI structurally cannot host because it needs vigil's fold. Then it is a pact feature reading vigil, not a vigil TUI |
| D5 | **Network: remote fleets, a shared server, HTTP** | vigil reads one repository's files on one machine. There is no wire format because there is no wire | A fleet that actually runs across machines. The honest first answer is then a shared filesystem or agentic-db — a vigil server is the *last* resort, and would need its own ADR |
| D6 | **A config file** | Flags and environment variables cover it, and a config file is a third place a threshold can be set and therefore a third place it can be wrong | More than about eight knobs a typical user must set on every invocation. Count them before arguing |
| D7 | **A plugin or reader SPI** | The three readers are compiled in ([docs/spec.md](../spec.md#readers)). An SPI is an ABI, versioning, and a support surface, bought for zero known third-party readers | One reader that genuinely cannot live in this repository — a proprietary source, or a licence that forbids vendoring. A fourth in-tree reader is not a reason; it is a fourth reader |
| D8 | **Persistence beyond the resume cursor** | The cursor must be correct to throw away ([ADR-0001](0001-stream-first-tile.md)); anything else vigil keeps is state that can disagree with the ledger | Nothing keeps this row alive by itself — it is the invariant restated. A need for durable state is a need for agentic-db |
| D9 | **An MSRV promise, a platform matrix, prebuilt binaries** | The toolchain is pinned in `rust-toolchain.toml` and CI runs one job on one platform. A matrix is three times the minutes for a promise nobody has made | Somebody depending on vigil from a distribution package, or an install path that is not `cargo install`. Then the promise gets written down and CI grows to match it — in that order |
| D10 | **Guessing at agent liveness beyond what the readers can see** — heuristics over process tables, tty activity, editor state | The state machine is decided by evidence agents *write*. A heuristic over ambient machine state is unfalsifiable, unportable, and reports confidently when it is wrong | An agent runtime that publishes liveness vigil can *read* — a pact event, a pidfile with a contract. Evidence, not inference |

## Alternatives considered

**Keep the refusals in the README, as prose.** This is what pact and recount do,
and it works there because their refusals are stable properties of a finished
design. vigil's are a live queue: several of these rows are expected to be
reversed, and the README is exactly the wrong place for something that changes,
because a README is read once and never re-read. Rejected — but the README still
*summarises* the boundary and links here, because a reader deciding whether vigil
is the right tool must not have to open an ADR to find out.

**One ADR per refusal.** Rigorous, and the honest shape if each refusal carried
its own weight of context. Most of these are one sentence of reasoning and one
sentence of reversal condition; ten files of two sentences each would bury the two
rows that matter (D1 and D2) among eight that do not, and no reader would hold the
whole boundary in their head at once — which is the only useful way to hold it.
Rejected. A row that outgrows the table graduates to its own ADR, which is how D1
and D2 already became [0001](0001-stream-first-tile.md) and
[0002](0002-no-daemon-renderer-boundary.md).

**Track them as issues in the tracker.** Deferrals would then live where work
lives, and could be voted on. They would also be closed by whoever is tidying the
backlog, and a closed issue carries no reasoning to the person who asks again in
March. An issue is a request for work; this is a record of a decision, and the two
have different lifetimes. Rejected, though a *request* for one of these features
is of course an issue — one that should link to its row here.

**No register at all: decide each time it comes up.** Cheapest today. It also
means the answer depends on who is asked and how tired they are, which over ten
questions produces a tool with no boundary — the exact failure mode that makes
small tools grow into bad large ones. Rejected.

## Consequences

- **A feature request has a place to land.** The first question about any
  proposal is whether it is already a row here, and if it is, whether its reversal
  condition has been met. That is a five-minute conversation instead of a
  re-litigation.
- **This file is expected to shrink and churn**, unlike a normal ADR. Its
  `status` stays `active` while any row stands; individual rows are struck as they
  reverse, in a commit that says so.
- **Reversing a row can require a new ADR**, when the reversal changes vigil's
  shape rather than just adding a flag. D2 and D5 are both like that.
- **Every row names an owner elsewhere or an absent piece of evidence**, which is
  what stops the register reading as a list of things vigil is bad at. Where a row
  cannot name either, it is not ready to be a row.
- **The register is a house convention, not just this repository's**: it is the
  mechanism [docs/conventions.md](../conventions.md) points at for "we are not
  building that", and a sibling repository adopting these conventions is adopting
  this shape too.
