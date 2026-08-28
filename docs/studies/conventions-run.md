---
title: Proving the conventions before the code
status: active
date: 2026-08-28
description: Field notes from the run that established this repository's conventions — what the docs gate was measured against, what the measurement found in itself, and what stayed unproven.
---

# Proving the conventions before the code

This repository's conventions, decision records and specification landed before
its Rust crate did. That ordering was deliberate, and this page is the evidence
from that run: what was actually measured, what the measurement found, and what is
still only asserted.

The genre is pact's `docs/studies/`: field notes, not a tutorial. A study is
allowed to report that nothing was learned. It is not allowed to report a number
nobody ran.

## Why the conventions went first

Two of this repository's rules are about *drift* — front matter on every page,
priced alternatives in every ADR — and drift is cheap to prevent and expensive to
reverse. A convention introduced on day 40 has to be applied retroactively to 40
days of files by somebody who did not write them, and the usual outcome is that
the rule gets narrowed until it fits what already exists.

The cost of the ordering is honest and worth naming: **every cargo gate in
`mise.toml` is unexercised.** `fmt-check`, `lint`, `test`, `tile-goldens`, `bench`
and `fleet` are written exactly as they will run, and today every one of them
prints `with-crate: SKIPPED` and exits 0. They are prose until the crate lands.
Nothing in this run tested them, and this page does not claim otherwise.

## Experiment 1 — seeded breakage against the docs gate

**Question.** Does `scripts/check-docs.sh` catch the defects it claims to, or does
it only catch the ones that were convenient to write?

**Method.** Nineteen trials. Each copies the repository to a throwaway directory,
breaks exactly one convention, runs the gate, and asserts that **the specific
expected finding** appears in the output. One defect per trial, so a finding cannot
be credited to the wrong check.

**Result: 19 caught, 0 missed.**

| Seeded defect | Caught by |
|---|---|
| front matter block deleted | `has no YAML front matter (needs title, status, date)` |
| `status:` key removed | `front matter is missing status` |
| `status: finished` | `allowed: draft, active, superseded` |
| `date: 28 Aug 2026` | `expected YYYY-MM-DD` |
| agent file's `name` and `title` disagree | `name docs-writer and title doc-writer disagree` |
| agent file's `description` removed | `is missing description (the harness reads it)` |
| ADR renamed to `no-daemon.md` | `is not named NNNN-<lower-kebab-slug>.md` |
| `## Alternatives considered` retitled | `has no ## Alternatives considered section` |
| `decision-makers:` removed from an ADR | `is missing decision-makers` |
| `supersedes: [0009]`, no such ADR | `says it supersedes 0009, and no docs/adr/0009-*.md exists` |
| ADR marked superseded, successor link removed | `is superseded but links to no successor ADR` |
| new ADR added, index not updated | `docs/adr/README.md does not list 0004-a-new-decision.md` |
| index row pointed at a nonexistent file | `lists 0003-yagni-deferrals.md, which does not exist` |
| `docs/spec.md` link retargeted | `link target does not exist: docs/specification.md` |
| `#the-state-machine` anchor retargeted | `no heading matches anchor #the-state-diagram` |
| `docs/architecture.png` added | `docs/ contains an image file` |
| a markdown image embed added to a page | `embeds an image; diagrams are Mermaid in markdown` |
| unterminated ```` ``` ```` fence | `has an unclosed fenced block (5 fence lines)` |
| unlinked page added under `docs/` | `is linked from no other page — add it to an index` |

### The two defects the experiment found in itself

Both were in the *measurement*, and both are the reason this section exists rather
than a bare "19/19".

**1. The first nineteen trials were partly meaningless, and reported as passes.**
The first version of the harness asserted only that the gate exited non-zero. At
that point the tree had one genuine pre-existing failure — a link to this page,
which did not exist yet — so **six trials "passed" without their seeded defect
being detected at all**. The gate was red for a reason unrelated to the trial and
the harness could not tell the difference.

This is the shape of a whole class of bad test: a pass that is caused by something
other than the thing under test. The fix was to assert the specific finding, which
is why the table above quotes a message per row rather than a verdict per row.

**2. Two of the corrected trials then failed for a reason in the harness.** The
expected-finding strings contain backticks, and backticks inside a double-quoted
shell string are command substitution — so two expectations were silently
evaluated as commands and compared against empty output. The gate had caught both
defects; the harness could not see it.

That hazard is already written down in three of the `.claude/agents/` role files
("pass a commit message with `-F <file>`, never inline"), which is a fair summary
of how much good writing a rule down does on its own.

**A third finding, from writing this page.** The row above deliberately does not
quote the seeded image-embed syntax, because an earlier draft did — and the gate
flagged this study for embedding an image. It is line-based and cannot distinguish
a quoted example from a violation. That is being left as it is: a dumb, predictable
checker whose cost is that documentation must not quote a violation literally is a
better trade than a checker with a parser in it. Worth knowing before you write the
page that explains a rule.

**Not established by this experiment:** that the gate has no *false positives*
beyond the baseline pass on the real tree, and that its checks are the right
nineteen. A seeded-breakage run can only show that the defects you thought of are
caught.

## Experiment 2 — do the diagrams parse

**Question.** "Graphs are Mermaid in markdown" is enforced by
`scripts/check-docs.sh` in the sense that no image file may exist. Nothing checks
that the Mermaid actually *parses* — and a diagram that does not parse renders as
an error box, which is strictly worse than a missing diagram.

**Method.** Every ```` ```mermaid ```` block in the repository, parsed with
mermaid 11's own `mermaid.parse()` under jsdom.

