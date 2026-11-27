#!/usr/bin/env bash
# GTA6-btw: system wine 11.21 + rbtree wineserver + vkd3d-proton/DXVK dlls in the prefix.
# (proton-cachyos's wine stack-overflows its services.exe on this host, hanging every
# launcher path; the victory-ntdll optimizations are optional per linux/wine/README.md,
# and the rbtree server below IS the upstream fix for the loading-speed issue.)
set -euo pipefail

PROJ=/home/manse/A.Projects/GTA6-Emulation
VICTORY_TREE="${VICTORY_TREE:-victory-7900xtx}"
VICTORY_EXE="${VICTORY_EXE:-$PROJ/LeoTux/$VICTORY_TREE/target/x86_64-pc-windows-msvc/diag/victory.exe}"
GAME_EXE="${GAME_EXE:-$PROJ/GTA6.Xbox.v1.0.2.1-CYBERLEEK/game_scarlett_master.exe}"
# Both monitors are 1080p: series-s profile (1080p, less VRAM, ~5 ms faster, softer
# picture). VICTORY_CONSOLE=series-x env restores 4K.
VICTORY_CONSOLE="${VICTORY_CONSOLE:-series-s}"

defaults() { local kv name; for kv in "$@"; do name="${kv%%=*}"; [[ -n "${!name+x}" ]] || export "$kv"; done; }

export WINEPREFIX="$PROJ/victory-proton-umu/pfx"
# rbtree wineserver (views-tree fix; distro-11.21 clients only). wine's exec_wineserver
# tries bin_dir FIRST and WINESERVER only as fallback (loader.c), so the override is
# ignored — the server must be running for the prefix BEFORE wine starts.
RBTREE_SERVER="$PROJ/wine-patches/wineserver-11.21-rbtree"
defaults WINEDEBUG=-all \
         WINEDLLOVERRIDES="d3dcompiler_47=n;d3d12=n;d3d12core=n;dxgi=n" \
         MESA_VK_DEVICE_SELECT=1002:747e DRI_PRIME=1 \
         VKD3D_CONFIG=ignore_shared_fence \
         MESA_SHADER_CACHE_MAX_SIZE=10G \
         MESA_SHADER_CACHE_DIR="$PROJ/mesa-cache" \
         RUST_MIN_STACK=67108864

# Caches (per tree, on the SSD)
CACHE_DIR="${VICTORY_HOST_CACHE_DIR:-$PROJ/victory-cache/$VICTORY_TREE}"
mkdir -p "$CACHE_DIR/vkd3d" "$PROJ/GTA6-btw-logs" \
         "$WINEPREFIX/drive_c/users/manse/AppData/Local/$VICTORY_TREE/cache/pipelines"
defaults "VICTORY_CACHE_DIR=C:\\users\\manse\\AppData\\Local\\$VICTORY_TREE\\cache" \
         "VKD3D_SHADER_CACHE_PATH=Z:${CACHE_DIR//\//\\\\}\\vkd3d"

# VIctory settings (same defaults as run_victory_proton.sh)
defaults VICTORY_LOG=info "VICTORY_LOG_FILE=Z:\\home\\manse\\A.Projects\\GTA6-Emulation\\GTA6-btw-logs\\victory-trace.log" \
         VICTORY_NO_WATCHDOG=1 \
         VICTORY_SKIP_INT3=1 \
         VICTORY_HAIR=1 \
         VICTORY_PIPELINE_WAIT_MS=100
if [[ "${VICTORY_FORK_SETTINGS:-1}" != 0 ]]; then
  defaults VICTORY_TEXTURE_NO_WAIT=1 VICTORY_RELAY=2 VICTORY_RELAY_COPY=1 VICTORY_WIDE_BACK=1 \
           VICTORY_CARD_REHASH_RUNS=1 VICTORY_CARD_STEPS=2 VICTORY_CARD_AWAKE=1 \
           VICTORY_JOIN_COPIES=1 VICTORY_JOIN_PARTS=8 VICTORY_INDIRECT_WINDOW=1 VICTORY_INDIRECT_BATCH=2 \
           VICTORY_MAP_THREADS=1 VICTORY_TITLE_PRIORITIES=0 VICTORY_SHOW_AFTER=1 \
           VICTORY_THIN=1 VICTORY_UPLOAD_AHEAD=1 VICTORY_D3D12_DRED=light \
           VICTORY_AMBIENT=0 VICTORY_SKIP_PIPELINES=none \
           VICTORY_PIPELINE_BOOK=1 VICTORY_CARD_BUFFERS_MIN_MB=1 VICTORY_GAPS_TOGETHER=0 VICTORY_POOLS_MORE=0
fi

# Distro wine 11.21: the play-proven stack (run10: stable geometry, quick world, good
# faces). GE wine (11.0 staging) breaks skinned geometry (eyes/teeth only) — see
# GTA6-Emulation/docs/gta6btw-setup.md.
WINE="${WINE:-/usr/bin/wine}"
echo "tree:        $VICTORY_TREE"
echo "victory.exe: $VICTORY_EXE"
echo "wine:        $WINE (server: $RBTREE_SERVER)"
echo "prefix:      $WINEPREFIX"
echo "console:     $VICTORY_CONSOLE"

# Kill any stock server for the prefix, then pre-start the rbtree one (see above).
/usr/bin/wineserver -k 2>/dev/null || true
if [[ "${RBTREE:-1}" == 1 && -x "$RBTREE_SERVER" ]]; then
  "$RBTREE_SERVER" -p
fi

if [[ "${1:-}" == "warm" ]]; then
  shift
  [[ $# -gt 0 ]] || set -- --used
  exec "$WINE" "$VICTORY_EXE" warm "$@"
fi

if [[ "${VICTORY_WARM_FIRST:-1}" == 1 ]]; then
  echo "compiling shaders before the title starts..."
  "$WINE" "$VICTORY_EXE" warm --used || echo "warning: warm failed; starting anyway" >&2
  echo "shaders ready"
fi
exec "$WINE" "$VICTORY_EXE" run --execute --console "$VICTORY_CONSOLE" "$GAME_EXE" "$@"
