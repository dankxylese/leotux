#!/usr/bin/env bash
# LeoTux warm-up: the title's shaders and the host pipelines it draws with, made
# before it starts, with a progress window.
#
#   ./warm-up.sh                     # translate the shaders (once per victory.exe),
#                                    # then make the pipelines earlier runs used (--used)
#   ./warm-up.sh --skip-translate    # only the pipelines
#   ./warm-up.sh --jobs 8 --limit 500 --again
#
# What prints what (the progress is parsed from this; nothing is invented):
#   victory shaders <archives>
#     "N shaders in the cache, T s" every 1000 shaders, and
#     "N shaders in the cache, M do not translate, T s" at the end.
#     No total is known before the scan, so this stage pulses and shows N and T.
#   victory warm --used
#     "K pipelines known, D made before (fp), B bad; N to make, J processes" once,
#     "C of N, T s" every 1000 pipelines, and "C made in T s (...)" at the end.
#     Percentages are C*100/N against that N.
# With no display (or ZENITY=0) the tool's own lines go to stdout and the warm
# stage adds a "[warm] NN% (C of N)" line per update; the translate stage has no
# percentage to add, so its counts are all it prints.
#
# The translation pass reads the whole shader archive (about a minute the first
# time, a few seconds once every shader is translated), so it is skipped while
# the archive and the translator are the ones the last full pass used
# (<host cache>/translate.stamp); --translate-always redoes it, --skip-translate
# leaves it out.
set -euo pipefail

PROJ=/home/manse/A.Projects/GTA6-Emulation
VICTORY_TREE="${VICTORY_TREE:-victory-7900xtx}"
VICTORY_EXE="${VICTORY_EXE:-$PROJ/LeoTux/$VICTORY_TREE/target/x86_64-pc-windows-msvc/diag/victory.exe}"
GAME_DIR="${GTA6_DIR:-${VICTORY_GAME_DIR:-$PROJ/GTA6.Xbox.v1.0.2.1-CYBERLEEK}}"
# The title's shader archive (colon-separated for more than one).
SHADER_ARCHIVES="${VICTORY_SHADER_ARCHIVES:-$GAME_DIR/shaders.rpf}"
WINE="${WINE:-/usr/bin/wine}"
ZENITY="${ZENITY:-/usr/bin/zenity}"
# Host-side caches, the same as run-victory-direct.sh's.
CACHE_DIR="${VICTORY_HOST_CACHE_DIR:-$PROJ/victory-cache/$VICTORY_TREE}"

defaults() { local kv name; for kv in "$@"; do name="${kv%%=*}"; [[ -n "${!name+x}" ]] || export "$kv"; done; }

export WINEPREFIX="${WINEPREFIX:-$PROJ/victory-proton-umu/pfx}"
# The same settings run-victory-direct.sh uses for the title, so the pipelines
# and translations made here are the ones it finds (an env value wins).
defaults WINEDEBUG=-all \
         RUST_MIN_STACK=67108864 \
         WINEDLLOVERRIDES="d3dcompiler_47=n;d3d12=n;d3d12core=n;dxgi=n" \
         "VICTORY_CACHE_DIR=C:\\users\\manse\\AppData\\Local\\$VICTORY_TREE\\cache" \
         "VKD3D_SHADER_CACHE_PATH=Z:${CACHE_DIR//\//\\}\\vkd3d" \
         MESA_SHADER_CACHE_DIR="$PROJ/mesa-cache" \
         MESA_SHADER_CACHE_MAX_SIZE=10G \
         MESA_VK_DEVICE_SELECT=1002:747e DRI_PRIME=1 \
         VKD3D_CONFIG=ignore_shared_fence \
         VICTORY_LOG=warn

log() { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die() { warn "warm-up: $*"; exit 1; }

# The path as Wine's Z: drive sees it (the prefix's drives are Wine's own).
wine_path() { printf 'Z:%s' "${1//\//\\}"; }

# The bar, when there is one: zenity reads plain numbers and "# text" lines off a
# FIFO this script keeps open on fd 9 (so it never sees an early end of file).
BAR=0
BAR_FIFO=
BAR_PID=

# <done> <total> -> 0..100
warmup_percent() {
  local done=${1:-0} total=${2:-0}
  (( total > 0 )) || { printf '0'; return 0; }
  (( done > total )) && done=$total
  printf '%s' $(( done * 100 / total ))
}

zenity_wanted() {
  [[ ${ZENITY:-1} != 0 && ${WARMUP_ZENITY:-1} != 0 ]] || return 1
  [[ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]] || return 1
  [[ -x "$ZENITY" ]] || return 1
}

bar_open() { # <title> <pulsate|percent>
  local title=$1 mode=pulsate
  [[ ${2:-} == percent ]] && mode=percent
  BAR_FIFO=$(mktemp -u "${TMPDIR:-/tmp}/leotux-warmup.XXXXXX") || { BAR_FIFO=; return 1; }
  mkfifo "$BAR_FIFO" || { BAR_FIFO=; return 1; }
  local -a flags=(--progress "--title=$title" --width=560 --auto-close --no-cancel)
  [[ $mode == pulsate ]] && flags+=(--pulsate)
  "$ZENITY" "${flags[@]}" < "$BAR_FIFO" &
  BAR_PID=$!
  # O_RDWR: opening the FIFO cannot block on a reader that never came.
  if ! exec 9<> "$BAR_FIFO"; then
    kill "$BAR_PID" 2>/dev/null || true
    BAR_PID=
    rm -f "$BAR_FIFO"
    BAR_FIFO=
    return 1
  fi
}

bar_set() { printf '%s\n' "$1" >&9 2>/dev/null || true; }
bar_text() { printf '# %s\n' "$*" >&9 2>/dev/null || true; }

bar_close() {
  exec 9>&- 2>/dev/null || true
  if [[ -n $BAR_PID ]]; then
    # It closes itself on 100 (--auto-close); anything still there after that
    # has had its say and is stopped rather than waited on.
    sleep 0.3
    kill "$BAR_PID" 2>/dev/null || true
    wait "$BAR_PID" 2>/dev/null || true
    BAR_PID=
  fi
  [[ -n $BAR_FIFO ]] && rm -f "$BAR_FIFO"
  BAR_FIFO=
}

# One progress line from the tool: a bar update when there is a bar, else the
# percentage on stdout where one can be computed.
progress_update() { # <translate|warm> <line>
  local kind=$1 line=$2
  case $kind in
    translate)
      if [[ $line =~ ^([0-9]+)\ shaders\ in\ the\ cache,\ ([0-9]+)\ do\ not\ translate,\ ([0-9]+)\ s$ ]]; then
        # The final summary: the pass is over.
        if [[ $BAR == 1 ]]; then
          bar_text "translated ${BASH_REMATCH[1]} shaders (${BASH_REMATCH[2]} do not translate), ${BASH_REMATCH[3]} s"
          bar_set 100
        fi
      elif [[ $line =~ ^([0-9]+)\ shaders\ in\ the\ cache,\ ([0-9]+)\ s$ ]]; then
        [[ $BAR == 1 ]] && bar_text "${BASH_REMATCH[1]} shaders translated, ${BASH_REMATCH[2]} s so far"
      fi
      ;;
    warm)
      if [[ $line =~ ([0-9]+)\ to\ make,\ ([0-9]+)\ processes$ ]]; then
        # The total, said once before the first 1000.
        local to=${BASH_REMATCH[1]} procs=${BASH_REMATCH[2]} pct
        if [[ $BAR == 1 ]]; then
          bar_text "$to pipelines to make, $procs processes"
          (( to == 0 )) && bar_set 100
        else
          [[ $to == 0 ]] && printf '[warm] 100%% (0 to make)\n'
        fi
      elif [[ $line =~ ^([0-9]+)\ of\ ([0-9]+),\ ([0-9]+)\ s$ ]]; then
        local done=${BASH_REMATCH[1]} total=${BASH_REMATCH[2]} secs=${BASH_REMATCH[3]}
        pct=$(warmup_percent "$done" "$total")
        if [[ $BAR == 1 ]]; then
          bar_text "$done of $total pipelines, $secs s"
          bar_set "$pct"
        else
          printf '[warm] %s%% (%s of %s, %s s)\n' "$pct" "$done" "$total" "$secs"
        fi
      elif [[ $line =~ ^([0-9]+)\ made\ in\ ([0-9]+)\ s ]]; then
        if [[ $BAR == 1 ]]; then
          bar_text "${BASH_REMATCH[1]} pipelines made in ${BASH_REMATCH[2]} s"
          bar_set 100
        fi
      fi
      ;;
  esac
  return 0
}

# Reads the tool's output, echoes it, and derives the bar from it.
progress_filter() { # <kind>
  local kind=$1 line
  while IFS= read -r line; do
    line=${line%$'\r'}
    printf '%s\n' "$line" || true
    progress_update "$kind" "$line"
  done
}

# run_progress <window title> <translate|warm> <command...>
run_progress() {
  local title=$1 kind=$2 mode=pulsate
  shift 2
  [[ $kind == warm ]] && mode=percent
  if [[ $BAR == 1 ]] && ! bar_open "$title" "$mode"; then
    BAR=0
    warn "warm-up: no progress window; the tool's own lines follow"
  fi
  local -a ends=()
  set +e
  "$@" 2>&1 | progress_filter "$kind"
  ends=("${PIPESTATUS[@]}")
  set -e
  if [[ $BAR == 1 ]]; then bar_close; fi
  return "${ends[0]:-1}"
}

