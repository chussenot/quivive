#!/usr/bin/env bash
# Soak the resume cursor against concurrent writers, and assert the one invariant
# the whole design rests on.
#
# ## The experiment
#
# The claim in docs/adr/0001-stream-first-tile.md is that the cursor is CORRECT TO
# THROW AWAY: delete it, re-read everything, get a byte-identical tile. The unit
# tests assert that over fixtures they wrote themselves. This asserts it over a
# cursor that was advanced, tick by tick, through writes it did not control —
# which is the only condition under which the interesting failures happen:
#
#   * a tick landing mid-append, seeing half a line
#   * a rewrite (pact compacts events.jsonl to its newest 4000 lines once it
#     passes 5000) landing between two ticks
#   * a rewrite that then grows back PAST the old offset, which a byte offset
#     alone cannot detect
#
# Three phases:
#   1. Workers append concurrently while the treatment ticks. Every tick must
#      exit 0 and report ZERO declines — a partial line is not damage, and a run
#      that reports damage here has found a real bug.
#   2. A rewrite is injected mid-run, twice, then the file is grown back past
#      where the cursor was.
#   3. Writers stop. The treatment ticks once more (resuming the cursor built up
#      under all of the above) and the control ticks with --no-cursor. The two
#      tiles must be byte-identical.
#
# ## Why half the writers stop early
#
# The first version of this script ran every worker for the whole run, and a
# NEGATIVE CONTROL — a build whose cursor trusted its byte offset without
# verifying the tail — passed it. That comparison proved nothing, for a reason
# worth writing down: the fold keeps only the NEWEST evidence per agent, so a
# resumed read that skips events in the middle of the file reaches the same
# maximum as a cold read. Skipping is invisible to a max.
#
# The scenario that discriminates needs three things in this exact order, and a
# second draft that got the order wrong still proved nothing:
#
#   1. a rewrite the cursor DOES notice (tick 10), which resets the accumulator
#   2. agents that then go quiet (tick 15), so the accumulator learns them
#   3. a rewrite that drops those agents' lines and then GROWS THE FILE BACK past
#      the cursor's old offset (tick 25), so a length comparison cannot notice
#
# Only then does a cold read fail to know the quiet agents exist while a trusting
# cursor still carries them — and the phase-3 comparison has something to find.
# Silencing at tick 5 instead put step 2 before step 1, and the shrink at tick 10
# wiped the accumulator clean before the interesting rewrite ever happened.
#
# The script verifies at tick 26 that the silenced agents really did vanish from
# the ledger, because if they did not, the comparison is back to proving nothing.
#
# The clock is frozen with QUIVIVE_NOW for phase 3, because a tile carries the
# instant it was computed at and two invocations a millisecond apart are
# legitimately different tiles. Freezing it is what makes "byte-identical" a
# statement about the fold rather than about the clock.
#
# Deliberately NOT part of `mise run check`: it is seconds-to-minutes long and
# concurrent. Run it when you have touched a reader, the fold or the cursor.
#
# Usage: scripts/fleet-sim.sh [-n workers] [-t ticks]

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

workers=8
ticks=40
while [ $# -gt 0 ]; do
	case "$1" in
	-n)
		workers="$2"
		shift 2
		;;
	-t)
		ticks="$2"
		shift 2
		;;
	*)
		echo "usage: $0 [-n workers] [-t ticks]" >&2
		exit 2
		;;
	esac
done

BIN=${BIN:-./target/debug/quivive}
[ -x "$BIN" ] || {
	echo "fleet-sim: $BIN not built — run \`mise run build\` first" >&2
	exit 2
}

work=$(mktemp -d)
repo="$work/repo"
ledger="$repo/.pact/events.jsonl"
mkdir -p "$repo/.pact"
: >"$ledger"

pids=()
cleanup() {
	for p in ${pids+"${pids[@]}"}; do kill "$p" 2>/dev/null; done
	wait 2>/dev/null
	rm -rf "$work"
}
trap cleanup EXIT

fail=0
problem() {
	printf 'FLEET: %s\n' "$1" >&2
	fail=1
}

# ---------------------------------------------------------------- phase 1 and 2

# One writer per agent, appending in pact's shape. A single small `printf` to a
# file opened O_APPEND is what pact itself relies on for atomicity; the point of
# this harness is NOT to be more careful than pact is, because a tick has to
# survive what pact actually produces.
for i in $(seq 1 "$workers"); do
	(
		while :; do
			printf '{"at":"%s","agent":"agent-%s","kind":"acquired","path":"src/f%s.rs","detail":null,"ttl_secs":900}\n' \
				"$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)" "$i" "$RANDOM" >>"$ledger"
			sleep 0.01
		done
	) &
	pids+=("$!")
done

