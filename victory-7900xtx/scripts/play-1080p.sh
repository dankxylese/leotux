#!/bin/bash
# The title under Wine, straight into the game: Enter pressed past the title screen (--start:
# from second 12, the world half a minute after the launch),
# the title rendering at 1920x1080, and the settings measured fastest on an RX 7900 GRE with a
# picture that is there (docs/linux-wine.md, "Above the title's 30 frames a second": 27 to 28 ms
# a frame, 36 frames a second, in the first scene, where play.sh alone has 40 ms and 25):
#   VICTORY_OUTPUT=1920x1080          the title renders to that size itself
#   VICTORY_PORT_CONSTANTS=1          renderer patch 1: the title's constants heap read by the
#   VICTORY_PORT_DESCRIPTORS=1        card where the title writes it; patches 2 and 5 (6 and 7
#   VICTORY_PORT_CBV=1                gain nothing under Wine)
#   VICTORY_FRAME_INTERVAL_MS=16.667  the title let above its 30 frames a second
#   VICTORY_FRAME_INTERVAL_DRAWS=500  ... in the world only: the opening animation and videos tick
#                                     at the title's own 30 (at 60, a frame in fourteen came twice)
#   VICTORY_CARD_BUFFERS_MIN_KB=64    every title buffer's copy in the card's memory (4 MiB and
#                                     more by default on a 16 GB card): 13.2 GiB of it in use
#   VICTORY_JOIN_PARTS_DRAWS=500      a frame's submissions joined (play.sh's VICTORY_JOIN_PARTS=8)
#                                     only after a frame of 500 draws or more: in the world,
#                                     where it gains 3 ms, and not in the intro videos, whose
#                                     frames come three new and then the same three again joined
#   VICTORY_WC_NO_BACK=1              what shaders write into title memory asked for as write-
#                                     combined stays on the card (4 ms a frame less on this
#                                     branch, runs 130 and 131, both keeping frames)
#   VICTORY_CARD_TOUCHED=1            a submission asks only the copies in the card's memory
#                                     that changed, not all 3,450 of them: 1.3 ms less of the
#                                     submitting thread's work a frame (run 124)
#   VICTORY_WINDOWS_BAR_MB=4096       windows outside buffers in the card's memory, written by the
#                                     CPU through the bus (no copy to send, no list of changes):
#                                     28.2 ms a frame against 31.6, the submitting thread 16.9
#                                     against 21.5 (runs 237-240, the prison; after zocram4cc)
#   VICTORY_UI_PLANE_720=1            the interface on a plane of its own at 3840x2160 over the
#                                     scene at 1920x1080 (the fork's pull request 28): at the
#                                     title's single plane it kept its 4K pixel size, hints and
#                                     subtitles twice too large. Frame the same (runs 216-219).
#   FSR 4 `native`                    when AMD's signed amd_fidelityfx_upscaler_dx12.dll is found
#                                     (FIDELITYFX_DIR, default: Forza Horizon 6's Steam folder):
#                                     the title's picture made at 1920x1080 and run through PC FSR
#                                     4.1.1 as anti-aliasing, in place of the title's own upscale
#                                     from 1280x720. Sharp where the title's is soft, for 0 to 2 ms
#                                     a frame here (runs 137 to 145): the card has room to spare,
#                                     the submitting thread sets the pace. FIDELITYFX_DIR=none
#                                     keeps the title's upscaler.
# The light is play.sh's (VICTORY_AMBIENT=0.8, an estimate from what is on screen);
# VICTORY_AMBIENT=constant:0.08 as a KEY=VALUE gives the same faint light everywhere instead,
# by which people away from lamps can be made out at night.
#   scripts/play-1080p.sh [play.sh options] [KEY=VALUE ...]
# A KEY=VALUE given here wins over these (VICTORY_PORT_CONSTANTS=0, VICTORY_WC_NO_BACK=0 ...).
# VICTORY_PORT_CBV=1 has shaders of its own in the cache: the first launch with it translates
# them as it goes and stutters, unless they were made ahead (warm-set.sh VICTORY_PORT_CBV=1,
# two minutes: docs/linux-wine.md, Tools).
# GAME_EXE: the title's executable (default ~/Games/GTA6/game_scarlett_master.exe).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
exe="${GAME_EXE:-$HOME/Games/GTA6/game_scarlett_master.exe}"

# play.sh takes its options before the executable and the settings after it.
options=() settings=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --wine) options+=("$1" "${2:-}"); shift ;;
        --*) options+=("$1") ;;
        *) settings+=("$1") ;;
    esac
    shift
done

fsr=()
fx="${FIDELITYFX_DIR:-$HOME/.steam/steam/steamapps/common/ForzaHorizon6}"
if [[ "$fx" != none && -f "$fx/amd_fidelityfx_upscaler_dx12.dll" ]]; then
    p="$(realpath -m "$fx")"
    fsr=(VICTORY_FIDELITYFX_DIR="Z:${p//\//\\}" VICTORY_FSR_VERSION=4.1.1 VICTORY_FSR_4K=native
         DXIL_SPIRV_CONFIG=wmma_rdna3_workaround)
    echo "FSR 4 native from $fx (FIDELITYFX_DIR=none: the title's upscaler)"
fi

exec "$here/play.sh" --start "${options[@]}" "$exe" \
    VICTORY_OUTPUT=1920x1080 \
    VICTORY_PORT_CONSTANTS=1 VICTORY_PORT_DESCRIPTORS=1 VICTORY_PORT_CBV=1 \
    VICTORY_FRAME_INTERVAL_MS=16.667 VICTORY_FRAME_INTERVAL_DRAWS=500 VICTORY_CARD_BUFFERS_MIN_KB=64 \
    VICTORY_JOIN_PARTS_DRAWS=500 VICTORY_WC_NO_BACK=1 VICTORY_CARD_TOUCHED=1 VICTORY_UI_PLANE_720=1 VICTORY_WINDOWS_BAR_MB=4096 \
    "${fsr[@]}" "${settings[@]}"
