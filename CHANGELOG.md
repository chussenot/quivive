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

<!-- cocogitto inserts each new release below the `- - -` separator. The front
     matter and this preamble sit above it and are preserved across bumps; do not
     move the separator. -->

- - -

## Unreleased

No release yet, and no tag. The crate exists and `mise run check` is green, but
nothing here has been used on a real bar during a real fleet run, which is the
only evidence that would justify calling a version 0.1.0.

Order of construction, since it is unusual and deliberate: the house conventions
and the docs gate landed first, then the decision records, then the specification
and the tile contract — and only then the crate, which implemented the contract
unchanged. [docs/studies/conventions-run.md](docs/studies/conventions-run.md)
records what that bought, what it cost, and the six times during both runs that a
check passed for a reason other than the one claimed.

The first release will be cut with `cog bump` — never by editing a version by
hand — and will carry the first Notes paragraph.
