---
title: House conventions
status: active
date: 2026-08-28
description: The rules every change to this repository follows, and the gate that enforces each one.
---

# House conventions

These are binding. A violation is a review failure, not a style note — and the
reason that is a fair thing to say is that almost every rule below has a gate
behind it, so a violation is usually a red build rather than somebody's opinion.

Where a rule has no gate, it says so, and that is an admission rather than an
exemption.

## Language and tasks

Rust. The compiler is **pinned** in `rust-toolchain.toml`; the edition is chosen
in `Cargo.toml` and must be one that channel supports. Moving the pin is its own
commit (`build(toolchain): ...`), never a side effect — a floating toolchain makes
every `-D warnings` gate a moving target that goes red on a commit nobody touched.

**The task runner is [mise](https://mise.jdx.dev). There is no Makefile.** CI
invokes the same `mise run` tasks a developer does, so local and CI are the same
commands, and there is no second definition of "the tests" to drift.

| Task | Gate |
|---|---|
| `mise run build` | builds |
| `mise run test` | unit, reader fixtures, tile goldens |
| `mise run tile-goldens` | the [tile contract](tile-contract.md) alone, for iterating |
| `mise run lint` | clippy `--all-targets -- -D warnings` |
| `mise run lint-scripts` | shellcheck at warning severity |
| `mise run bench` | the [per-tick ceilings](spec.md#the-tick). Not in `check`: minutes long |
| `mise run fleet` | the resume-cursor invariant against a live fleet. Not in `check`: probabilistic |
| `mise run check-docs` | everything on this page marked *gated* below |
| `mise run check` | all of the above that belong in a required gate, **serially, in CI's order** |

`check` is sequential and that is not a preference: `depends` runs tasks in
parallel, and these legs all reach one binary through one path, so a parallel leg
rebuilding it mid-spawn is a race with a confusing failure.

`mise run test` passes `--no-fail-fast`, and that is not a preference: cargo stops
after the first test *binary* that fails, so one broken unit test hides everything
the four integration suites would have said. That cost a wrong conclusion during
this crate's own construction — see
[docs/studies/conventions-run.md](studies/conventions-run.md).

The conventions and the documentation in this repository landed **before** the
crate, deliberately. While that was true, every cargo task was routed through a
`scripts/with-crate.sh` guard that said loudly it had skipped and exited 0, built
to remove itself once `Cargo.toml` existed. It has: the tasks invoke cargo
directly, and the guard is gone. A permanent indirection that always execs is the
ceremony this convention exists to avoid.

## Versioning and commits

[cocogitto](https://github.com/cocogitto/cocogitto) owns the version and the
changelog, from day one, via `cog.toml`. Nothing else may write either: no
hand-edited `version =`, no hand-added release heading.

- **Conventional Commits, always, with a scope.** `cog check` runs on every PR
  (*gated*). It is PR-only on purpose: running it on push would fail branches for
  history that is already immutable, which teaches people to ignore it.
- **Version bumps happen only via `cog bump`**, which moves the manifest, the
  lockfile and `CHANGELOG.md` together. A tag whose manifest disagrees with it
  ships a binary whose `--version` lies.
- A commit type cocogitto does not recognise is **silently dropped** from the
  changelog — no error, no warning, just a release note that never mentions the
  feature. That is the specific failure `cog check` exists to catch, and it is
  cheap on a PR and expensive after a tag.

### The two-layer changelog

`CHANGELOG.md` has two layers and both are required:

1. **The generated record** — what cocogitto writes from commit subjects. Never
   hand-edited.
2. **Hand-written Notes** — a short paragraph per release saying *why this release
   exists*: what was learned, what changed shape, what a reader upgrading needs to
   know. Written by a person, above the generated list.

A changelog with only layer 1 is a diff with extra steps. Layer 2 is the only part
anybody reads twice. Not gated — this one is a review responsibility.

### "We are not building that"

Refusals go in the [YAGNI deferral register](adr/0003-yagni-deferral-register.md),
one row each, and **a row is only valid if it names the condition that would
reverse it.** A refusal with no reversal condition is a prejudice; six months on,
nobody can tell whether a thing was rejected or merely not reached.

## Documentation

Written as an expert technical writer would: plainly, concretely, and never at
more length than the idea needs.

**Every markdown file carries YAML front matter** with `title`, `status`
(`draft` | `active` | `superseded`) and `date` (*gated*). The front matter is what
makes the docs greppable and lintable — "which pages are still draft" has to be
one command, or nobody asks it. `.claude/agents/*.md` additionally carry `name`
and `description`, which the harness reads, and `name` must agree with `title`
(*gated*), so a renamed role cannot leave a stale name behind in the same file.

**`status: draft` is a real answer.** Several pages in this repository specify
behaviour that does not exist yet, and saying so in the front matter is what keeps
them from being lies.

**Decisions live in `docs/adr/NNNN-<slug>.md`** with four sections — Context,
Decision, **Alternatives considered** (*priced* — what each one costs, not merely
that it was thought of), Consequences — and front matter naming
`decision-makers` and `supersedes` (*gated*, all of it, including the section
headings). A record is never rewritten to say something else: it is superseded by
a new record, and the old one changes only its status and gains a link forward
(*gated*: a superseded record with no successor link fails).

Every ADR appears in [docs/adr/README.md](adr/README.md) and every index row
points at a real file — both directions (*gated*), because a one-directional check
catches only the half of the drift you thought of.

**Field evidence lives in `docs/studies/`.** Proving notes from a real run: what
was measured, on what, and what it changed. A study is allowed to report that
nothing was learned; it is not allowed to report a number nobody ran.

**Graphs are Mermaid in markdown.** No image files, no embeds (*gated*). A diagram
you cannot diff is a diagram that quietly stops matching the system it draws.

**The README answers *why* and stays DRY.** What gap vigil fills in the family,
the boundary with agentic-db, the shape of the refusals — and a link out for
everything else. If a section explains *how* at length, it belongs in `docs/`.
Not gated, and the honest reason is that no cheap check distinguishes a necessary
paragraph from a drifting one; but *orphan pages are* (*gated*): a page under
`docs/` that nothing links to fails, so the index cannot silently stop listing
things.

**Never write a claim you have not checked**, and never hand-edit sample output.
Run the command and paste what it printed. This is the rule that decays first and
costs most.

## Agent guidance

**When a role recurs, write it down.** Roles live in `.claude/agents/<role>.md`,
front-mattered like every other doc. Orchestrator briefs **reference** these files
rather than inlining role prose — a brief that inlines a role is a brief that will
disagree with the next brief.

They are edited when a run teaches the role something, under the same commit
discipline as code. A role file is documentation of how this repository is worked,
not a prompt fragment.

Current roles: [ledger-reader](../.claude/agents/ledger-reader.md),
[tile-contract](../.claude/agents/tile-contract.md),
[docs-writer](../.claude/agents/docs-writer.md),
[cli-surface-auditor](../.claude/agents/cli-surface-auditor.md).

## What is not gated

Said plainly, so nobody mistakes silence for approval:

- the two-layer changelog's Notes layer
- the README's *why*-only discipline and its length
- "never write a claim you have not checked"
- whether an ADR's priced alternatives are honestly priced
- **whether a test would fail if the code were wrong.** Four times during this
  repository's construction, a check passed for a reason other than the one
  claimed. The only defence found so far is a **negative control**: break the
  thing on purpose, in a throwaway copy, and confirm the check goes red. Nothing
  enforces that; `docs/studies/conventions-run.md` records what it caught.
- **whether a Mermaid diagram parses.** It is checked, but by hand: mermaid's own
  parser needs node and a DOM, and adding a JavaScript toolchain to a Rust
  repository's required gate costs more than the defect it prevents. The
  procedure and the run are in
  [docs/studies/conventions-run.md](studies/conventions-run.md); a diagram that
  does not parse renders as an error box, so re-run it when you add one.

Each of those is a review responsibility. Every other rule on this page fails a
build when broken, and if one of them grows a cheap mechanical check, it should
move.
