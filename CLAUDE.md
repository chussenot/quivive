---
title: Project instructions for agents
status: active
date: 2026-08-28
description: Entry point for coding agents working on this repository — the rules that bind, and the role files that carry the detail.
---

# Project instructions for agents

**Read [docs/conventions.md](docs/conventions.md) first.** It is the binding
rulebook: the task runner, the commit and versioning discipline, the
documentation rules, and which gate enforces each one. A violation is a review
failure, not a style note.

This file deliberately does not restate it. Two copies of a rule become two
different rules.

## Before you commit

```
mise run check
```

That is the whole required gate — `fmt-check`, `lint`, `lint-scripts`, `test`,
`check-docs` — in the same order CI runs them.

Two more that are not in `check`, because both are slow and one is probabilistic:

```
mise run bench    # the per-tick ceilings, release profile
mise run fleet     # the resume-cursor invariant, against concurrent writers
```

Run `fleet` whenever you touch a reader, the fold or the cursor. Its header
explains the experiment; two earlier drafts of it passed a deliberately broken
cursor, so the tick numbers in it are the experiment and not decoration.

## The one invariant

**Deleting `.pact/vigil-cursor.json` and re-running must produce a byte-identical
tile.** Everything in `src/` is in service of that sentence
([ADR-0001](docs/adr/0001-stream-first-tile.md)). `--no-cursor` is the fastest
diagnostic in the repository: if the tile changes when you pass it, the cursor is
wrong and you already know which half to read.

## The rules that get broken most

- **Conventional Commits, with a scope.** `cog check` gates every PR. `cog bump`
  is the only thing that may write a version or a changelog heading.
- **`git commit -- <explicit paths>`**, never a bare `git commit`. A bare commit
  takes the whole index and has swept another agent's staged work into an
  unrelated commit before.
- **Backticks inside a double-quoted shell string are command substitution.** Pass
  a commit message with `-F <file>`, never inline.
- **Never mention any AI, model, or assistant name** in a commit message, tag, PR
  title or PR body.
- **Do not commit or push unless asked.** Report what changed and let the caller
  decide.

## Roles

When a role recurs, it is written down in `.claude/agents/` and an orchestrator
brief **references** the file rather than inlining its prose:

- [ledger-reader](.claude/agents/ledger-reader.md) — readers, the fold, the resume
  cursor, and the invariant that the cursor is correct to throw away.
- [tile-contract](.claude/agents/tile-contract.md) — the emitted shape, the text
  form, exit codes, and the goldens.
- [docs-writer](.claude/agents/docs-writer.md) — README, `docs/`, ADRs, studies,
  and the docs gate.
- [cli-surface-auditor](.claude/agents/cli-surface-auditor.md) — whether `--help`
  and the completions still describe the binary that exists.

Edit a role file when a run teaches the role something, under the same commit
discipline as code.

## Where the design lives

[docs/adr/](docs/adr/README.md) holds the decisions, with alternatives priced.
Before proposing anything vigil currently refuses, check the
[deferral register](docs/adr/0003-yagni-deferral-register.md): the row probably
exists, and it names the condition that would reverse it.
