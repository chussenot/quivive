---
title: No daemon, and quivive does not draw
status: active
date: 2026-08-28
decision-makers: [chussenot]
supersedes: none
---

# ADR-0002 — No daemon, and quivive does not draw

## Context

A status tile has two obvious implementations, and both are wrong in the same
way.

The first is a background service: something starts at login, watches the
repository, keeps the current state in memory, and answers over a socket. It
gives the fastest possible tick and the freshest possible answer.

The second is a bar plugin: quivive emits waybar's JSON, or tmux's format string,
or i3blocks' three lines, and the user pastes one line into their config and gets
colour and glyphs for free.

Both are the version a user asks for. Both make quivive responsible for something
that is not its question. And the family quivive joins has already priced the first
one: pact has no server, no daemon and no database, on the grounds that the
moment coordination needs a long-running process it becomes one more thing that
can crash, drift out of sync, or need babysitting.

There is a real requirement underneath the daemon idea, though, and it should not
be dismissed with it: some consumers want a *stream* of tiles rather than to
schedule their own polling, and a human still has to notice the stream changed —
staring at a bar is not "the human's continuous attention" that
[the README](../../README.md#the-problem) says the tool exists to replace.

## Decision

**Two rules, and they are separate rules that happen to travel together.**

**1. No daemon.** quivive's default mode is one shot: `quivive tile` starts, reads,
prints one tile, exits. The status bar is already a scheduler — it is *made of*
a scheduler — so shipping a second one inside quivive duplicates the one part of
the job the caller already does well. There is no service to install, nothing to
start before the bar works, and nothing whose crash produces a stale tile that
looks fresh.

Two foreground loops answer the streaming requirement, one per consumer, and
neither is a daemon by the same three tests: each runs in the foreground, is
owned by whoever started it, binds no socket, accepts no connections, and
serves no second consumer. When either dies its consumer finds out
immediately — a closed pipe or a process that stopped — which is precisely the
property a daemon does not have.

`quivive tile --stream` is the bar's loop: it writes one JSON line per *change*
to stdout and nothing else (S9), which is the tile contract with a spawn cost
paid once instead of once a second. `quivive watch` is the human's loop
(S14-S20): it writes nothing on a quiet tick and calls `notify-send` on a
transition, so a human away from the bar still finds out. Both fold the same
readers through the same pure `assess`; the only difference is which side of
"the human still has to look" each one is answering.

**2. quivive emits a contract; it does not draw, and `watch` points rather than
acts.** The unit quivive produces is the tile defined in
[docs/tile-contract.md](../tile-contract.md): a stable, versioned shape
describing the fleet. Colour, glyphs, truncation, ordering-for-looks and
bar-specific encodings are the renderer's business — the bar's config, or a
`--format` adapter that is a **pure function of the tile and nothing else**.
An adapter may not read the ledger, and may not know anything the tile does not
say.

`watch`'s notification is downstream of the same rule, not an exception to it:
title and body are built only from an [`AttentionItem`](../tile-contract.md#attention-items)
the tile would already carry, and the one thing every notification adds is a
follow-up *command* — `pact lease ls`, `bd show <id>` — that names who answers
rather than answering itself (S20). quivive never runs that command. Saying
"a dead agent holds this lease" and naming `pact lease ls` is pointing; running
`pact lease acquire --steal` on the human's behalf would be acting, and nothing
in this crate does that.

## Alternatives considered

**Daemon plus unix socket.** Fastest tick, freshest answer, and one place to keep
state. The cost is a lifecycle: start it, supervise it, notice when it dies,
decide what a client shows while it is down, version the socket protocol, and
handle two clients wanting different thresholds. That is a service, and a service
that caches ledger state is [ADR-0001](0001-stream-first-tile.md)'s rejected
index wearing a different hat. The resumable cursor already gets the tick cheap
enough that the daemon buys latency nobody can perceive on a one-second bar.
Rejected.

**A library, not a binary — let bars link quivive.** Correct in the abstract and
useless in practice: status bars shell out. waybar, tmux, i3blocks, polybar and
starship all run a command and read its output, and none of them will link a Rust
crate. A library would serve one hypothetical consumer and zero real ones.
Rejected, though the fold is of course library code *inside* the crate.

**Native support for one bar — pick waybar and do it beautifully.** Tempting,
because it is the author's bar and it would look finished on day one. It also
makes every other bar a second-class port of a shape chosen for waybar's
convenience, and it welds an aesthetic into the contract: once the tile *is*
waybar JSON, changing how it looks is a breaking change to what it means.
Rejected. Adapters, downstream of the contract, get the same result without the
welding.

**Emit only text and let everyone parse it.** No contract to version, no JSON to
maintain. It also makes every consumer a parser of prose, which means any change
to the wording — a pluralisation, a re-ordering — silently breaks somebody's bar.
The tile is machine-read; giving it a declared shape is cheaper than pretending
it does not have one. Rejected, but it is why the text form is *also* specified
rather than left to chance.

## Consequences

- **The tile is the API.** [docs/tile-contract.md](../tile-contract.md) is
  therefore a contract document with goldens behind it
  (`mise run tile-goldens`), not a description of current output.
- **`quivive tile` must be fast enough to be called at 1 Hz from cold**, every
  time, including process start. That constraint is what
  [ADR-0001](0001-stream-first-tile.md) pays for, and the two decisions fail
  together: a slow fold forces a daemon, and a daemon hides a slow fold.
- **No install step, no service file, nothing to supervise.** Setup is one line
  in a bar's config, and uninstalling is deleting it.
- **Colour and glyphs are somebody else's taste.** quivive will accumulate example
  configs (in the docs, for real bars) rather than rendering code, and a bug
  report about how the tile *looks* is usually a bug report about an example
  config.
- **Two consumers wanting different thresholds is free**, because there is no
  shared process to disagree with: each invocation carries its own flags.
- **A push is no longer forbidden; acting on one still is.** This ADR's first
  draft refused notifications outright, on the reasoning that quivive is not
  running when nothing is asking it. `quivive watch` reverses that specific
  claim — a foreground loop the human started can push a local OS notification
  the moment a transition fires — without reversing the rule this ADR is
  actually about: quivive still only *says*, through `notify-send`, and never
  *does*. Hooks, webhooks, running the follow-up command automatically, and
  anything else that would turn a notification into an action remain deferred,
  in [ADR-0003](0003-yagni-deferral-register.md).
