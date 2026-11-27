#!/usr/bin/env bash
# Runs a VIctory tree of this repository under a Proton compat tool.
#
#   scripts/run_victory_proton.sh [args for the title]   compile shaders ahead, then play
#   scripts/run_victory_proton.sh warm [args]            only compile shaders ahead (`victory warm`)
#
# Trees (VICTORY_TREE):
#   victory-7900xtx (default)  avatarmanz's RDNA 3 fork + zeex64's Linux work + our Wine pieces
#   victory                    Belobug's fork as of 2026-11-25 + Scarlight's Wine fixes
#
# Every setting below is a default: a variable already set in the environment wins.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="$HOME/.local/share/Steam/compatibilitytools.d"

# Sets each NAME=value given unless NAME is already set.
defaults() {
  local kv name
  for kv in "$@"; do
    name="${kv%%=*}"
    [[ -n "${!name+x}" ]] || export "$kv"
  done
}

# --- 1. Where things are -------------------------------------------------------------------

VICTORY_TREE="${VICTORY_TREE:-victory-7900xtx}"
GTA6_DIR="${GTA6_DIR:-$HOME/Downloads/GTA6.Xbox.v1.0.2.1}"
GAME_EXE="${GAME_EXE:-$GTA6_DIR/game_scarlett_master.exe}"
# The Proton built from ../Proton (proton-btw: faster mapping in the Wine server, see README.md);
# Proton-CachyOS-Victory when it is not installed.
if [[ -z "${PROTON:-}" ]]; then
  PROTON="$TOOLS/victory-ntdll/proton"
  [[ -x "$PROTON" ]] || PROTON="$TOOLS/Proton-CachyOS-Victory/proton"
fi
COMPAT_DATA="${STEAM_COMPAT_DATA_PATH:-$HOME/.local/share/victory-proton}"
STEAM_CLIENT="${STEAM_COMPAT_CLIENT_INSTALL_PATH:-$HOME/.local/share/Steam}"
TARGET_DIR="$ROOT/$VICTORY_TREE/target/x86_64-pc-windows-msvc"
VICTORY_EXE="${VICTORY_EXE:-}"
if [[ -z "$VICTORY_EXE" ]]; then
  for candidate in "$TARGET_DIR"/{diag,release,debug}/victory.exe; do
    if [[ -f "$candidate" ]]; then VICTORY_EXE="$candidate"; break; fi
  done
fi
LOG_DIR="${LOG_DIR:-/tmp/opencode}"
mkdir -p "$LOG_DIR"

# --- 2. Proton, Wine, the driver -----------------------------------------------------------

export STEAM_COMPAT_CLIENT_INSTALL_PATH="$STEAM_CLIENT"
export STEAM_COMPAT_DATA_PATH="$COMPAT_DATA"
defaults WINEDEBUG=-all \
         MESA_VK_DEVICE_SELECT=1002:747e DRI_PRIME=1 \
         VKD3D_CONFIG=ignore_shared_fence \
         MESA_SHADER_CACHE_MAX_SIZE=10G
# Under vkd3d-proton a pipeline's DXIL becomes SPIR-V on the stack of the thread that asks for
# it; the largest compute programs overflow Rust's default 2 MB thread stack, and under Wine the
# thread then dies silently with the pipeline "being made" for good (zeex64, linux/run.sh).
defaults RUST_MIN_STACK=67108864

# --- 3. Caches (kept between runs, out of the repository; one set per tree) -----------------

if [[ "$VICTORY_TREE" == "victory" ]]; then
  CACHE_DIR="${VICTORY_HOST_CACHE_DIR:-$HOME/.cache/victory}"
else
  CACHE_DIR="${VICTORY_HOST_CACHE_DIR:-$HOME/.cache/$VICTORY_TREE}"
  # VIctory's own: translations, the pipeline book and early list (saves stay shared).
  defaults "VICTORY_CACHE_DIR=C:\\users\\steamuser\\AppData\\Local\\$VICTORY_TREE\\cache"
fi
mkdir -p "$CACHE_DIR/vkd3d"
# vkd3d-proton's compiled pipelines (else written beside the working directory).
defaults "VKD3D_SHADER_CACHE_PATH=Z:${CACHE_DIR//\//\\}\\vkd3d"
rm -f "$ROOT/vkd3d-proton.cache.write"

# --- 4. The prefix -------------------------------------------------------------------------

# VIctory compiles helper shaders with D3DCompile; Wine's d3dcompiler_47 fails on the
# indirect-argument clamp and every indirect draw is dropped (black world).
D3DC="$COMPAT_DATA/pfx/drive_c/windows/system32/d3dcompiler_47.dll"
if [[ ! -f "$D3DC" ]] || grep -qa "Wine builtin DLL" "$D3DC"; then
  echo "warning: no Microsoft d3dcompiler_47.dll in the prefix (winetricks d3dcompiler_47); the 3D world will stay black" >&2
fi

# --- 5. VIctory settings -------------------------------------------------------------------

