# GTA6-btw

Running the Xbox Series build of GTA VI on Linux, through Proton, with [VIctory](https://github.com/Marvisak/VIctory)
(a compatibility layer that runs the console executable on a PC and translates its GPU work). The repository started
from [Leonida](https://github.com/ItsNotRexed/Leonida), whose own runtime is still here (see [Leonida path](#leonida-path)).

**No game files are in this repository.** You need your own copy of the game.

## Where it stands (2026-11-26)

Tested on one machine: Ryzen 7 5800x3d, 32 GB RAM, Radeon RX 7800 XT 12 GB, Arch Linux.

| | |
|---|---|
| Boot, menus, intro video | 30 fps, the title's own cap |
| From the start to the world | about 20-30s |
| Open world, first mission | 10-20 fps (`victory-7900xtx`) |
| Shaders | every shader the title used translates |

Known to be wrong:

- **Mouths do not move** while characters speak (the voice plays). Not a rendering drop: still so with every GPU pass
  running. Never worked in any tree here.
- **Some scenes are too dark, and the phone's screen stays blank**: both need the console's ray-tracing structures,
  which are off (`VICTORY_BVH_BUILDER=1` turns them on, at about half the frame rate).
- **The first intro scene can stutter** on some settings while the title's movie competes for the CPU.
- **Textures appear late** while the title streams; 12 GB of video memory fills up.

## Quick start

You need: Docker (for the Proton build), Rust with the `x86_64-pc-windows-msvc` target and
[`xwin`](https://github.com/Jake-Shadle/xwin) (see `victory-7900xtx/.cargo/config.toml`), `winetricks`, `zenity`
(optional, for the "Compiling shaders" window), and your own dump of the game.

### 1. Proton

Proton is built from `../Proton` (Valve's Proton with changes to Wine's server and ntdll: memory views found through a
tree instead of a list, which makes the title's 1,000-14,000 mappings a second cheap: loading is much faster) into
`../proton-btw`, and installed as the compat tool `victory-ntdll`:

```bash
cd ../proton-btw
make -j16            # in Valve's Steam Runtime container; -march=znver4 -mno-avx is set in its Makefile
make install         # -> ~/.local/share/Steam/compatibilitytools.d/victory-ntdll
```

The run script uses `victory-ntdll` when it is installed, else `Proton-CachyOS-Victory`; `PROTON=<tool>/proton`
picks another.

### 2. The prefix

The prefix is `~/.local/share/victory-proton` (`STEAM_COMPAT_DATA_PATH`). It needs Microsoft's `d3dcompiler_47.dll`:
Wine's cannot compile two of VIctory's helper shaders, and without them the 3D world stays black.

```bash
WINEPREFIX=~/.local/share/victory-proton/pfx winetricks d3dcompiler_47
```

### 3. VIctory

```bash
cd victory-7900xtx
cargo build --profile diag -p victory --features diagnostics --bins --target x86_64-pc-windows-msvc
```

### 4. Play

```bash
GTA6_DIR=/path/to/GTA6.Xbox.v1.0.2.1 ./scripts/run_victory_proton.sh
```

Before the title starts, the pipelines earlier runs used are compiled while a "Compiling shaders" window is up (a few
seconds with warm caches, minutes after a new build or driver). The log goes to `/tmp/opencode/victory-trace.log`.

## The trees

Each tree is a whole VIctory fork with this repository's Wine pieces added; `VICTORY_TREE` picks one. Where each comes
from, what was merged and how conflicts were resolved is in its `UPSTREAM.md`.

| Tree | What it is | State here |
|---|---|---|
| `victory-7900xtx` (default) | [avatarmanz's RDNA 3 fork](https://github.com/avatarmanz/VIctory-7900XTX) + [zeex64's Linux work](https://github.com/zeex64/VIctory-7900XTX) + ours | the fastest under Wine: 6-10 fps in the world |
| `victory-belobug` | the above merged with [Belobug's AMD fork](https://github.com/Belobug/VIctory-Belobug-AMD) | experimental: about 5 fps, its renderer patches black the picture under Wine |
| `victory-dexnom` | [DexNom Runtime](https://github.com/DexNom/VIctory) + ours | the previous default: 1-4 fps |
| `victory`, `scarlight` | older Belobug and [Scarlight](https://github.com/pablogonz12/Scarlight) trees | kept for reference |

"Ours" is: the title's watchdog hooks (`VICTORY_NO_WATCHDOG`), the log in a file under Proton (`VICTORY_LOG_FILE`),
crash reports and stack dumps in that log, the 1.0.2.1 int3 assert stepped over (`VICTORY_SKIP_INT3`), copy-back into
write-watched pages that Wine refuses to write, the early pipeline list kept without a host pipeline library, and
[KLBF Lab](https://github.com/PastiiJay/klbf-lab)'s half-wave port for 64-lane AMD hosts (`VICTORY_PS_HALVES`).

## Settings

The run script ([scripts/run_victory_proton.sh](scripts/run_victory_proton.sh)) sets everything below as defaults; a
variable set in the environment wins.

| Variable | Default | What it does |
|---|---|---|
| `VICTORY_TREE` | `victory-7900xtx` | which tree runs |
| `GTA6_DIR`, `GAME_EXE` | `~/Downloads/GTA6.Xbox.v1.0.2.1` | the game |
| `PROTON` | `victory-ntdll` | the compat tool |
| `VICTORY_CONSOLE` | `series-x` | `series-s`: 1080p, less video memory, about 5 ms faster, a softer picture |
| `VICTORY_WARM_FIRST` | `1` | `0`: start the title without compiling shaders first |
| `VICTORY_FORK_SETTINGS` | `1` | `0`: none of the tree's speed switches (to tell whether one of them is at fault) |
| `VICTORY_AMBIENT` | `0` | the title's own ambient light; `0.8`: the forks' stand-in for the missing ray-traced light (flatter skin) |
| `VICTORY_BVH_BUILDER` | unset | `1`: the ray-tracing structures (lit interiors, the phone's screen), much slower |
| `RUST_MIN_STACK` | 64 MiB | needed: without it the largest compute pipelines never finish under vkd3d-proton |
| `VICTORY_PIPELINE_WAIT_MS` | `100` | how long a frame waits for a pipeline still compiling before it draws without |

`./scripts/run_victory_proton.sh warm` only compiles shaders ahead (`victory warm --used`).

### Caches

| What | Where |
|---|---|
| Shader translations, the pipeline book, the early list | `<prefix>/drive_c/users/steamuser/AppData/Local/<tree>/cache` |
| vkd3d-proton's compiled pipelines | `~/.cache/<tree>/vkd3d` |
| Mesa's RADV shaders | `~/.cache/mesa_shader_cache` (10 GB) |

Saves are shared between trees. A tree's first run translates every shader; the run after a new build compiles every
pipeline again (the "Compiling shaders" window then takes minutes).

## Tools for testing

Diagnostics builds only (`--features diagnostics`), in `victory-7900xtx`:

| Variable | What it does |
|---|---|
| `VICTORY_BUILDER_RUN=<hash>,...` | compute programs taken for ray-tracing builder stages that run all the same (each one taken is logged) |
| `VICTORY_PIPELINE_DUMP=<dir>` | dumps what the title passes to device methods 0x1c8/0x218 (title data: keep it out of the repository) |
| `VICTORY_SHADER_SAVE=<dir>` | each shader as the title gave it and as translated (title data, as above) |
| the "present times" log line | every 5 s: median, 95th and 99th percentile and longest time between presents |

What was tried before, tree by tree, with the measurements: [docs/linux-progress.md](docs/linux-progress.md).

## Leonida path

Leonida's own runtime (`runtime/`, `gta6rt.exe`, a PE loader with shims) also runs under Wine, but stops at the title's
configuration load (`platform:/data/gameconfig.zmt` not found):

```bash
./setup_leonida_wine.sh check && ./setup_leonida_wine.sh install-xwin && ./setup_leonida_wine.sh build-runtime
GTA6_DIR=/path/to/GTA6.Xbox.v1.0.2.1 LEONIDA_SKIP_INT3=1 ./setup_leonida_wine.sh run
python tools/boot_trace.py --latest ~/leonida-wine-logs
```

## Rules

See [AGENTS.md](AGENTS.md) and [CONTRIBUTING.md](CONTRIBUTING.md): no game data in the repository (logs and dumps
quote it too), no keys or protection workarounds, nothing online. Traces and logs can contain details of the title;
look before sharing.

## Legal

For interoperability/research purposes only. Not affiliated with or endorsed by Rockstar Games, Take-Two, or Microsoft.
