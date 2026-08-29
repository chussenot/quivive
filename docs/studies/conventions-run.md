---
title: Proving the conventions before the code
status: active
date: 2026-08-28
description: Field notes from the two runs that built this repository and the fleet run that then rebuilt it for v0.1 — what each gate was measured against, the six times a check passed for the wrong reason, and what is still unproven.
---

# Proving the conventions before the code

This repository's conventions, decision records and specification landed before
its Rust crate did. That ordering was deliberate, and this page is the evidence
from both runs: what was actually measured, what the measurements found, and what
is still only asserted.

**The recurring finding is not about vigil.** Six times across the two runs, a
check passed for a reason other than the one claimed — and in five of those the
green result was reported before anybody noticed. They are each recorded below
where they happened, and collected at the end, because the pattern is more useful
than any of the instances.

The genre is pact's `docs/studies/`: field notes, not a tutorial. A study is
allowed to report that nothing was learned. It is not allowed to report a number
nobody ran.

## Why the conventions went first

Two of this repository's rules are about *drift* — front matter on every page,
priced alternatives in every ADR — and drift is cheap to prevent and expensive to
reverse. A convention introduced on day 40 has to be applied retroactively to 40
days of files by somebody who did not write them, and the usual outcome is that
the rule gets narrowed until it fits what already exists.

While that was true, the cost was worth naming plainly: **every cargo gate in
`mise.toml` was unexercised.** `fmt-check`, `lint`, `test`, `tile-goldens`, `bench`
and `fleet` were written exactly as they would run, and every one of them printed
`with-crate: SKIPPED` and exited 0. They were prose.

They are not any more — experiments 4 to 7 are those gates running — and the
`with-crate.sh` guard that made them green on an empty repository has removed
itself, as it was built to.

The ordering paid for itself in one specific way worth recording: **the tile
contract was written before the code and implemented unchanged.** Not because the
first guess was lucky, but because writing down what the output had to be left the
goldens something to be golden against, so the shape was argued about once, in
prose, instead of drifting field by field.

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

## Experiment 4 — the invariant against real pact ledgers

**Question.** Does the resume cursor actually satisfy
[ADR-0001](../adr/0001-stream-first-tile.md) — delete it, re-read, get a
byte-identical tile — on a ledger nobody wrote for the purpose?

**Method.** pact's own committed `.pact/events.jsonl` (729 lines, 246 KiB) and
recount's (308 lines, 135 KiB). With `VIGIL_NOW` frozen, three reads: a cold one
(no cursor on disk), a warm one (resuming the cursor the cold read left), and a
forced cold one (`--no-cursor`).

**Result.** All three byte-identical. The cursor consumed 251,425 of 251,425
bytes. **Zero declined lines across 1,037 real rows**, which is the number that
says the reader understands the format rather than tolerating it.

The corpus is also what taught the reader two things a fixture would not have:
pact's rows carry fields vigil has never heard of (`chain_hash`, `invoked_from`,
`scope`, `context_key`) and a `kind` — `context` — that is not in any list vigil
was written against. Both are read correctly, and both are read correctly *because
the reader was written to ignore what it does not recognise* rather than to
enumerate what it does.

## Experiment 5 — the per-tick ceilings

**Question.** Is a streamed fold actually cheap enough to make the whole design
work? Its failure is the documented reversal condition for
[D2](../adr/0003-yagni-deferral-register.md) — a daemon — so this is the number
the architecture rests on.

**Method.** `mise run bench`: a 100,000-event synthetic ledger (14.7 MiB, 64
agents), release profile, four timings.

| Tick | Time |
|---|---|
| cold (full re-read of 100,000 events) | 50.6 ms |
| warm, nothing appended | 157 µs |
| warm, 8 events appended | 115 µs |
| forced cold, for the ratio | 49.2 ms |

**Speedup, cold to warm: 426x.** At 1 Hz a warm tick is roughly one ten-thousandth
of the interval, which is the margin that makes "call it every second forever"
a reasonable thing to ask of a status bar. The daemon in D2 would buy latency
nobody can perceive, and the row stays where it is.

The bench refuses to assert under `debug_assertions`, which is not ceremony: the
same fold in debug is slow enough to fail these ceilings by more than an order of
magnitude, and a gate that fails for the profile is a gate somebody deletes.

