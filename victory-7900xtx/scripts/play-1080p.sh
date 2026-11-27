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
# The light is play.sh's (VICTORY_AMBIENT=0.8, an estimate from what is on screen);
# VICTORY_AMBIENT=constant:0.08 as a KEY=VALUE gives the same faint light everywhere instead,
# by which people away from lamps can be made out at night.
#   scripts/play-1080p.sh [play.sh options] [KEY=VALUE ...]
# A KEY=VALUE given here wins over these (VICTORY_PORT_CONSTANTS=0 ...).
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

exec "$here/play.sh" --start "${options[@]}" "$exe" \
    VICTORY_OUTPUT=1920x1080 \
    VICTORY_PORT_CONSTANTS=1 VICTORY_PORT_DESCRIPTORS=1 VICTORY_PORT_CBV=1 \
    VICTORY_FRAME_INTERVAL_MS=16.667 VICTORY_FRAME_INTERVAL_DRAWS=500 VICTORY_CARD_BUFFERS_MIN_KB=64 \
    VICTORY_JOIN_PARTS_DRAWS=500 "${settings[@]}"