**Result: 3 blocks, 0 failed** — the family diagram in `README.md`, and the state
machine and tick data-flow in `docs/spec.md`.

The first run failed all three with `DOMPurify.addHook is not a function`, which is
mermaid needing a DOM rather than anything about the diagrams; jsdom globals fixed
it. Worth recording because a validator that fails identically on good and bad
input is a validator that will be believed once and then ignored.

**This check is deliberately not in `mise run check`.** It needs node and a DOM,
and adding a JavaScript toolchain to a Rust repository's required gate costs more
than the defect it prevents. It is listed among the ungated rules in
[docs/conventions.md](../conventions.md#what-is-not-gated), and re-running it is a
manual step when a diagram is added.

## Experiment 3 — does the commit gate actually gate

**Question.** `cog check` is configured and wired into CI on every PR. Does it
pass on this repository's own history, and does it fail on the defect it exists to
catch — a commit type cocogitto silently drops from the generated changelog?

**Method.** cocogitto 7.0.0. `cog check` over all history, then the
`BASE..HEAD` range form CI actually invokes, then a negative control: a throwaway
clone with one commit subject cocogitto cannot parse.

**Result.** Both positive runs report `No errored commits`. The negative control
reports `Commit type 'chora' not allowed` and exits 1.

**The finding was in the configuration, and it was a hard failure.** The first
`cog check` never looked at a commit at all:

```
Error: failed to parse config
	cause unknown field `packages`, expected one of ... `monorepo`, `scopes`
```

`packages` was renamed to `monorepo` in cocogitto 7, and an unknown field fails
config parsing outright. The table was **empty** — it set nothing whatsoever. It
was there because it is in the config shape the sibling repositories use, which
predates that release. Three more empty tables went with it; a gate that exits 1
before reading a commit is not a stricter gate, it is an absent one, and every PR
would have been red for a reason unrelated to its commits.

**And the contamination from Experiment 1 recurred here, immediately.** The first
negative control "passed" — exit 1, as hoped — while actually failing on the
config error, because the throwaway clone was taken before the config fix was
committed. Twice in one run, the same mistake: **an exit code is not evidence
about the thing you are testing.** Reading the message rather than the status is
what caught it both times.

## What this run leaves unproven

Said plainly, because a study that only reports its successes is an advertisement:

- **Every cargo gate.** Unexercised, as above.
- **The resume-cursor invariant** — that deleting the cursor and re-reading
  produces a byte-identical tile — is the load-bearing claim of
  [ADR-0001](../adr/0001-stream-first-tile.md) and is at present a sentence.
  `mise run fleet` is written to measure it and has never run.
- **The per-tick ceilings** in [docs/spec.md](../spec.md#the-tick) are targets, not
  measurements. No number in this repository came from a profiler.
- **`mise run lint-scripts`.** shellcheck could not be obtained in the
  environment this run happened in — the binary download returns 403 — so the two
  scripts were reviewed by hand against the warning-severity rules and are
  otherwise unverified. It is a required leg of `mise run check`, so the first CI
  run is its first real execution. If it is red, that is this note being cashed.
- **`mise run check` as a whole.** mise itself was not installed here. Each leg
  was invoked directly; the composition was not.
- **The two-layer changelog** has no release to demonstrate it on.

Each of those is a thing to come back and fill in. The first release's Notes
paragraph is the natural place to say which of them turned out to be true.
