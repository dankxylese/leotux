# LeoTux

Running the Xbox Series build of GTA VI on Linux with [VIctory](https://github.com/Marvisak/VIctory)
(a compatibility layer that runs the console executable on a PC and translates its GPU work), on a stock
distro Wine — **no Proton build**.

**No game files are in this repository.** You need your own copy of the game.

## Where it stands (2026-11-28)

Tested on one machine: Ryzen 7 5800X3D, 32 GB RAM, Radeon RX 7800 XT 16 GB (RADV, Mesa 26.2.6),
EndeavourOS (Arch) with the CachyOS kernel 7.3.3. The reference scene is the prologue (it renders
real world content and is 1-to-1 comparable between runs).

| | |
|---|---|
| Boot, menus, intro video | 30 fps, the title's own cap (no rubber-banding: joins are world-only) |
| From the start to the world | about 20-30 s |
| Prologue, in-world | 14-24 fps - consistent 14-18 with high peaks in lighter scenes, 67-69 ms a frame median (p90 ~79), frame-time stdev ~11 |
| Faces and mouths | **working** — skin on every run, lips follow the dialogue |
| Shaders and pipelines | pre-computed before the title starts (`warm-up.sh`: every shader translated once, every pipeline earlier runs drew with made ahead, with a progress window — the title does not stall on them) |

The tree is avatarmanz/main + zeex64 + PR [#440](https://github.com/avatarmanz/VIctory-7900XTX/pull/440)
(Linux under Wine) + its advance of the same day (the mouths fix), with ours on top; see
`victory-7900xtx/UPSTREAM.md`. What made faces and mouths move: a face-deform compute pass is never dropped
while its pipeline builds (`VICTORY_COMPUTE_WAIT_MS`, and the pipeline library is kept between runs), and the
face network's 16-bit buffer accesses are lowered as integers through half views with the raw typed loads —
they used to be read as floats through integer views, which computes zeros: a face at rest.

Known to be wrong (only what our own runs hit):

- **The frame is the layer's memory overlay, not the GPU**: ~21 ms uploading pages the title wrote and ~20 ms
  reading back what shaders wrote, of a ~68 ms frame, with the GPU idle for most of it. The read-back was ~49 ms
  until shaders' writes into write-combined title memory were left on the card (`VICTORY_WC_NO_BACK`, on by
  default). What is left: the uploads move ~90 MB a frame because one ~480-byte constant-buffer fill dirties a
  whole 64 KiB copy unit; ~22 s a run of one-fence-wait-per-allocation page-ins of new card memory (the
  world-load frame's few-second dip; a batched wait is being written); and refreshes that re-inspect
  ~21 GB a frame to find those writes — epoch summaries now answer ~4 GB a frame of that in O(1)
  (`VICTORY_REFRESH_EPOCH`), with the rest being dug into (docs/knowledge-base/TODO.md). 30 fps is the target.
- **Rare title crash**: null dereferences in the title's own streaming code (a spawn descriptor consumed before its
  object is constructed; the upstream "null pointer in own code" race, ~1 in 30 runs). Three survival guards
  (each on unless `0`): `VICTORY_SKIP_NULL_VTABLE` for the vtable-call shape (the load of the register may sit up to
  16 bytes behind the call, across argument setup only), `VICTORY_SKIP_NULL_CHAIN` for null-base (or all-ones-sentinel)
  direct-offset loads (`mov`, and `movzx`/`movsx` of a word or a byte), and `VICTORY_SKIP_SMALL_WRITE` for a *store*
  through a small non-null pointer (a size where a pointer belongs) — the fault is skipped with a zero result, decoded
  fail-closed (only the exact instruction shape, only a null, sentinel or small base, only a near-null fault address,
  only in a title image), counted (`NULL_VTABLE_SKIPS` / `NULL_CHAIN_SKIPS` / `WRITE_SKIPS`, printed at exit),
  reported, and a per-site loop breaker keeps a limping title from spinning in place. `VICTORY_WATCH_RANGE=<base:end>`
  (hex, one 64 KiB block) keeps one block read-only for the whole run and journals every write fault on it (µs,
  thread, address, qword before and after): the instrument that names the title's writer and its lag.
- **`--console series-s` does not boot here** (stalls before boot compute; the Series-S memory profile it reports
  wedges the title's pool setup). Series X at 4K output is the tested profile; on a 1080p monitor the compositor
  downscale looks native. `VICTORY_OUTPUT` is not used (it upscales from a low internal resolution).

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
The log goes to `$LOG_DIR/victory-trace.log` (default `LeoTux-logs/` beside the repo); the layer's crash reports
and the title's Breakpad dumps land in the prefix under `users/<user>/AppData/Local/victory/data/<titleid>/`
(crashes.log, temp/). Controllers need hidraw access: a udev `uaccess` rule for the Sony ids (see
docs/knowledge-base) — Wine enumerates hidraw controllers at startup.

## The trees

Each tree is a whole VIctory fork with this repository's Wine pieces added; `VICTORY_TREE` picks one. Where each comes
from, what was merged and how conflicts were resolved is in its `UPSTREAM.md`.

| Tree | What it is | State here |
|---|---|---|
| `victory-7900xtx` (default) | [avatarmanz's RDNA 3 fork](https://github.com/avatarmanz/VIctory-7900XTX) + [zeex64's Linux work](https://github.com/zeex64/VIctory-7900XTX) + PR [#440](https://github.com/avatarmanz/VIctory-7900XTX/pull/440) and its mouths-fix advance + [Belobug#477](https://github.com/Belobug/VIctory-Belobug-AMD/pull/477) (`VICTORY_WC_NO_BACK`) + ours (a staged read-back relay, the port-1 home decoupled, refresh epochs and disjoint runs, guards-v2, the null-vtable and null-chain guards) | 14-15 fps in the prologue with faces and mouths working |
| `victory`, `scarlight` | older Belobug and [Scarlight](https://github.com/pablogonz12/Scarlight) trees | kept for reference |

`victory-belobug` and `victory-dexnom` were dropped here (2026-11-27): both were far behind
`victory-7900xtx` (about 5 and 1-4 fps in the world against its 6-10, and the belobug merge's
renderer patches blacked the picture under Wine). Their history is in the git history and in
the GTA6-btw clone.

"Ours" is: the title's watchdog hooks (`VICTORY_NO_WATCHDOG`), the log in a file under Proton (`VICTORY_LOG_FILE`),
crash reports and stack dumps in that log, the 1.0.2.1 int3 assert stepped over (`VICTORY_SKIP_INT3`), copy-back into
write-watched pages that Wine refuses to write, the early pipeline list kept without a host pipeline library, and
[KLBF Lab](https://github.com/PastiiJay/klbf-lab)'s half-wave port for 64-lane AMD hosts (`VICTORY_PS_HALVES`);
plus, from LeoTux: the direct-Wine launcher (`run-victory-direct.sh`) with the `warm-up.sh` progress window,
the rbtree-server engagement, the `VICTORY_SKIP_NULL_VTABLE`/`VICTORY_SKIP_NULL_CHAIN`/`VICTORY_SKIP_SMALL_WRITE`
survival guards with the `VICTORY_WATCH_RANGE` write journal, the
port-1 home decoupled from the mmap mode (`VICTORY_PORT_HOME=1` under `VICTORY_WINE_MAP=0`), the refresh
region/piece/block epochs with their verifier and the disjoint refresh runs (`VICTORY_REFRESH_EPOCH`,
`VICTORY_REFRESH_RUNS`), and guards-v2 (protection coalescing, queries outside the lock).

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
| `VICTORY_CONSOLE` | `series-x` | `series-s` is broken here (see "Where it stands") |
| `VICTORY_WINE_MAP` | `0` | the mmap title-memory mode: poison with our write-tracking (gigabytes read as changed per frame); the rbtree server covers its benefit |
| `VICTORY_PORT_HOME` | `1` | renderer port 1's home under `VICTORY_WINE_MAP=0`: the title's 32-bit graphics heap used in place (prologue A/B: -7 ms median, p90 -18) |
| `VICTORY_TEXTURE_MAX` | `0` | the merged tree caps streamed textures at 1024 px by default (muddy textures); `0` keeps full mips on the 16 GB card |
| `VICTORY_WC_NO_BACK` | `1` | shaders' writes into write-combined title memory stay on the card (prologue A/B: read-back 49 -> 20 ms, median 74.7 -> 53.6); `0` restores fetching |
| `VICTORY_CARD_BUFFERS_MIN_KB` | `1024` | buffers under 1 MiB do not get a VRAM copy (prologue A/B: p90 95.4 -> 78.8 ms, the world-load dip ~2 s shorter, no VRAM pressure); `64` restores the old floor |
| `VICTORY_REFRESH_EPOCH` | `1` | epoch summaries answer provably-clean refresh spans in O(1) instead of walking them (verified 0 disagreements against full walks; `0` walks); pairs with `VICTORY_EPOCH_VERIFY=1` to watch agreement live |
| `VICTORY_REFRESH_RUNS` | `1` | refreshes track up to 8 disjoint runs per copy instead of one min-max envelope (stdev 26.9 -> 10.6 with the verifier off; `0` restores the single envelope) |
| `VICTORY_THIN_THREADS` | `14` | thin worker threads (neutral A/B in the prologue; kept for denser scenes) |
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
