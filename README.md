# LeoTux

Running the Xbox Series build of GTA VI on Linux with [VIctory](https://github.com/Marvisak/VIctory)
(a compatibility layer that runs the console executable on a PC and translates its GPU work), on a stock
distro Wine — **no Proton build**. This repository is [GTA6-btw](https://github.com/JokiiiBoy/GTA6-btw)
with our own work on top; GTA6-btw started from [Leonida](https://github.com/ItsNotRexed/Leonida), whose
own runtime is still here (see [Leonida path](#leonida-path)). Upstream fixes come in from the GTA6-btw
clone, which keeps tracking its author; they are merged here by hand (see
[The trees](#the-trees) and docs/knowledge-base).

**No game files are in this repository.** You need your own copy of the game.

## Where it stands (2026-11-26)

Tested on one machine: Ryzen 7 5800x3d, 32 GB RAM, Radeon RX 7800 XT 16 GB, Arch Linux.

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
- **The first intro scene can stutter** on some settings while the title's movie competes for the CPU (the opening
  videos can also repeat three frames in a row: the submissions joining, `VICTORY_JOIN_PARTS=1` avoids it at ~3 ms).
- **Textures appear late** while the title streams; 12 GB of video memory fills up.
- **Rare title crash**: a null-vtable read in the title's own streaming code (a spawn descriptor consumed before its
  object is constructed; the upstream "null pointer in own code" race). `VICTORY_SKIP_NULL_VTABLE` (in progress)
  survives it.

## Quick start

You need: Rust with the `x86_64-pc-windows-msvc` target and [`xwin`](https://github.com/Jake-Shadle/xwin) (see
`victory-7900xtx/.cargo/config.toml` — its paths are this machine's), `winetricks`, `zenity` (optional), and your
own dump of the game. **No Docker, no Proton build**: the stock `/usr/bin/wine` runs it.

### 1. Wine (nothing to build)

The distro's Wine 11.21 is the play-proven choice: correct skinned geometry, quick world. Two wines we tried and
rejected: proton-cachyos's build (its `services.exe` stack-overflows at prefix boot here) and GE-Proton11-14's
11.0-staging (breaks skinned geometry: faces render as eyes and teeth only). The remaining versioned pieces are
PE dlls copied into the prefix, not builds:

- `d3d12.dll`, `d3d12core.dll` — **GE-Proton11-14**'s `files/lib/wine/vkd3d-proton/{x86_64,i386}-windows`
  (vkd3d-proton past 3.2). An older vkd3d-proton (proton-cachyos's 3.1.0) crashes in
  `vkCreateGraphicsPipelines` during loading.
- `dxgi.dll` — DXVK (proton-cachyos's `files/lib/wine/dxvk` works).
- `d3dcompiler_47.dll` — **Microsoft's** (`winetricks d3dcompiler_47` in the prefix): Wine's builtin cannot compile
  two of VIctory's helper shaders, and without them the 3D world stays black. The launcher forces
  `d3dcompiler_47,d3d12,d3d12core,dxgi` native.

The custom-wineserver optimization the upstream Proton build carried (memory views through a tree, not a list —
the title maps thousands of 64 KiB views a second while streaming, and a linear list makes every map call walk it)
is a **6-second server-only rebuild**, not a Proton build: upstream commit `3217449b` ("server: Store per-process
memory views in an rbtree", Wine-Bug 60868) applied to the distro Wine's tag. Wine runs a server already started
for the prefix, so the launcher kills the stock one and pre-starts it with `-p` (Wine 11.21's loader checks the
install directory first and `$WINESERVER` only as a fallback — the env var alone does nothing). Worth several fps
and the scene-change dips; `RBTREE=0` skips it.

### 2. The prefix

Any 64-bit prefix the dlls above were copied into. The launcher's default is ours
(`$PROJ/victory-proton-umu/pfx`); Microsoft's d3dcompiler_47 must be in it (see above).

### 3. VIctory

```bash
cd victory-7900xtx
cargo build --profile diag -p victory --features diagnostics --bins --target x86_64-pc-windows-msvc
```

### 4. Play

```bash
sh ./run-victory-direct.sh           # our launcher: env, caches, rbtree server, warm, run
# long runs (the title eats RAM), under systemd with a budget:
systemd-run --user --collect -p MemoryMax=58G -p MemorySwapMax=40G \
  --working-directory=$PWD --unit=gta6 sh ./run-victory-direct.sh
```

`run-victory-direct.sh` mirrors `scripts/run_victory_proton.sh`'s env but runs a bare Wine binary: it pre-starts
the rbtree server, compiles the pipelines earlier runs used (`victory warm --used`), then starts the title.
`WINEDEBUG=err+seh` when triaging: the worst crash family is silent under `-all` (exit code 3, nothing logged).
The log goes to `$LOG_DIR/victory-trace.log` (default `GTA6-btw-logs/` beside the repo); the layer's crash reports
and the title's Breakpad dumps land in the prefix under `users/<user>/AppData/Local/victory/data/<titleid>/`
(crashes.log, temp/). Controllers need hidraw access: a udev `uaccess` rule for the Sony ids (see
docs/knowledge-base) — Wine enumerates hidraw controllers at startup.

## The trees

Each tree is a whole VIctory fork with this repository's Wine pieces added; `VICTORY_TREE` picks one. Where each comes
from, what was merged and how conflicts were resolved is in its `UPSTREAM.md`.

| Tree | What it is | State here |
|---|---|---|
| `victory-7900xtx` (default) | [avatarmanz's RDNA 3 fork](https://github.com/avatarmanz/VIctory-7900XTX) + [zeex64's Linux work](https://github.com/zeex64/VIctory-7900XTX) + ours | the fastest under Wine: 10-20 fps in the world |
| `victory`, `scarlight` | older Belobug and [Scarlight](https://github.com/pablogonz12/Scarlight) trees | kept for reference |

`victory-belobug` and `victory-dexnom` were dropped here (2026-11-27): both were far behind
`victory-7900xtx` (about 5 and 1-4 fps in the world against its 6-10, and the belobug merge's
renderer patches blacked the picture under Wine). Their history is in the git history and in
the GTA6-btw clone.

"Ours" is: the title's watchdog hooks (`VICTORY_NO_WATCHDOG`), the log in a file under Proton (`VICTORY_LOG_FILE`),
crash reports and stack dumps in that log, the 1.0.2.1 int3 assert stepped over (`VICTORY_SKIP_INT3`), copy-back into
write-watched pages that Wine refuses to write, the early pipeline list kept without a host pipeline library, and
[KLBF Lab](https://github.com/PastiiJay/klbf-lab)'s half-wave port for 64-lane AMD hosts (`VICTORY_PS_HALVES`);
plus, from LeoTux: the direct-Wine launcher (`run-victory-direct.sh`), the rbtree-server engagement, and
`VICTORY_SKIP_NULL_VTABLE` (in progress).

## Settings

The run script ([scripts/run_victory_proton.sh](scripts/run_victory_proton.sh)) sets everything below as defaults; a
variable set in the environment wins. Our `run-victory-direct.sh` takes the same variables, with `WINE` and
`RBTREE` instead of `PROTON`.

| Variable | Default | What it does |
|---|---|---|
| `VICTORY_TREE` | `victory-7900xtx` | which tree runs |
| `GTA6_DIR`, `GAME_EXE` | `~/Downloads/GTA6.Xbox.v1.0.2.1` | the game |
| `WINE` | `/usr/bin/wine` | the Wine binary (run-victory-direct.sh) |
| `RBTREE` | `1` | `0`: run the stock wineserver (run-victory-direct.sh) |
| `PROTON` | `victory-ntdll` | the compat tool (run_victory_proton.sh; needs the Proton build) |
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
| Shader translations, the pipeline book, the early list | `<prefix>/drive_c/users/<wine-user>/AppData/Local/<tree>/cache` |
| vkd3d-proton's compiled pipelines | `~/.cache/<tree>/vkd3d` (`VKD3D_SHADER_CACHE_PATH` overrides; ours lives beside the repo) |
| Mesa's RADV shaders | `~/.cache/mesa_shader_cache` (10 GB; `MESA_SHADER_CACHE_DIR` overrides) |

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