## Experiment 6 — the fleet soak, and two negative controls it failed

**Question.** The invariant holds over fixtures the test author wrote and over
static real ledgers. Does it hold over a cursor advanced, tick by tick, through
writes it did not control?

**Method.** `scripts/fleet-sim.sh`: eight concurrent writers appending in pact's
shape, 30 ticks, two injected rewrites (pact compacts `events.jsonl` to its newest
4000 lines once it passes 5000, so a rewrite is routine here, not exotic). Then
writers stop, the clock freezes, and the warm tile is compared against a cold one.

**Result on the real binary.** Passes: no tick reported a decline, and warm, cold
and re-cold agree.

**And that result meant nothing, twice.** The negative control — a build whose
cursor trusted its byte offset without verifying the tail — sailed through two
drafts of this script.

*Draft one* ran all eight writers for the whole run. The fold keeps only the
**newest** evidence per agent, so a resumed read that skips events in the middle
of the file arrives at the same maximum as a cold read. **Skipping is invisible to
a max.** Every writer being active meant every agent's newest event survived every
rewrite, and there was nothing for the comparison to find.

*Draft two* silenced half the writers, so a rewrite would drop their lines
entirely and a stale accumulator would show up as extra agents. It still passed —
because the silencing happened *before* the first rewrite, and that rewrite shrank
the file below the cursor's offset, which even a trusting cursor notices. The
accumulator was reset before it had anything stale in it.

The scenario that discriminates needs three things in this order:

1. a rewrite the cursor **does** notice, which resets the accumulator;
2. agents that then go quiet, so the accumulator learns them;
3. a rewrite that drops those agents' lines and then **grows the file back past
   the cursor's old offset**, so a length comparison cannot notice.

Only then does a cold read fail to know the quiet agents exist while a trusting
cursor still carries them. With that ordering the negative control fails, loudly,
naming the four agents the warm tile invented. The script's header carries the
reasoning so the tick numbers in it are not tidied away as arbitrary.

**What this says about the invariant.** It holds. It also says the fold's
insensitivity to skipped middle events is a real property of the design and not
an accident — which is reassuring for correctness and is exactly what made it hard
to test.

## Experiment 7 — what the suites found in the code

Two findings from writing tests rather than from running them, both recorded
because neither would have survived to a user without them.

**A panic reachable from a file on disk.** `a_lock_with_an_absurd_ttl_does_not_panic`
failed on first run. `chrono::TimeDelta::seconds` **panics** out of range rather
than saturating, which the code had assumed the other way round; `ttl_secs` is a
`u64` read from a lock file, so a garbage value crashed the tick. A panic is the
one failure mode a status bar cannot survive. The same audit found a second
instance: subtracting two `DateTime`s panics when the span exceeds i64
milliseconds, and two timestamps chrono will happily parse can be 500,000 years
apart. Both are now epoch-second arithmetic, which cannot overflow.

**Drift the auditor role was written to catch, committed in the same session.**
The four window defaults existed as literals in *two* places — `Duration::from_secs(60)`
in `src/state.rs` and `default_value = "60s"` in `src/cli.rs` — with nothing
connecting them. Writing `.claude/agents/cli-surface-auditor.md`, whose whole
subject is defaults quoted in a second place, is what exposed it. They are now
four string consts that clap renders and `Thresholds::default()` parses, so there
is one source; and the numbers were deleted from `docs/spec.md`'s Mermaid diagram
labels, which was a third copy.

## Experiment 8 — the two legs only CI could run

Two caveats were written down here as unverified, with the note that "the first CI
run is where they get cashed". It ran, and they are.

**`mise run lint-scripts`.** shellcheck could not be obtained in the environment
these runs happened in — its binary download returns 403 — so `check-docs.sh` and
`fleet-sim.sh` were reviewed by hand against the warning-severity rules and
otherwise unverified. CI ran `shellcheck --severity=warning scripts/*.sh` and both
are clean. Hand review is not a substitute for the tool; it was green this time,
which is not the same as being a reliable method.

**`mise run check` as a whole.** mise itself was not installed locally, so each leg
had been invoked directly and the composition never was. CI ran it end to end:
`fmt-check`, `lint`, `lint-scripts`, `test`, `check-docs`, in that order, in 15
seconds.

