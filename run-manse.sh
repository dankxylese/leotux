#!/usr/bin/env bash
# GTA6-btw launcher for this machine: all caches/prefix on the SSD inside GTA6-Emulation.
set -euo pipefail

PROJ=/home/manse/A.Projects/GTA6-Emulation

export GTA6_DIR="$PROJ/GTA6.Xbox.v1.0.2.1-CYBERLEEK"
export PROTON="$PROJ/GTA6-btw/proton-umu.sh"
export STEAM_COMPAT_DATA_PATH="$PROJ/victory-proton-umu"
export STEAM_COMPAT_CLIENT_INSTALL_PATH="$HOME/.local/share/Steam"
export VICTORY_HOST_CACHE_DIR="$PROJ/victory-cache/victory-7900xtx"
export LOG_DIR="$PROJ/GTA6-btw-logs"
export VICTORY_LOG_FILE='Z:\home\manse\A.Projects\GTA6-Emulation\GTA6-btw-logs\victory-trace.log'
export MESA_SHADER_CACHE_DIR="$PROJ/mesa-cache"
export VICTORY_TREE=victory-7900xtx
export VICTORY_CONSOLE=series-x

mkdir -p "$LOG_DIR"
exec "$PROJ/GTA6-btw/scripts/run_victory_proton.sh" "$@"
