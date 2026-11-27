#!/bin/bash
# Linux counterpart of scripts/play.ps1: runs victory.exe under Wine, with vkd3d-proton (D3D12)
# and DXVK's dxgi copied from Proton into the prefix (Proton's own launcher hangs outside Steam).
#   scripts/play.sh [options] [--] <game exe> [KEY=VALUE ...]
#   scripts/play.sh [--wine system|proton] --cmd <victory args...>   # shaders, warm, config, ...
# Options:
#   --safe            without the newer, faster ways (play.ps1 -Safe)
#   --rt              the title's ray-traced indirect light
#   --windowed        a window instead of the whole screen
#   --no-hud          without MangoHud
#   --start           presses Enter every 4 s from 12 s to 48 s (VICTORY_KEYS, diag builds): from
#                     the profile screen to the first scene without anyone at the keyboard, the
#                     world's first frame 33 s after the launch. (From 52 s for a day: the sooner
#                     the scene started, the oftener nobody but the heroine had a head in it.
#                     That was a compute pass dropped, not the time: docs/linux-wine.md, "People
#                     without a head"; 8 runs of 8 with heads at 33 s since.)
#                     VICTORY_KEYS=<seconds>:Enter,... as a KEY=VALUE gives other times.
#   --wine W          proton (default): Proton's own Wine, whose write watches use the
#                     kernel's userfaultfd and PAGEMAP_SCAN; system: Arch's Wine (not measured
#                     since docs/linux-wine.md: what made both slow was not the write watch)
#   --release         target/.../release instead of target/.../diag
# Environment: PROTON (Proton folder), WINEPREFIX, GPU_NAME (adapter by name or DXGI index),
# VICTORY_SCREEN (the monitor the title covers, by its output's name as `xrandr --listmonitors`
# gives it, e.g. HDMI-1; also as a KEY=VALUE; unset: the primary monitor).
# What was learned under Wine, and the tools for measured runs: docs/linux-wine.md.
# WINEDEBUG=err+all,fixme-all as a KEY=VALUE shows what Wine complains of (-all hides it).
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
PROTON="${PROTON:-$HOME/.steam/steam/steamapps/common/Proton - Experimental}"
GPU_NAME="${GPU_NAME:-}"
build=diag safe=0 rt=0 cmd=0 windowed=0 hud=1 runtime=proton start=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --safe) safe=1 ;;
        --rt) rt=1 ;;
        --windowed) windowed=1 ;;
        --no-hud) hud=0 ;;
        --start) start=1 ;;
        --wine) shift; runtime="${1:-}" ;;
        --release) build=release ;;
        --cmd) cmd=1; shift; break ;;
        --) shift; break ;;
        *) break ;;
    esac
    shift
done

case "$runtime" in
    proton)
        WINE="$PROTON/files/bin/wine" WINESERVER="$PROTON/files/bin/wineserver"
        [[ -x "$WINE" ]] || { echo "No Wine in $PROTON (set PROTON=... or --wine system)"; exit 1; }
        export WINEPREFIX="${WINEPREFIX:-$HOME/Games/victory-proton}" ;;
    system)
        WINE="$(command -v wine || true)" WINESERVER="$(command -v wineserver || true)"
        [[ -n "$WINE" ]] || { echo "No wine (pacman -S wine)."; exit 1; }
        export WINEPREFIX="${WINEPREFIX:-$HOME/Games/victory-wine}" ;;
    *) echo "--wine proton|system"; exit 1 ;;
esac
export WINESERVER

victory="$here/target/x86_64-pc-windows-msvc/$build/victory.exe"
[[ -f "$victory" ]] || { echo "No $victory. Build it first: cargo xwin build --locked --profile diag -p victory --features diagnostics --target x86_64-pc-windows-msvc"; exit 1; }

winpath() { local p; p="$(realpath -m "$1")"; printf 'Z:%s' "${p//\//\\}"; }

# Leftover VICTORY_* settings go, except where the files are (as play.ps1 does).
for v in $(compgen -e | grep '^VICTORY_' || true); do
    case "$v" in VICTORY_CONFIG|VICTORY_GAME_DIR|VICTORY_CACHE_DIR|VICTORY_DATA_DIR|VICTORY_SCREEN) ;; *) unset "$v" ;; esac
done

export VICTORY_TEXTURE_NO_WAIT=1 VICTORY_RELAY=2 VICTORY_WIDE_BACK=1 VICTORY_CARD_REHASH_RUNS=1 \
    VICTORY_JOIN_COPIES=1 VICTORY_INDIRECT_WINDOW=1 VICTORY_RELAY_COPY=1 VICTORY_CARD_STEPS=2 \
    VICTORY_INDIRECT_BATCH=2 VICTORY_MAP_THREADS=1 VICTORY_TITLE_PRIORITIES=0 VICTORY_CARD_AWAKE=1 \
    VICTORY_SHOW_AFTER=1 VICTORY_AMBIENT=0.8 VICTORY_D3D12_DRED=light
if [[ $safe == 1 ]]; then
    export VICTORY_UNDER_WIDE=0 VICTORY_LAND_ECHO=0
else
    export VICTORY_THIN=1 VICTORY_JOIN_PARTS=8 VICTORY_UPLOAD_AHEAD=1
fi
if [[ $rt == 1 ]]; then export VICTORY_BVH_BUILDER=1; fi
if [[ $windowed == 0 ]]; then export VICTORY_FULLSCREEN=1; fi   # borderless, the whole primary monitor
if [[ $hud == 1 ]]; then export MANGOHUD=1; fi
if [[ $start == 1 ]]; then
    keys=""; for t in $(seq 12 4 48); do keys+="${keys:+,}$t:Enter"; done
    export VICTORY_KEYS="$keys"
