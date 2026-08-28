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

No release yet. There is no crate to version: this repository's conventions,
decision records and specification landed first, deliberately, and
[docs/studies/conventions-run.md](docs/studies/conventions-run.md) records what
that bought and what it cost.

The first release will be cut with `cog bump` — never by editing a version by
hand — and will carry the first Notes paragraph.