The run also confirms two things worth checking rather than assuming, given
everything else on this page. `rust-toolchain.toml`'s pin was honoured — the
runner has both 1.94.1 and 1.98.0 available and the build used **1.94.1** — and
`cargo test --no-fail-fast` reported all six test binaries: 58 passed, with the
bench correctly `ignored` rather than silently skipped. A green step is not by
itself evidence that the step did the work; the log is.

### A written claim, falsified by the first real run

Not a check passing for the wrong reason — the inverse defect, and cheaper to
make. `.github/dependabot.yml`'s cargo entry shipped with a comment asserting
that, with no `Cargo.toml` in the repository, dependabot "finds nothing to update
rather than erroring". Nobody had checked. The first scheduled cargo job aborted
with `dependency_file_not_found: /Cargo.toml not found` and reported an error on
the repository — exactly the failure the comment promised away.

Self-healing in this case, because the crate landed hours later and the next run
succeeds. Recorded anyway, because it is the conventions' own rule — *never write
a claim you have not checked* — broken in the file where breaking it was cheapest
to avoid: the claim was testable by reading dependabot's documented behaviour, or
by waiting one scheduled run before asserting anything.

## The six times a check passed for the wrong reason

Collected, because the pattern is worth more than the instances:

| # | Where | The green result was really |
|---|---|---|
| 1 | Experiment 1, first harness | six trials with the seeded defect undetected — the tree had one unrelated pre-existing failure and the harness only checked the exit code |
| 2 | Experiment 1, corrected harness | two trials failing on backticks inside a double-quoted shell string, evaluated as commands — the gate had caught both defects |
| 3 | Experiment 3, first negative control | a config parse error, on a clone taken before the config fix — not the bad commit subject |
| 4 | The first cursor comparison | wall-clock drift between two runs of a **stale binary** — `cargo clippy` checks without rebuilding, so the new code was never in it |
| 5 | Experiment 6, soak draft one | a comparison that could not have differed: the fold is a max, and skipping is invisible to a max |
| 6 | Experiment 6, soak draft two | the same, one step subtler: the accumulator was reset before it had anything stale in it |

A seventh, which cost a wrong conclusion rather than a wrong pass: `cargo test`
stops after the first test **binary** that fails, so a seeded defect looked like it
was caught by a unit test and missed by the goldens, when the goldens had simply
never run. `mise run test` now passes `--no-fail-fast`.

Two defences worked, and nothing else did:

* **Read the message, not the exit code.** Every trial now asserts the specific
  finding it expects. This caught 1, 2 and 3.
