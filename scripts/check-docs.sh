#!/usr/bin/env bash
# Fail if the documentation drifts from its own conventions.
#
# The house rules in docs/conventions.md are only rules because this script
# enforces them. Front matter that nothing checks is front matter that half the
# files will not have by the tenth commit; an ADR index nothing checks is an
# index that stops listing the newest ADR; a link nothing checks rots silently,
# which is the worst kind, because a reader trusts it.
#
# Six checks. Each one exists because the convention it guards is cheap to
# forget and invisible when broken:
#
#   1. Every markdown file has YAML front matter carrying `title`, `status` and
#      `date`. `status` is one of draft|active|superseded; `date` is ISO. This is
#      what makes the docs greppable — "which pages are still draft" has to be
#      one command, or nobody asks it.
#   2. Every `.claude/agents/*.md` additionally carries `name` and `description`
#      (the harness reads those), and `name` agrees with `title`. A renamed role
#      whose title still says the old name is a role two files disagree about.
#   3. ADRs: filename `NNNN-<slug>.md`, front matter also carrying
#      `decision-makers` and `supersedes`, and a body with all four required
#      sections. An ADR without priced alternatives is a decision nobody can
#      re-open, which is the whole reason for writing one down.
#   4. Every ADR appears in docs/adr/README.md's index and every index row points
#      at a real ADR — both directions, because a one-directional check only
#      catches the half of drift you happened to think of.
#   5. Every relative link and every #anchor in every markdown file resolves.
#   6. Diagrams are diffable text: no image file under docs/, no markdown image
#      embed anywhere, and no unclosed fenced block.
#
# The file list is walked at runtime, never hardcoded here — a hardcoded list is
# the same drift problem one level down.
#
# Usage: scripts/check-docs.sh

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

fail=0
note() {
	printf 'DRIFT: %s\n' "$1" >&2
	fail=1
}

# `find`, not `git ls-files`: an untracked page is a brand-new one, which is
# exactly the page most likely to be missing its front matter or carrying a
# broken link.
mapfile -t docs < <(find . -name '*.md' \
	-not -path './target/*' -not -path './.git/*' -not -path './wt/*' | sort)

[ "${#docs[@]}" -gt 0 ] || {
	echo "check-docs: found no markdown at all — run me from the repo" >&2
	exit 2
}

# The front-matter block, or empty if the file does not open with one. `---` must
# be line 1: a block further down is not front matter, it is a horizontal rule,
# and every parser that reads these files agrees on that.
front_matter() {
	awk 'NR==1 { if ($0 != "---") exit } NR>1 { if ($0 == "---") exit; print }' "$1"
}