defaults VICTORY_LOG=info "VICTORY_LOG_FILE=Z:\\tmp\\opencode\\victory-trace.log" \
         VICTORY_NO_WATCHDOG=1 \
         VICTORY_SKIP_INT3=1 \
         VICTORY_HAIR=1 \
         VICTORY_PIPELINE_WAIT_MS=100
# Series X (default) or series-s: 1080p, less video memory, about 5 ms faster, a softer picture.
VICTORY_CONSOLE="${VICTORY_CONSOLE:-series-x}"

# The newer trees: their forks' launcher settings (scripts/play.ps1, without -Safe) plus zeex64's
# Linux measurements (linux/local.env.example). VICTORY_FORK_SETTINGS=0 leaves all of them out.
#   VICTORY_AMBIENT=0         the title's own ambient light (the forks' 0.8 stand-in flattened skin)
#   VICTORY_SKIP_PIPELINES=none   nothing skipped (needs RUST_MIN_STACK above)
#   VICTORY_PIPELINE_BOOK=1   every pipeline written down, for `warm`
#   VICTORY_BVH_BUILDER=1     not set: ray tracing (the phone's screen, indirect light), much slower
case "$VICTORY_TREE" in
  victory-7900xtx) NEWER=1 ;;
  *) NEWER=0 ;;
esac
if [[ "$NEWER" == 1 && "${VICTORY_FORK_SETTINGS:-1}" != 0 ]]; then
  defaults VICTORY_TEXTURE_NO_WAIT=1 VICTORY_RELAY=2 VICTORY_RELAY_COPY=1 VICTORY_WIDE_BACK=1 \
           VICTORY_CARD_REHASH_RUNS=1 VICTORY_CARD_STEPS=2 VICTORY_CARD_AWAKE=1 \
           VICTORY_JOIN_COPIES=1 VICTORY_JOIN_PARTS=8 VICTORY_INDIRECT_WINDOW=1 VICTORY_INDIRECT_BATCH=2 \
           VICTORY_MAP_THREADS=1 VICTORY_TITLE_PRIORITIES=0 VICTORY_SHOW_AFTER=1 \
           VICTORY_THIN=1 VICTORY_UPLOAD_AHEAD=1 VICTORY_D3D12_DRED=light \
           VICTORY_AMBIENT=0 VICTORY_SKIP_PIPELINES=none \
           VICTORY_PIPELINE_BOOK=1 VICTORY_CARD_BUFFERS_MIN_MB=1 VICTORY_GAPS_TOGETHER=0 VICTORY_POOLS_MORE=0
fi
# Older trees: the three compute pipelines that never finished before RUST_MIN_STACK stay skipped.
defaults VICTORY_SKIP_PIPELINES=559d9f0184e15fe3,a1b20bc47e3c9597,e8c962358d862e3f

# --- 6. Run --------------------------------------------------------------------------------

if [[ ! -x "$PROTON" ]]; then
  echo "Proton not found: $PROTON (set PROTON=<compat tool>/proton)" >&2
  exit 1
fi
if [[ -z "$VICTORY_EXE" || ! -f "$VICTORY_EXE" ]]; then
  echo "victory.exe not found under $TARGET_DIR. Build it:" >&2
  echo "  cd $VICTORY_TREE && cargo build --profile diag -p victory --features diagnostics --bins --target x86_64-pc-windows-msvc" >&2
  exit 1
fi
echo "tree:        $VICTORY_TREE"
echo "victory.exe: $VICTORY_EXE"
echo "proton:      $PROTON"
echo "console:     $VICTORY_CONSOLE"
echo "trace log:   $VICTORY_LOG_FILE"

if [[ "${1:-}" == "warm" ]]; then
  shift
  [[ $# -gt 0 ]] || set -- --used
  exec "$PROTON" run "$VICTORY_EXE" warm "$@"
fi

# Shaders first: the pipelines earlier runs used are compiled while a "Compiling shaders" window
# is up, so the intro waits for none. VICTORY_WARM_FIRST=0 starts the title at once.
if [[ "${VICTORY_WARM_FIRST:-$NEWER}" == 1 ]]; then
  echo "compiling shaders before the title starts..."
  "$PROTON" run "$VICTORY_EXE" warm --used > /dev/null 2>&1 &
  WARM=$!
  DIALOG=
  if command -v zenity > /dev/null && [[ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]]; then
    ( while kill -0 "$WARM" 2> /dev/null; do echo "#Compiling shaders... ${SECONDS}s"; sleep 1; done; echo 100 ) |
      zenity --progress --pulsate --auto-close --no-cancel --title="LeoTux" \
        --text="Compiling shaders..." --width=320 2> /dev/null &
    DIALOG=$!
  fi
  wait "$WARM" || echo "warning: compiling shaders ahead failed; the title starts anyway" >&2
  [[ -z "$DIALOG" ]] || wait "$DIALOG" 2> /dev/null || true
  echo "shaders ready"
fi
exec "$PROTON" run "$VICTORY_EXE" run --execute --console "$VICTORY_CONSOLE" "$GAME_EXE" "$@"