fi                   # MangoHud's Vulkan layer (vkd3d-proton)

# The prefix, with vkd3d-proton and DXVK's dxgi from Proton.
sys32="$WINEPREFIX/drive_c/windows/system32"
if [[ ! -f "$sys32/d3d12core.dll" ]] || grep -q 'Wine builtin DLL' "$sys32/d3d12core.dll"; then
    dlls="$PROTON/files/lib/wine"
    [[ -d "$dlls/vkd3d-proton" ]] || { echo "No vkd3d-proton in $PROTON (set PROTON=...)"; exit 1; }
    WINEDEBUG=-all "$WINE" wineboot -i >/dev/null 2>&1
    "$WINESERVER" -w
    cp "$dlls/vkd3d-proton/x86_64-windows/"{d3d12,d3d12core}.dll "$dlls/dxvk/x86_64-windows/dxgi.dll" "$sys32/"
fi
# Microsoft's D3DCompile: Wine's own (vkd3d-shader) cannot compile the runtime's internal
# shaders (InterlockedOr on byte buffers...), which leaves the picture black. Taken from
# Lutris' d3d_extras or D3DCOMPILER=<path of a 64-bit d3dcompiler_47.dll>.
if [[ ! -f "$sys32/d3dcompiler_47.dll" ]] || grep -q 'Wine builtin DLL' "$sys32/d3dcompiler_47.dll"; then
    src="${D3DCOMPILER:-$HOME/.local/share/lutris/runtime/d3d_extras/v2/x64/d3dcompiler_47.dll}"
    [[ -f "$src" ]] || { echo "No native 64-bit d3dcompiler_47.dll (set D3DCOMPILER=...)"; exit 1; }
    cp "$src" "$sys32/"
fi
export WINEDLLOVERRIDES="d3d12,d3d12core,dxgi,d3dcompiler_47=n;${WINEDLLOVERRIDES:-}"
export WINEDEBUG="${WINEDEBUG:--all}" DISABLE_LSFGVK="${DISABLE_LSFGVK:-1}"
# vkd3d-proton's pipeline cache, kept with the rest of the cache (otherwise the current folder).
export VKD3D_SHADER_CACHE_PATH="${VKD3D_SHADER_CACHE_PATH:-$HOME/Games/victory-cache/vkd3d}"
mkdir -p "$VKD3D_SHADER_CACHE_PATH"
# The runtime prefers the high-performance SM 6.6 adapter (the discrete card) by itself.
# GPU_NAME forces one by name or DXGI index; vkd3d/DXVK's own name filters are not used:
# with VKD3D_FILTER_DEVICE_NAME the device is not created (DXGI_ERROR_NOT_FOUND).
if [[ -n "$GPU_NAME" ]]; then export VICTORY_D3D12_ADAPTER="$GPU_NAME"; fi

# Started without the session's display (ssh, a tool's shell): the session's own. Proton's Wine
# draws through X11 and, with no display, reports a 1024x768 screen; the title then stops. The
# newest authority file: a session that was ended leaves its own behind.
if [[ -z "${DISPLAY:-}" && -S /tmp/.X11-unix/X0 ]]; then
    export DISPLAY=:0
    auth="$(ls -t "${XDG_RUNTIME_DIR:-/run/user/$UID}"/.mutter-Xwaylandauth.* 2>/dev/null | head -1 || true)"
    if [[ -n "$auth" ]]; then export XAUTHORITY="$auth"; fi
fi
if [[ -z "${WAYLAND_DISPLAY:-}" && -S "${XDG_RUNTIME_DIR:-/run/user/$UID}/wayland-0" ]]; then
    export WAYLAND_DISPLAY=wayland-0
fi

cd "$here"   # victory.toml is looked for here
if [[ $cmd == 1 ]]; then
    exec "$WINE" "$victory" "$@"
fi

[[ $# -ge 1 ]] || { echo "usage: $0 [options] <game exe> [KEY=VALUE ...] (see the top of this file)"; exit 1; }
exe="$1"; shift
[[ -f "$exe" ]] || { echo "No $exe."; exit 1; }
for kv in "$@"; do export "$kv"; done
# VICTORY_SCREEN: the title's borderless window over that monitor and not the primary one
# (VICTORY_FULLSCREEN_AT: its rectangle, counted from the primary monitor's corner as Wine does).
if [[ -n "${VICTORY_SCREEN:-}" && "${VICTORY_FULLSCREEN:-}" == 1 ]]; then
    at="$(xrandr --listmonitors 2>/dev/null | awk -v want="$VICTORY_SCREEN" '
        NR > 1 {
            split($3, g, /[x+]/); sub(/\/.*/, "", g[1]); sub(/\/.*/, "", g[2])
            if ($2 ~ /\*/) { px = g[3]; py = g[4] }
            if ($NF == want) { w = g[1]; h = g[2]; x = g[3]; y = g[4]; found = 1 }
        }
        END { if (found) printf "%d,%d,%d,%d", x - px, y - py, w, h }')"
    if [[ -n "$at" ]]; then
        export VICTORY_FULLSCREEN_AT="$at"
    else
        echo "VICTORY_SCREEN=$VICTORY_SCREEN: no such monitor (xrandr --listmonitors); the primary one is used." >&2
    fi
fi
exec "$WINE" "$victory" run "$(winpath "$exe")" --execute