# What the last full translation pass read: the translator and the archives.
build_stamp() {
  local p
  for p in "$VICTORY_EXE" $SHADER_ARCHIVES; do
    stat -c '%n %s %Y' "$p" 2>/dev/null || true
  done
}

STAMP_FILE="$CACHE_DIR/translate.stamp"

translate_stage() {
  local -a args=("$VICTORY_EXE" shaders)
  local p
  local -a wanted=()
  while IFS= read -r p; do
    [[ -n $p ]] || continue
    if [[ ! -e $p ]]; then
      warn "translate: no shader archive at $p; leaving it out"
      continue
    fi
    wanted+=("$(wine_path "$p")")
  done <<< "${SHADER_ARCHIVES//:/$'\n'}"
  if (( ${#wanted[@]} == 0 )); then
    warn "translate: nothing to read (set VICTORY_SHADER_ARCHIVES); skipping"
    return 0
  fi
  args+=("${wanted[@]}")
  (( ${#JOBS[@]} )) && args+=("${JOBS[@]}")
  log "translate: ${#wanted[@]} archive(s) -> $VICTORY_CACHE_DIR"
  run_progress "LeoTux: translating shaders" translate "$WINE" "${args[@]}"
}

warm_stage() {
  local -a args=("$VICTORY_EXE" warm "${WARM_FLAGS[@]}")
  (( ${#JOBS[@]} )) && args+=("${JOBS[@]}")
  log "warm: ${WARM_FLAGS[*]:-no flags}${JOBS[*]:+ ${JOBS[*]}}"
  run_progress "LeoTux: making pipelines" warm "$WINE" "${args[@]}"
}

usage() {
  cat <<'EOF'
usage: warm-up.sh [--skip-translate] [--translate-always] [--jobs N]
                  [--used|--again] [--limit N] [--plain] [-h]

  (no flags)           translate the shaders (once per victory.exe), then make
                       the pipelines earlier runs drew with (--used)
  --skip-translate     only the pipelines
  --translate-always   translate again even if this victory.exe and archive
                       were already done (cache/translate.stamp)
  --jobs N             worker processes / threads for both stages
  --used               only the pipelines earlier runs drew with (default)
  --again              make the pipelines made before too (all of them unless
                       --used is given as well)
  --limit N            at most N pipelines
  --plain              no window: progress on stdout
EOF
}

SKIP_TRANSLATE=0
TRANSLATE_ALWAYS=0
USED=0
AGAIN=0
JOBS=()
WARM_FLAGS=()

main() {
  while (( $# )); do
    case $1 in
      --skip-translate) SKIP_TRANSLATE=1 ;;
      --translate-always|--force-translate) TRANSLATE_ALWAYS=1 ;;
      --used) USED=1 ;;
      --again) AGAIN=1 ;;
      --jobs) JOBS=(--jobs "${2:?--jobs needs a number}"); shift ;;
      --jobs=*) JOBS=(--jobs "${1#*=}") ;;
      --limit) WARM_FLAGS+=(--limit "${2:?--limit needs a number}"); shift ;;
      --limit=*) WARM_FLAGS+=(--limit "${1#*=}") ;;
      --plain|--no-zenity) ZENITY=0 ;;
      -h|--help) usage; return 0 ;;
      *) warn "warm-up: unknown argument $1 (see --help)"; return 2 ;;
    esac
    shift
  done

  # --used is the default: the pipelines earlier runs drew with. --again alone
  # means every pipeline the book knows; say --used as well to keep it to those.
  if (( USED )) || (( ! AGAIN )); then
    WARM_FLAGS=(--used "${WARM_FLAGS[@]}")
  fi

  [[ -x $WINE ]] || die "no wine at $WINE"
  [[ -f $VICTORY_EXE ]] || die "no victory.exe at $VICTORY_EXE (build it first)"
  mkdir -p "$CACHE_DIR" "$CACHE_DIR/vkd3d" "$PROJ/mesa-cache"

  zenity_wanted && BAR=1
  log "warm-up:      $VICTORY_TREE"
  log "progress:     $([[ $BAR == 1 ]] && echo 'zenity window' || echo 'stdout (no display)')"

  if (( SKIP_TRANSLATE )); then
    log "translate:    skipped (--skip-translate)"
  elif (( ! TRANSLATE_ALWAYS )) && [[ -f $STAMP_FILE ]] \
       && [[ "$(build_stamp)" == "$(cat "$STAMP_FILE")" ]]; then
    log "translate:    skipped (this victory.exe and archive were done before; --translate-always to redo)"
  else
    if translate_stage; then
      build_stamp > "$STAMP_FILE"
    else
      warn "translate: failed; carrying on with the pipelines (the run translates what it needs)"
    fi
  fi

  if ! warm_stage; then
    warn "warm: failed; pipelines the cache lacks are made while the title runs"
    return 1
  fi
  log "warm-up:      done"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  main "$@"
fi
