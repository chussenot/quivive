---
title: Can the tile follow the focused desktop
status: active
date: 2026-08-29
description: Measurements against the live claude-status daemon and pwetty bar on this laptop — what focus signal actually exists, what it costs to read, where it resolves to a repository and where it does not, and the one finding that decided ADR-0004.
---

# Can the tile follow the focused desktop

The `quivive` tile renders every repository in the registry, always the same
way, regardless of what the human is looking at. The bar directly above it —
pwetty's bundled `claude` tile — already knows which niri desktop is focused and
which Claude session is on it. The question this page answers is whether quivive
can use that, and what it would cost.

**The short answer is that the signal exists, is cheap, and is a file read — and
that the obvious use of it is wrong.** Three of five focused desktops on this
machine point at a repository quivive does not watch, so a tile that *filtered*
to the focused repo would be blank or undefined most of the time. That single
measurement is what [ADR-0004](../adr/0004-focus-is-emphasis-not-filtering.md)
turns on.

Everything below was measured on 2026-08-29 against the running daemon
(`claude-status tile-watch`), the live `~/.config/quivive/repos`, and the
installed waybar config — not against fixtures.

## What is already wired

```
claude-status (daemon)  --writes-->  ~/.local/state/claude-status/tiles.json
                        --writes-->  ~/.local/state/claude-status/claude.sqlite

waybar cffi/pwetty#claude   exec: claude-status tile-watch --output eDP-1 <N>
waybar cffi/pwetty#quivive  exec: quivive tile --stream
```

The two tiles sit on the same bar, read from two different producers, and share
no state. `quivive tile --stream` is a single long-running process that knows
nothing about desktops.

## Experiment 1 — does a focus signal exist, and is it a file

`tiles.json` is one object per `output:workspace`. The focused desktop carries
`active: true`; the others omit the key entirely.

```console
$ python3 - <<'PY'
import json; d=json.load(open('~/.local/state/claude-status/tiles.json'))
for k,v in d.items():
    if v.get('active'): print(k, [s.get('folder') for s in v.get('sessions',[])])
PY
eDP-1:1 ['waybar-pwetty-box']
```

**Result: yes, and it is a plain file.** That matters more than it sounds.
[S3](../spec.md#tick) forbids a subprocess, a network call or a database on the
tick path, and the same daemon publishes the same facts *twice* — once as this
JSON file and once as `claude.sqlite`. Only one of those two is readable from a
tick without breaking the spec.

## Experiment 2 — what it costs on the tick path

```console
$ stat -c%s tiles.json
642
$ # 100 parses
0.033 ms/parse
```

**Result: 642 bytes, 0.033 ms per parse.** Against the
[per-tick ceiling](../spec.md#tick) this is noise — roughly three hundredths of
a millisecond added to a tick that already reads a lease directory, an activity
directory and a ledger tail per repository. Cost is not the reason to say no to
anything on this page.

## Experiment 3 — does a focused desktop resolve to a repository

This is the experiment that decided the ADR.

`tiles.json` carries `sessions[].folder`, documented in pwetty's own schema as
*"REAL (basename of sessions.cwd)"*. A basename, not a path. The full path
exists — `sessions.cwd` and `repos.root_path` in the sqlite — but that is the
copy a tick may not read (Experiment 1).

So resolution has to go through something quivive already has, and it has
exactly one candidate: the registry, which is a list of absolute paths.

```console
registry basenames:  {quivive: /home/chussenot/Documents/quivive,
                      pact:    /home/chussenot/Documents/pact}

folders seen across the five desktops:
  waybar-pwetty-box  -> NOT IN REGISTRY
  pact               -> /home/chussenot/Documents/pact
  quivive            -> /home/chussenot/Documents/quivive
  agentic-db         -> NOT IN REGISTRY
  (none)             -> NOT IN REGISTRY   # a non-Claude window, is_claude:false
```

**Result: 2 of 5 resolve. 3 of 5 do not.**

Two distinct failure shapes hide in that 3, and they are not the same problem:

1. **A real repository quivive does not watch** (`waybar-pwetty-box`,
   `agentic-db`). The human is working somewhere quivive has no opinion about.
   This is the normal case, not an error — the registry is a deliberate subset
   ([S1-S2](../spec.md#registry)), and D15 in the
   [deferral register](../adr/0003-yagni-deferral-register.md) keeps it that way.
2. **No repository at all** — an ordinary window, `is_claude:false`, no
   `sessions` array. There is nothing to resolve.

A tile that answers "show me the focused repository" has to have an answer for
both, and on this machine it needs one 60% of the time.

## Experiment 4 — is basename resolution even sound

Setting aside how often it resolves, is it *correct* when it does?

On this machine, no two registry entries share a basename, so the mapping is
unambiguous today. It is not unambiguous in general: `~/work/pact` and
`~/Documents/pact` are two different fleets with one basename, and the registry
is explicitly allowed to contain both. `tiles.json` gives no way to tell them
apart, because the daemon threw the path away before writing the file.

**Result: sound today, unsound by construction.** Any design that leans on
basename matching is one `git clone` away from silently rendering the wrong
fleet — which is worse than rendering none, because it is confidently wrong.
The fix belongs upstream (publish `cwd`, not just its basename), not in a
heuristic here; matching on a basename is precisely the "unfalsifiable inference
over ambient state" that D10 refuses.

## Experiment 5 — could quivive follow focus at all, given `--stream`

waybar runs `exec: quivive tile --stream`: one process, started once, emitting on
its own interval. A focus change is an event that happens outside it.

Nothing about that is blocking. The stream already re-reads its inputs every
tick; `tiles.json` would simply be one more input, re-read on the same schedule,
with a focus change visible on the next tick like any other state change.
[ADR-0001](../adr/0001-stream-first-tile.md)'s fold is unaffected — focus is not
history, it is a fact about right now, which is the only kind of fact the tile
carries.

**Result: no obstacle.** This is a "may", not a "cannot" — which is why the
decision had to be made on grounds other than feasibility.

## What this leaves

The signal exists (E1), costs nothing (E2), and is technically usable (E5).
It resolves to a watched repository 40% of the time (E3), and the resolution
mechanism is unsound in the general case (E4).

Those last two are the whole argument, and they point away from the feature as
originally posed. The reasoning from here — why the tile marks focus instead of
filtering to it, and why the mark is a field rather than a colour — is
[ADR-0004](../adr/0004-focus-is-emphasis-not-filtering.md).

## Still unmeasured

- **Focus-change latency.** How long between a niri workspace switch and
  `tiles.json` being rewritten. Needs a workspace switch, which needs a human at
  the keyboard; not measurable from inside a session.
- **Behaviour with two outputs.** Every measurement here is `eDP-1`. `tiles.json`
  is keyed `output:workspace`, so a second monitor means more than one desktop
  can be focused at once, and nothing above says which one wins.
- **Whether anyone wants it.** The tile has been shipping the whole-fleet view
  since v0.1 and no one has reported the absence of focus as a problem. This
  page was written because the question was asked, which is not the same thing
  as the feature being needed.