has_key() { grep -qE "^$2:[[:space:]]*[^[:space:]]" <<<"$1"; }
value_of() { sed -nE "s/^$2:[[:space:]]*//p" <<<"$1" | head -1 | sed -E 's/^"(.*)"$/\1/; s/^'\''(.*)'\''$/\1/'; }

echo "front matter"
for f in "${docs[@]}"; do
	fm=$(front_matter "$f")
	if [ -z "$fm" ]; then
		note "$f has no YAML front matter (needs title, status, date)"
		continue
	fi
	for k in title status date; do
		has_key "$fm" "$k" || note "$f front matter is missing \`$k\`"
	done
	status=$(value_of "$fm" status)
	case "$status" in
	draft | active | superseded | "") ;;
	*) note "$f front matter has status \`$status\`; allowed: draft, active, superseded" ;;
	esac
	date=$(value_of "$fm" date)
	if [ -n "$date" ] && ! [[ $date =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
		note "$f front matter has date \`$date\`; expected YYYY-MM-DD"
	fi

	# Agent files are read by the harness as well as by people, so they carry the
	# keys it needs on top of the house three. `name` is the address the
	# orchestrator calls the role by; `title` is what a reader sees. They must
	# agree, or a rename has landed in one file and not the other.
	case "$f" in ./.claude/agents/*)
		for k in name description; do
			has_key "$fm" "$k" || note "$f front matter is missing \`$k\` (the harness reads it)"
		done
		n=$(value_of "$fm" name)
		t=$(value_of "$fm" title)
		[ -z "$n" ] || [ -z "$t" ] || [ "$n" = "$t" ] \
			|| note "$f front matter: name \`$n\` and title \`$t\` disagree"
		;;
	esac
done

echo "decision records"
mapfile -t adrs < <(find docs/adr -name '*.md' -not -name 'README.md' 2>/dev/null | sort)
for f in "${adrs[@]}"; do
	base=$(basename "$f")
	[[ $base =~ ^[0-9]{4}-[a-z0-9]+(-[a-z0-9]+)*\.md$ ]] \
		|| note "$f is not named NNNN-<lower-kebab-slug>.md"

	fm=$(front_matter "$f")
	for k in decision-makers supersedes; do
		has_key "$fm" "$k" || note "$f front matter is missing \`$k\` (required on an ADR; use \`supersedes: none\`)"
	done

	# Four sections, and the third is the one that gets dropped. A decision
	# without priced alternatives cannot be re-opened by anyone who was not in
	# the room, which defeats the point of writing it down.
	for h in '## Context' '## Decision' '## Alternatives considered' '## Consequences'; do
		grep -qxF "$h" "$f" || note "$f has no \`$h\` section"
	done

	# A superseded ADR that does not name its successor is a dead end: the reader
	# knows the decision changed and has nowhere to go.
	if [ "$(value_of "$fm" status)" = "superseded" ]; then
		grep -qE 'docs/adr/[0-9]{4}-|\]\([0-9]{4}-' "$f" \
			|| note "$f is superseded but links to no successor ADR"
	fi

	# `supersedes:` must point at something real, in this directory.
	sup=$(value_of "$fm" supersedes)
	case "$sup" in
	none | "" | "[]") ;;
	*)
		for n in $(grep -oE '[0-9]{4}' <<<"$sup"); do
			ls docs/adr/"$n"-*.md >/dev/null 2>&1 \
				|| note "$f says it supersedes $n, and no docs/adr/$n-*.md exists"
		done
		;;
	esac
done

# Both directions. The index is the only page a reader lands on first, so an ADR
# missing from it is an ADR nobody reads.
if [ -f docs/adr/README.md ]; then
	for f in "${adrs[@]}"; do
		grep -qF "$(basename "$f")" docs/adr/README.md \
			|| note "docs/adr/README.md does not list $(basename "$f")"
	done
	while read -r listed; do
		[ -f "docs/adr/$listed" ] || note "docs/adr/README.md lists $listed, which does not exist"
	done < <(grep -oE '\]\([0-9]{4}-[a-z0-9-]+\.md\)' docs/adr/README.md | sed -E 's/^\]\(//; s/\)$//' | sort -u)
elif [ "${#adrs[@]}" -gt 0 ]; then
	note "docs/adr/ has records but no README.md index"
fi

echo "links and anchors"
# Slugs the way GitHub makes them: lowercase, punctuation dropped, spaces to `-`.
slugs_of() {
	grep -E '^#{1,6} ' "$1" | sed -E 's/^#+ //' \
		| tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9 _-]//g; s/ /-/g'
}
for src in "${docs[@]}"; do
	# Process substitution, NOT a pipe into `while`: a piped loop runs in a
	# subshell, so every `fail=1` it set would be discarded and this half of the
	# checker could not fail the script no matter what it found.
	#
	# `|| true` on the grep: a file with no links exits 1, and under `pipefail`
	# that would abort the pass silently — an abort that looks exactly like the
	# success it is pretending to be.
	while read -r t; do
		[ -n "$t" ] || continue
		case "$t" in http*| mailto:*) continue ;; esac
		path=${t%%#*}
		frag=${t#*#}
		[ "$frag" = "$t" ] && frag=""
		if [ -z "$path" ]; then target="$src"; else target="$(dirname "$src")/$path"; fi
		if [ ! -e "$target" ]; then
			note "$src: link target does not exist: $t"
			continue
		fi
		if [ -n "$frag" ] && [ -f "$target" ]; then
			slugs_of "$target" | grep -qxF "$frag" \
				|| note "$src: no heading matches anchor #$frag in ${path:-$src}"
		fi
	done < <(grep -oE '\]\([^)#][^)]*\)|\]\(#[^)]*\)' "$src" | sed -E 's/^\]\(//; s/\)$//' || true)
done

echo "diagrams are text"
# Mermaid in-markdown, never an image file: a diagram you cannot diff is a
# diagram that stops matching the system it draws and nobody can see that it has.
while read -r img; do
	note "docs/ contains an image file: $img — diagrams are Mermaid in markdown"
done < <(find docs -type f \( -name '*.png' -o -name '*.jpg' -o -name '*.jpeg' -o -name '*.svg' -o -name '*.gif' -o -name '*.webp' \) 2>/dev/null)

for f in "${docs[@]}"; do
	grep -qE '!\[[^]]*\]\(' "$f" && note "$f embeds an image; diagrams are Mermaid in markdown"
	# An unclosed fence swallows the rest of the page on every renderer. Cheap to
	# do, invisible in a diff, and it has to be an odd count to be wrong.
	fences=$(grep -cE '^[[:space:]]*```' "$f")
	[ $((fences % 2)) -eq 0 ] || note "$f has an unclosed fenced block ($fences fence lines)"
done

echo "no orphan pages"
# A page nothing links to is a page nobody finds, and the README index is the
# thing that rots first.
#
# Three exemptions, all of them pages a *tool* opens rather than a reader
# following a link: README.md and CLAUDE.md are entry points discovered by name
# (by GitHub and by the agent harness respectively), CHANGELOG.md is owned by
# cocogitto and reached from the release process, and docs/adr/README.md is
# itself an index. Everything else has to be reachable from prose.
for f in "${docs[@]}"; do
	case "$f" in ./README.md | ./CHANGELOG.md | ./CLAUDE.md | ./docs/adr/README.md) continue ;; esac
	base=$(basename "$f")
	found=0
	for src in "${docs[@]}"; do
		[ "$src" = "$f" ] && continue
		grep -qE "\]\([^)]*$base(#[^)]*)?\)" "$src" && {
			found=1
			break
		}
	done
	[ "$found" = 1 ] || note "$f is linked from no other page — add it to an index"
done

if [ "$fail" -ne 0 ]; then
	echo "check-docs: FAILED" >&2
	exit 1
fi
echo "check-docs: ok (${#docs[@]} markdown files, ${#adrs[@]} decision records)"
