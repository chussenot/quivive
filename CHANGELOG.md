---
title: Changelog
status: active
date: 2026-08-28
description: Release record — cocogitto's generated list, and the hand-written Notes saying why each release exists.
---

# Changelog

All notable changes to this project are documented here. Commit guidelines:
[conventional commits](https://www.conventionalcommits.org/).

This file has **two layers**, and both are required by
[the house conventions](docs/conventions.md#the-two-layer-changelog):

1. **The generated record** — written by `cog bump` from commit subjects. Never
   hand-edited. A commit type cocogitto does not recognise is silently dropped
   from it, which is what `cog check` on every PR exists to prevent.
2. **Hand-written Notes** — a short paragraph per release, above the generated
   list, saying *why this release exists*: what was learned, what changed shape,
   what somebody upgrading needs to know. A changelog with only layer 1 is a diff
   with extra steps.

<!-- cog inserts each release below the triple-dash separator line. Do not move
     that separator, and do NOT write a literal one anywhere above it, this
     comment included: cog anchors on the FIRST occurrence in the file, and the
     0.1.0 bump spliced its whole record into the middle of this very comment
     because an earlier wording quoted the separator verbatim while describing
     it. The record rendered invisible; the repair is in the 0.1.0 Notes. -->

- - -
## 0.1.0 - 2026-08-28

### Notes

quivive exists because a fleet of coding agents on one machine is easy to run
and hard to glance at: pact records what happened, recount explains why,
agentic-db watches the session — and nothing said *when to look*. 0.1.0 is that
sentry, built to a spec written before the code by a Sonnet fleet coordinated
through pact under a planning orchestrator, and the contract page the fleet
implemented shipped unchanged. The surface is deliberately three commands: a
one-shot `tile` (and its `--stream` push mode for the pwetty tile this release
contributed to waybar-pwetty-box), `watch` with exactly four notify-send
transitions that each carry their follow-up command, and `why` with evidence
lines. Everything else is a row in the deferral register with its reversal
trigger.

Three things the build itself taught, all shipped fixed and recorded in
[docs/studies/conventions-run.md](docs/studies/conventions-run.md): the merge
oracle rejected a test that was green when written and red two hours later
(real file mtimes racing a frozen clock); the mtime prune was silently freezing
exactly the time-driven transitions `watch` exists for; and gate-order
violations were structurally unreachable until the started-set was folded
through the resume cursor. The numbers that matter: a steady-state tick costs
~23 µs per repo against the spec's 10 ms ceiling, and the five tile samples are
byte-identical between this repository's goldens and the pwetty contribution.

Upgrading: nothing to upgrade from — this is the first tag. No binary is
published and no cross-platform promise is made (deferral D9); `mise run
install` from a checkout is the supported path.

## 0.1.0 - 2026-08-28
#### Bug Fixes
- **(cli)** rewire main.rs and cli.rs to the reshaped tile Payload API - (2b72b49) - chussenot
- **(cog)** drop the empty tables that fail config parsing on cocogitto 7 - (a6a66b4) - chussenot
- **(cursor)** rename the resume cursor file to match the crate - (837baa7) - chussenot
- **(deps)** correct the dependabot comment the first cargo run falsified - (47eadd7) - chussenot
- **(scripts)** repair fleet-sim's stale vigil references - (f2b010e) - chussenot
- **(watch)** wire plan and started ids into to_snapshot so S18 can fire - (cfb1712) - chussenot
- **(watch)** re-assess the cached snapshot on an unchanged pass - (b8be057) - chussenot
- **(why)** pin activity fixture mtime to the frozen test clock - (975db55) - chussenot
#### Build system
- **(crate)** rename package from vigil to quivive - (b663341) - chussenot
- **(deps)** bump Swatinem/rust-cache - (50469c1) - dependabot[bot]
- **(tooling)** pin the toolchain and add mise, cocogitto and CI - (4f7af83) - chussenot
#### Documentation
- **(adr)** reconcile ADR-0001/0002/0003 to quivive v0.1 - (90d345a) - chussenot
- **(agents)** reconcile role files to quivive v0.1 - (dcaf528) - chussenot
- **(claude)** quivive-cursor.json and the deferral-register reference - (708e719) - chussenot
- **(conventions)** rename the stray vigil reference to quivive - (81d2344) - chussenot
- **(conventions)** establish the house rules, the first three decisions and the spec - (86382d9) - chussenot
- **(finale)** the tile that watched its own builders, and the closing account - (883cd5d) - chussenot
- **(readme)** the why-pass — sentry framing, agentic-db boundary, deferrals - (7299e75) - chussenot
- **(spec)** restore the state-machine and tick data-flow diagrams - (3aaf185) - chussenot
- **(spec)** respec for quivive v0.1 — registry, tick, stream-first tile, watch, why - (fa52cd2) - chussenot
- **(studies)** field notes from the v0.1 fleet build - (195484b) - chussenot
- **(studies)** cash the two caveats only CI could settle - (1334e0c) - chussenot
- **(studies)** record the commit-gate run and what the environment could not prove - (99cae45) - chussenot
- **(tile-contract)** sample-sync rule, and the D3 boundary rewrite - (a5d7f46) - chussenot
- bring the specification, contract and conventions in step with the code - (9732584) - chussenot
#### Features
- **(cli)** fix the surface to tile|watch|why and wire all modules - (de89224) - chussenot
- **(reader)** fold S18's started-bead ids from the ledger incrementally - (30b2e62) - chussenot
- **(reader)** wire activity/plan/sidecar into one Readings, add the prune gate - (e41fc5d) - chussenot
- **(reader)** read .beads/interactions.jsonl when bd's audit export exists - (0e0f60d) - chussenot
- **(reader)** read .pact/plan.json, pact's linted wave-plan snapshot - (38feb12) - chussenot
- **(reader)** read .pact/activity/* recency markers - (c6ff7fd) - chussenot
- **(registry)** implement plain-text repo registry reader - (e0156db) - chussenot
- **(state)** derive per-repo status and transition events (the seam) - (d96a39d) - chussenot
- **(stream)** implement the pwetty push contract for tile --stream - (580871f) - chussenot
- **(tile)** reshape the tile into a multi-repo payload - (c6ac26c) - chussenot
- **(watch)** notify-send on transitions, debounced, with follow-up commands - (10e8815) - chussenot
- **(why)** list attention items with evidence for one repo - (cff9c35) - chussenot
- implement the tile — readers, fold, state machine and the resume cursor - (9c1b3ea) - chussenot
#### Miscellaneous Chores
- **(beads)** commit the audit sidecar and ignore the plan manifest - (8b56cd3) - chussenot
- **(beads)** push the Dolt DB to an in-repo file remote that travels with git - (b2c313e) - chussenot
- **(beads)** initialize beads issue tracking - (8a185cf) - chussenot
- **(docs)** sweep stale post-pivot vigil references (quivive-3u0) - (d00ca8f) - chussenot
- **(ledger)** checkpoint — audit battery run, friction beads filed, sweep in flight - (43753b5) - chussenot
- **(ledger)** checkpoint — second-laptop sim passed (18/18 beads, one-row drift = the push boundary) - (0c2d38c) - chussenot
- **(ledger)** S18 wired and merged; unification follow-up filed post-v0.1; docs wave next - (e7ce0d7) - chussenot
- **(ledger)** wave-4 barrier — stream, goldens and the watch time-freeze fix merged - (82d2a06) - chussenot
- **(ledger)** checkpoint — goldens closed with the S6 bench at 23us/repo; watch-r3 in flight - (f0d2b87) - chussenot
- **(ledger)** checkpoint — stream closed; watch time-freeze bug filed (quivive-trx), watch-r3 on it - (8594da2) - chussenot
- **(ledger)** checkpoint — wave 4 spawned (stream, goldens) - (80cec16) - chussenot
- **(ledger)** wave-3 barrier — merge verified on the second attempt - (7d98827) - chussenot
- **(ledger)** checkpoint — why-r2 leased and working - (d116988) - chussenot
- **(ledger)** wave-3 merge rejected by its oracle — bug bead filed, fix in flight - (4bac082) - chussenot
- **(ledger)** checkpoint — integrate agent leased and working - (9960700) - chussenot
- **(ledger)** checkpoint — tile-r2 closed quivive-eea; integration bead quivive-s16 filed and claimed - (988ba97) - chussenot
- **(ledger)** checkpoint — watch-r2 closed quivive-8mq; tile-r2 in flight - (f9fb34d) - chussenot
- **(ledger)** checkpoint — r2 agents leased and working - (b2ac047) - chussenot
- **(ledger)** checkpoint after rate-limit deaths — leases reclaimed, r2 agents respawned - (1b223bd) - chussenot
- **(ledger)** mid-wave-3 checkpoint — why closed, tile and watch in flight - (c17925f) - chussenot
- **(ledger)** mid-wave-3 checkpoint of the append-only logs - (63fc123) - chussenot
- **(ledger)** mid-wave-3 checkpoint of the append-only logs - (4637ae7) - chussenot
- **(ledger)** mid-wave-3 checkpoint of the append-only logs - (b5564a9) - chussenot
- **(ledger)** mid-wave-3 checkpoint — pwetty closed, three beads in flight - (388d637) - chussenot
- **(ledger)** mid-wave-3 checkpoint of the append-only logs - (e6f91e3) - chussenot
- **(ledger)** wave-2 barrier — three branches merged atomically, graph pushed - (5c76259) - chussenot
- **(ledger)** mid-wave-2 checkpoint — state closed, readers still holds - (458933e) - chussenot
- **(ledger)** mid-wave-2 checkpoint — registry closed, two beads in flight - (2e6304b) - chussenot
- **(ledger)** mid-wave-2 checkpoint of the append-only logs - (035fec8) - chussenot
- **(ledger)** wave-1 barrier — merge proved, ledgers checkpointed, graph pushed - (882948b) - chussenot
- **(ledger)** checkpoint the plan snapshot and wave-1 lease activity - (e7102c8) - chussenot
- **(pact)** sync the coordination protocol block - (bc056ac) - chussenot
- **(plan)** snapshot the linted wave plan and push the bead graph - (d49da22) - chussenot
- **(world)** commit the Phase 0 ledger, merge attributes and gate evidence - (955c673) - chussenot
#### Tests
- **(bench)** measure S6's steady-state per-repo ceiling across a registry - (7a9826c) - chussenot
- **(cli)** adapt the binary surface tests to the reshaped payload - (70b9ec8) - chussenot
- **(cli)** update the suite to the quivive binary and the new surface - (d915da7) - chussenot
- **(fixtures)** add activity/plan/interaction builders to the support module - (61d239e) - chussenot
- **(goldens)** pin S13's five samples against real fixtures, synced with pwetty - (42230ce) - chussenot
