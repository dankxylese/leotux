#!/usr/bin/env bash
# PROTON drop-in for run_victory_proton.sh: routes `proton <verb> <exe> [args]` through umu-run
# (proton-cachyos's steam.exe launch path fails outside Steam/umu).
set -euo pipefail
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/proton-cachyos-11.0-20260703-slr-x86_64_v3"
export GAMEID=gta6btw
verb="${1:-run}"
shift || true
case "$verb" in
  run|waitforexitandrun|runinprefix) exec umu-run "$@" ;;
  *) echo "proton-umu.sh: unsupported verb: $verb" >&2; exit 1 ;;
esac