* **Break it on purpose.** A negative control — seed the defect in a throwaway
  copy, confirm the check goes red — caught 5 and 6, and is the only thing that
  could have. It is now named in
  [docs/conventions.md](../conventions.md#what-is-not-gated) as an ungated
  responsibility, because nothing can enforce it.

Number 4 was caught by neither, and by luck: the frozen clock in the output did
not match the frozen clock that was passed in. It is the argument for the
`VIGIL_NOW` seam being visible in the tile at all.

## What this run leaves unproven

Said plainly, because a study that only reports its successes is an advertisement:

- **The two-layer changelog** has no release to demonstrate it on.
- **Anything outside Linux.** No platform matrix, no MSRV promise, no published
  binary — and that is [D9](../adr/0003-yagni-deferral-register.md) rather than an
  oversight. The toolchain pin in `rust-toolchain.toml` is the only promise made.
- **Whether the tile is a good tile.** Everything above is about whether vigil
  computes what it says it computes. Whether one line of `3A 3I 1S 1D
  worst=dead …` is the *right* line for somebody watching a fleet is a question no
  test in this repository can answer, and the honest next step is to run it on a
  real bar during a real fleet run and see what gets looked at.

Each of those is a thing to come back and fill in. The first release's Notes
paragraph is the natural place to say which of them turned out to be true.

## The v0.1 fleet build

The single-repo tool the experiments above were run against was rebuilt as
quivive's multi-repo v0.1 (registry, tick, tile, stream, watch, why) by a
fleet of agents coordinating over pact, in the same repository. This section
is what `bd`, `pact log`/`pact audit` and `git log` say about that run — no
number below is estimated.

**Shape of the run.** [`.pact/plan.json`](../../.pact/plan.json) lints five
waves: wave 1 the crate skeleton and rename; wave 2 the readers, the state
model and the registry; wave 3 the tile payload, the pwetty contribution,
`watch` and `why`; wave 4 stream mode and the goldens/bench suite; wave 5 this
documentation bead. Ten planned beads closed across waves 1-4, plus a
phase-0 smoke bead (`quivive-ksy`) that predates the plan and exists only to
prove the shared Dolt store accepted a write. `bd list` shows 15 closed beads,
one open follow-up (`quivive-1y2`, filed to unify three independent
derivations of S18's "started" set — see below), the epic, and this bead. The
branch carries 76 commits.

**Four unplanned bug beads, all filed at the wave-3/4 boundary.** `bd show`
on each names the same free-run window: `quivive-s16` (an integration bead —
`watch` and `tile` had rewritten `main.rs`/`cli.rs` against incompatible APIs
on parallel branches, discovered when the two merged), `quivive-jwp`,
`quivive-trx`, and `quivive-374`. None was in the original plan; the wave
plan closes with ten beads, the actual run closed fourteen implementation
beads before this one.

**The merge oracle caught a test that had passed at authoring time.**
`quivive-jwp`'s own description: a `why` test built against a frozen clock
constant passed when written and failed hours later on the *same, untouched*
commit — "discovered by the wave-3 merge oracle" (`pact merge --verify`,
re-running the suite at merge time rather than trusting a stale green from
whenever the branch last ran it). The cause: a fixture wrote an activity
record whose file mtime was real wall-clock time while its content was
pinned to the frozen test clock, and the reader takes `max(content, mtime)`
— so as real time drifted past the frozen clock, the mtime started winning
and the fixture agent read `ACTIVE` instead of `DEAD`. Fixed by pinning the
fixture's mtime with `set_modified` instead of trusting the write time.

**A prune-freeze bug found by one bead's reasoning about a sibling's code.**
`quivive-trx`'s own description credits it explicitly: the `stream` bead's
module doc worked out that `watch`'s mtime-pruning (S5: skip the re-read on
an unchanged repo) must not also skip *re-assessing the clock* — DEAD and
`drained` are transitions that become true by time passing with no new
write, and a quiet repo whose read was skipped never re-evaluated them. The
reasoning was written for `stream`, which does not prune; whoever picked up
`quivive-trx` recognized the same gap sitting dormant in `watch.rs` and
reproduced it with two fixtures before fixing it (cache the last snapshot,
re-run `state::assess` against it at the new clock on every pass, even a
pruned one).

**S18 (gate-order violations) was wired last, immediately before this
bead.** `watch`'s live loop shipped with `plan: None` always set — a known
gap noted by `watch-r2` and confirmed still dormant by `watch-r3` — so a
gate-order violation could be asserted in a fixture but could never fire from
a real tick. `quivive-374` closed that gap (folding `S18`'s started-bead set
incrementally through the resume cursor, then wiring `plan` and `started`
into `watch`'s snapshot) two commits before the branch tip this bead started
from — the commit trailer literally reads "S18 wired and merged... docs wave
next."

**A retry storm, named by `pact audit --check retry-storm`:** *"tile refused
src/cli.rs 45 time(s), retrying every ~15s against a median 30m22s of holder
lease left — and NEVER got the path."* The ledger bears this out exactly: 45
of the run's 46 `refused` events are `tile` re-acquiring `src/cli.rs` between
11:13:15 and 11:28:52 while `watch` held it, none of them ever landing. Not a
rule violation — a refused agent is entitled to ask again — but exactly the
spent-for-nothing work `AGENTS.md`'s own coordination protocol names: `pact
lease acquire --wait` or `pact watch add` would have spent none of it.

18 distinct pact agent identities acted in this run (`pact audit`'s
attribution table), several of them resumption names for a bead whose first
holder went quiet — `tile-r2`, `watch-r2`, `watch-r3`, `why-r2` — which is the
`--to-owner-of` addressing scheme in `AGENTS.md` working as designed: the
lease note that `tile-r2` inherited on acquiring `docs/tile-contract.md` said
in so many words that it was "resuming dead holder's work."

## Phase 3 — the accounting battery, and what it flagged

Run after the last merge, against the committed ledger (262 events, 34
messages, 8 agent identities on 3 models across 88 commits and 8 verified
mutex merges). Clean checks first: **topology** (every context-stamped event
from a worktree, the orchestrator's 14 main-checkout events excused by
`--allow-main`), **double-win** (no two agents ever held one path at once),
**merge-divergence** (every hold started from the content the previous holder
left), **chain-integrity** (no gap, edit or forgery in the tracked portion),
and handoff coverage 10 of 10 beads-with-dependents.

The exceptions, each explained rather than excused away:

- **One uncovered commit** (`commit-correlation`): the gate-order agent edited
  `src/cursor.rs` — the `started` set had to persist through the resume
  cursor — without leasing it; its lease named four files and the fix needed a
  fifth. The check exists for exactly this and caught exactly this. The edit
  itself was correct and merged under the oracle.
- **Ten expired holds** (`stale-holds`): the two agents killed mid-work by a
  model session limit died holding 3 and 7 paths; the leases lapsed at TTL and
  were reclaimed, and `-r2` successors resumed the worktrees. A corpse cannot
  release.
- **Two silent contentions**: both are the same deaths seen from the waiting
  side — the paths were freed by expiry, so no release message was ever going
  to be sent.
- **One 45-retry "storm"** that was actually one compliant `--wait 10m`: pact's
  wait loop writes a refusal event per internal ~15s poll, so its own audit
  cannot tell an obedient waiter from the storm the check hunts. Filed as
  friction to upstream (quivive-cql), with this ledger as the evidence.
- **gate-order: could not run** — this plan declared waves but no gate beads,
  so the check had nothing to order against. The charter says gates are beads;
  this run enforced its barriers by orchestrator sequencing instead, and the
  audit's silence here is the honest price of that choice.
- **claim-lease-divergence: could not run** — bd's audit sidecar records only
  from enablement, and it was enabled at verification time, not at spawn time.
  On since `bd config set audit.enabled true`; the next fleet gets the check
  this one could not have.

The second-laptop simulation and its one-row drift are in
[docs/beads-on-other-laptops.md](../beads-on-other-laptops.md); the S6 bench
(23 µs per repo, ceiling 10 ms) is in the fleet-run section above.

## After the tag — the downstream review, and what 0.1.0's own bump broke

quivive's maintainer took the pwetty contribution onward in waybar-pwetty-box
(schema reconciliation, a minijinja checker fix, mise tasks with an
inode-verifying waybar restart) and those commits were reviewed from here with
the house discipline: verify, do not trust. Everything held — including the one
claim the new schema makes about *this* repository's behavior ("a stale holder
never produces a `dead_holding_paths` item"), confirmed against `state.rs`'s
holder filter, where an unknown holder deliberately reads as dead. Gates on the
reviewed tree: 59 tests and clippy clean, all five samples render, and the
cross-repo byte-identity test still passes 17/17 from this side.

Three findings worth the ink:

- **S12 amended.** The reconciliation dropped the schema's REAL vs MOCK
  provenance labels — correctly: they existed for the era when samples were
  invented by hand, and S13's byte-copy rule made every field REAL. The spec
  line that required them was amended rather than left to disagree with the
  artifact it governs; the original wording is preserved in the amendment.
- **One cold-run flake.** The first `cargo test` after fetching failed a single
  test; six consecutive runs after it were green. Most likely the
  stream-respawn timing tests under first-build load. Recorded because a flake
  seen once and never chased is how a suite's word gets cheap.
- **A robustness nit, not a bug**: the new `template_locals` helper matches
  `{% set ` literally, so a whitespace-control variant (`{%- set`) would slip
  past it and resurrect the false warning it exists to kill. No current
  template uses that form.

And one the tag itself delivered: **`cog bump` spliced the 0.1.0 record into
the changelog's own preamble comment.** The preamble described the insertion
point by quoting the triple-dash separator verbatim, cog anchors on the first
literal occurrence in the file, and the first occurrence was the quotation —
so the generated record landed inside an HTML comment, rendered invisible,
while the stale "No release yet, and no tag" prose below it stayed visible on
a tagged repository. A comment about the mechanism became the mechanism's
input: the same self-reference failure this study already records twice
(Experiment 1's harness reading its own backticks, the seeded image-embed the
gate flagged in its own documentation). The repair kept the generated layer
byte-identical — relocated, never edited — added the Notes layer the
convention requires, and rewrote the preamble to describe the separator
without containing one.
