#!/usr/bin/env bash
# Toggle the cffi/pwetty#quivive module out of / back into modules-right in
# this machine's waybar config, then restart waybar so the change takes
# effect. Personal-desktop convenience, not part of `mise run check` — it
# hardcodes $HOME/.config/waybar/config.jsonc and does nothing useful in CI.
set -euo pipefail

usage() {
	echo "usage: $(basename "$0") on|off" >&2
	exit 1
}
[ $# -eq 1 ] || usage

config="$HOME/.config/waybar/config.jsonc"
[ -f "$config" ] || {
	echo "waybar-quivive-toggle: no waybar config at $config" >&2
	exit 1
}

case "$1" in
off)
	sed -i -E 's|^([[:space:]]*)"cffi/pwetty#quivive",|\1// "cffi/pwetty#quivive",|' "$config"
	;;
on)
	sed -i -E 's|^([[:space:]]*)// "cffi/pwetty#quivive",|\1"cffi/pwetty#quivive",|' "$config"
	;;
*)
	usage
	;;
esac

pkill waybar 2>/dev/null || true
sleep 0.3
nohup waybar >/dev/null 2>&1 &
disown
echo "waybar-quivive-toggle: quivive tile turned $1, waybar restarted"