echo "phase 1: $workers writers, $ticks ticks, cursor advancing under concurrent appends"
declines=0
# Half the writers stop here: AFTER the shrink at tick 10 and BEFORE the
# grow-back at tick 25. The order is the whole experiment, and a draft that
# silenced them at tick 5 proved nothing — the shrink at 10 resets the
# accumulator, so by tick 25 there was no stale memory left to expose. See the
# header.
quiet_at=15
quieted=0
for t in $(seq 1 "$ticks"); do
	# Silence the first half of the fleet early. Their lines are then compacted
	# away by the rewrite below, which is the only condition under which a
	# non-verifying cursor produces a different tile from a cold read. See the
	# header.
	if [ "$t" -eq "$quiet_at" ] && [ "$quieted" -eq 0 ]; then
		half=$((workers / 2))
		for idx in $(seq 0 $((half - 1))); do
			kill "${pids[$idx]}" 2>/dev/null
		done
		quieted=1
		echo "  tick $t: silenced $half of $workers writers"
	fi
	out=$("$BIN" tile --json --repo "$repo" 2>&1)
	rc=$?
	[ "$rc" -eq 0 ] || problem "tick $t exited $rc: $out"
	# A partial line is not damage. If this fires, the reader is consuming bytes
	# past the last newline — the single most likely bug in the crate.
	if printf '%s' "$out" | grep -q 'unparsable'; then
		declines=$((declines + 1))
		problem "tick $t reported unparsable lines while writers were appending: $(printf '%s' "$out" | grep -o '"[^"]*unparsable[^"]*"')"
	fi

	# Phase 2, injected twice: a rewrite, exactly as pact compacts. The second one
	# shrinks the file well below the cursor and lets the writers grow it back
	# past that offset, which is the case a byte offset alone cannot catch.
	# Two rewrites, exercising the TWO different ways a stale cursor is detected.
	# They are not interchangeable, and a second draft of this script learned that
	# the hard way: with only the first shape, a negative control whose cursor
	# never verified its tail passed the whole run.
	#
	# Tick 10 — a plain shrink. The file ends up shorter than the cursor's offset,
	# which a length comparison alone catches.
	if [ "$t" -eq 10 ]; then
		tail -n 5 "$ledger" >"$ledger.compact" && mv "$ledger.compact" "$ledger"
		echo "  tick $t: rewrote the ledger to its newest 5 lines (shrink)"
	fi
	# Tick 25 — a rewrite that GROWS BACK past the old offset before the next tick.
	# A length comparison passes here, and only the tail hash can tell that the
	# bytes at that offset are no longer the line we consumed. This is the one case
	# a byte offset structurally cannot detect, and the only one in this script
	# that distinguishes a verifying cursor from a trusting one.
	if [ "$t" -eq 25 ]; then
		before=$(wc -c <"$ledger")
		tail -n 5 "$ledger" >"$ledger.compact" && mv "$ledger.compact" "$ledger"
		# Back past $before, in one go, before the next tick can look. Only the
		# still-active half is named, so the silenced agents stay absent from the
		# file and a cursor that kept them shows up as a difference.
		while [ "$(wc -c <"$ledger")" -le "$before" ]; do
			for idx in $(seq $((workers / 2 + 1)) "$workers"); do
				printf '{"at":"%s","agent":"agent-%s","kind":"renewed","path":"src/g%s.rs","ttl_secs":900}\n' \
					"$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)" "$idx" "$RANDOM" >>"$ledger"
			done
		done
		echo "  tick $t: rewrote, then grew back past the old offset ($before -> $(wc -c <"$ledger") bytes)"
	fi
	# The grow-back at tick 25 must actually have dropped the silenced agents, or
	# phase 3 is comparing two reads that could not have differed. Checked here,
	# where the evidence is still on disk.
	if [ "$t" -eq 26 ]; then
		half=$((workers / 2))
		still=0
		for idx in $(seq 1 "$half"); do
			grep -q "\"agent-$idx\"" "$ledger" && still=$((still + 1))
		done
		if [ "$still" -ne 0 ]; then
			problem "$still silenced agent(s) still in the ledger after the rewrite; phase 3 would prove nothing"
		else
			echo "  tick $t: the rewrite dropped every silenced agent, as the comparison needs"
		fi
	fi
	sleep 0.05
done
[ "$declines" -eq 0 ] && echo "  no tick reported a decline"

# ------------------------------------------------------------------------ phase 3

for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done
wait 2>/dev/null
pids=()
sleep 0.2 # let any write already in flight land, so the two reads see one file
lines=$(wc -l <"$ledger")
echo "phase 3: writers stopped, ledger is $lines lines; comparing warm against cold"

# Frozen, and this is the load-bearing line of the whole script.
export QUIVIVE_NOW="2026-08-28T09:00:00Z"

warm=$("$BIN" tile --json --repo "$repo")
cold=$("$BIN" tile --json --repo "$repo" --no-cursor)
rm -f "$repo/.pact/quivive-cursor.json"
recold=$("$BIN" tile --json --repo "$repo")

if [ "$warm" != "$cold" ]; then
	problem "warm and --no-cursor disagree — the cursor is NOT correct to throw away"
	diff <(printf '%s\n' "$warm") <(printf '%s\n' "$cold") | head -20 >&2
fi
if [ "$cold" != "$recold" ]; then
	problem "deleting the cursor changed the tile"
	diff <(printf '%s\n' "$cold") <(printf '%s\n' "$recold") | head -20 >&2
fi

# The control the invariant needs to be a measurement rather than a tautology: if
# the tile were empty, or the same for every input, the comparison above would
# pass while proving nothing. This is the "did the workload actually contend"
# check that a green treatment on its own cannot give you.
agents=$(printf '%s' "$warm" | grep -c '"id":')
if [ "$agents" -lt 1 ]; then
	problem "the tile names no agents at all — the comparison above proved nothing"
elif [ "$fail" -eq 0 ]; then
	echo "  tile names $agents agent(s); warm, cold and re-cold all agree"
else
	# Do not print "all agree" under a failure. A summary line that contradicts
	# the diff above it is how a red run gets read as a green one.
	echo "  tile names $agents agent(s); see the disagreement above"
fi

if [ "$fail" -ne 0 ]; then
	echo "fleet-sim: FAILED" >&2
	exit 1
fi
echo "fleet-sim: ok"
