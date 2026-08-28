---
title: Beads on other laptops
status: active
date: 2026-08-28
description: How a second machine reaches this repository's task graph — the recipe, proven by simulation, and the boundary of what travels.
---

# Beads on other laptops

The task graph lives in a Dolt database that git ignores. What git carries is a
**Dolt file remote inside the repository** — `.beads-remote/`, content-addressed
chunks committed like any other file — so the graph travels with `git clone` and
needs no hosted remote and no credentials beyond the git ones you already have.

## The recipe

```
git clone <this repo> quivive && cd quivive
dolt clone "file://$PWD/.beads-remote" .beads/dolt/quivive
bd list
```

That is all of it. `bd` finds the reconstructed database through the committed
`.beads/config.yaml` and spawns its own proxied `dolt sql-server`; no `bd init`
on the second machine. Later, after a `git pull` brings a newer `.beads-remote`:

```
cd .beads/dolt/quivive && dolt pull local-file main
```

Writing back from the second machine is the same push this repository's
orchestrator runs — `dolt push local-file main` from `.beads/dolt/quivive`,
then commit `.beads-remote/` to git.

## Proven, not asserted

The recipe above was run against a fresh tempdir clone during the v0.1 fleet
run. `bd list --json` on the clone matched the primary **18 beads for 18**,
with exactly one row differing: the docs bead was `in_progress` on the primary
and `open` on the clone, because it had been claimed *after* the last
`dolt push`. That is the semantics to expect, and it is git's own: **a clone is
as fresh as the last push**, and bead churn between pushes is invisible until
the next one.

## The boundary (charter, restated where users look)

Only the **task graph** travels. Leases, liveness, the pwetty tile's registry
(`~/.config/quivive/repos`) and every other piece of coordination state stay
single-machine by charter — see the
[deferral register](adr/0003-yagni-deferral-register.md)'s multi-machine row.
pact never reads the Dolt store at all: it reads only the committed
`.beads/interactions.jsonl` sidecar, so nothing pact does gains a database
dependency because beads has one.

## Two footnotes from the field

- bd's native sync lane is Dolt data over the **git remote itself**
  (`git+https://github.com/chussenot/quivive`, refs under `refs/dolt/*`) — bd
  configured it unprompted at `bd init`. It returned HTTP 403 through the
  build container's ref-scoped git proxy and was left recorded as the laptops'
  lane: from a machine with ordinary GitHub credentials, `bd sync` over that
  remote should work as designed, and would make `.beads-remote/` redundant.
  If it does, prefer it, and retire the file remote deliberately.
- The bd build matters: pact's own repository uses **embedded** Dolt, which
  needs a CGO bd; the pure-Go build (`CGO_ENABLED=0 go build -tags
  gms_pure_go`) used in the container runs the **proxied-server** mode this
  repository is configured for. Either bd opens this repo's store; a pure-Go
  bd cannot open pact's.
