# Session log — 2026-11-25 — Display path, the loading screen, and load speed

Branch: `feat/arena-single-address-space` (worktree `worktrees/a9f-descriptor-bridge`, shared object store with
Leonida-MainEffort). Host: CachyOS, RX 7800 XT (RADV), wine 11.21, KDE Wayland + XWayland. Game pkg 1.0.2.1.

This era ran from "black frame" to "the game's frame is on screen and it is genuinely loading, just slowly."
Everything below is committed on the branch; the run env and probe flags are at the end.

## TL;DR

- The frame **is** rendered and **is** on screen (in the layer's own helper window). The whole "presented black"
  saga was measurement errors, not a rendering fault.
- The game now **boots stably to the loading screen** (128+ presents, zero crash) and is **genuinely loading** —
  reading assets and CPU-decompressing them — not stuck, deadlocked, or waiting on a service.
- The remaining blocker is **load speed**, and it is **root-caused**: the DirectStorage queue is single-worker and
  strictly in-order, so read/inflate/copy never overlap (≈11 MB/s at 32% CPU). The inflater itself benchmarks at
  183 MiB/s. The ordered-drain worker pool is implemented and committed.

## What was done (by theme, with commit refs)

### The frame is rendered and the crash is fixed
- Mid-frame readback proved the composite's render target holds real content; every earlier "source readback = 0%"
  was sampling the **empty swapchain buffer** the title hands to Present, not the render target. (A9bc)
- **Present-8 crash root-caused and fixed** `eef3c04b9`: a varargs mismatch in `ensure_title_range`'s refuse-path
  `snprintf` (4 format conversions, 3 args) — the 4th `%s` read a garbage pointer (`0x2`) and faulted in ucrtbase.
  Found by reading the code after the SIGSEGV-handler approach failed (Wine's VEH owns the fault). Result: 128+
  presents, zero crash, window title "Americas".

### The display path works end-to-end
- The "white window" was the present going to a **hidden helper window** the layer creates and never shows.
  `fcea5068d` (A9bp) shows the layer's own `LeonidaPresentX` window and presents into it: PROBE_COLOR = solid green
  on screen; `LEONIDA_PRESENT_RT=1` = the game's frame (currently the dark loading screen, ~black, matching the
  mid-frame B≈4 measurement).
- The title's own "Americas" window stays white: winewayland does not composite our Vulkan swapchain onto a window
  the title created. Long-term fix (not done): present through winewayland's own surface for the title's window.
- Present-order/chain diagnostics (`c2bb4a2db`, `9d588b911`, `f2b8031d1`, `0c9ee3e03`) pinned that the blit runs
  unrefused and the present succeeds; the issue was only ever the window it was aimed at.

### OOM fixed
- `c7f2c94b2` (A9bq): the per-draw tint fill ran once per draw per frame against the same roaming addresses — now
  once per distinct address. RSS flat. `LOG_COMPOSITE`'s staging was the real memory pressure; avoid it on long runs.

### Scene-shader refusals fixed (creation, not rendering)
- Mesh module placed in the **vertex** slot → `f2ebed5d7` (A9bu) routes `MS_`/`_ms_` modules to the mesh slot;
  `e7121e7fe` (A9bv) clears the mesh pipeline's input assembly/topology and made slot 57 legible.
- Depth-format code 118 (a console-side code absent from the table) was refused → `dd33335fb` (A9bw) substitutes
  D32_FLOAT. Refusals now **0**; the CascadeShadows/OctaOrtho pipelines **create**.
- `2acf70673` (A9by): PAGE_BACKED range mirror cap 256→4096 (a refused range = a root view read as zeros).

### The loading screen — eight theories tested and ruled out
The game reaches the loading screen (3 draw families) and stays. Each candidate was instrumented and eliminated:
1. Memory-budget throttle — **disproved** (`7c3171da7` console budget made it abort; the host figures had the title
   *under* budget yet it stalled). Reverted via `5db162bb3`.
2. GPU-progress-word deadlock via queue holds — **ruled out** (`1fa0d196d` LOG_HOLD: zero holds).
3. Scene-shader refusal — fixed (above), not the stall.
4. Range cap — fixed, not the stall.
5. XSpeechSynthesizer assert — `8bba6c0af` (A9ca) fixed an **intermittent** abort, not the consistent stall.
6. Genuine-still-loading — 600s, no advancement; *initially* ruled out, later re-understood (see below).
7. FSPX kind-6 — is the **FidelityFX upscaler's frame work**, not the loading path. A9 translated 21/29 upscaler
   programs (the corpus simply never had them); the loader's own 2 are among 8 that fail to decode. Parked.
8. Service layer — `ab7e9831d` (A9ce) census: only one unserved XPackage interface, which the forks also lack yet
   reach gameplay. Not the gate.

**Resolution (the two-sample thread measurement):** the title's threads burn ~100% CPU while `read_bytes` climbs
steadily (~11 MB/s) — **real work, not a spin**. The game is **genuinely loading**; it just loads slowly.

### Load speed root-caused and the fix committed
- `eb76dc5e7` (A9cf) + benchmark: the inflater reaches **183 MiB/s**; the disk is fine. The serialisation is the
  **single-worker, strictly-in-order dstorage queue** (one thread does read→inflate→copy end-to-end).
- `d498fba67` (A9ch): **ordered-drain worker pool** committed — `LEONIDA_DSTORAGE_THREADS` (default 4/5 of cores ≈
  12/16, capped below 100% so other apps stay smooth; `=1` reproduces the serial path exactly). Work is parallel,
  reporting stays on one thread in submission order. Pending-count/read lifetimes and Close's drain handled.

## Decisions made

- Subagents write/analyse/verify-by-reading; the orchestrator does all builds, runs, screenshots (user mandate).
- Commit every experiment behind an env flag so a "revert" is a `git checkout`, never re-engineered (user's request).
  The branch is the navigable history; consolidate into `merge/port-gellhorn` once, when stable (it diverged, 117
  commits ahead — a real merge, deferred deliberately).
- Per AGENTS.md: no game data in the repo (the FSPX translations went to the work folder, not the tree); "fix,
  don't disable" for translator rejections; nothing online.
- A Wine page fault is caught by Wine's VEH — a Unix SIGSEGV handler never fires; get the faulting module from the
  dump or read the code (this is how the varargs bug was found).

## Pending items

- **Build + A/B the worker pool** (`THREADS` unset vs `=1`) with `LEONIDA_LOG_STREAM_RATE=1`: expect ~11 MB/s at
  ~32% CPU → ~25 MB/s at ~80-85% CPU. Watch the three risky spots A9 named: a stall at submit/Close (ring bound),
  a status/fence out of order (drain wait condition), a file-handle fault (pending-count rule).
- **Advance past the loading screen** once the load is fast (or a long memory-clear run finishes it): watch for new
  FAMILY lines / scene dispatches / real content in the Leonida window.
- **Banked for the scene-rendering stage** (from the victory-fork/ztrx/TuxBox inspectors): meshlet-culling
  computed-index descriptor reads return zeros unless (a) the bound heap base is placed in the shader's user SGPRs
  (docs 3.4: s10 CS / s23 MS / s15 PS) and (b) the V# BASE_ADDRESS inside a bound descriptor table is backed
  (back_root_views backs root+table VAs, not the V# base inside a table). TuxBox also: never bind a PSO lacking a
  descriptor layout (+ a dropped-dispatch tripwire log); release guest GPU memory waits after ~5 s; instrument
  indirect-argument counts.
- Display polish: route the frame into the title's own "Americas" window (winewayland surface), not just the helper.
- The flaky intermittent NULL-write at game+0x2215c35 (seen 2×, crashes before presenting) — not yet root-caused.

## Changed architectural constraints

- **Measure the surface the draw actually writes.** The presented-black hunt failed because the readback sampled the
  empty swapchain buffer, not the render target. The mid-frame readback (sample right after the draw) is the correct
  measurement and is now the standard.
- **Present the render target the title draws into, not the swapchain buffer it hands to Present** (`A9bd`: the
  current frame's `composite_target()`), and show it in a window the layer owns and shows.
- **The dstorage queue is concurrent for work, serial for reporting.** Worker threads do request-local work only;
  one drain reports in submission order. No title-visible behaviour change.
- **Commit experiments behind flags; the branch is the history.** No uncommitted reverts.
- Memory is a real constraint on this host: the game holds ~14-15 GB reserved while loading; keep heavy apps (e.g.
  Minecraft) capped during runs, and prefer fast loads (less time at high memory).

## Run env + probes

`WINEPREFIX=~/.local/share/leonida-wine LEONIDA_D3D12=vulkan LEONIDA_ALLOW_MISSING_FSPX=1 LEONIDA_PAGE_BACKED_VB=1
LEONIDA_ARENA_MB=4095 LEONIDA_PRESENT_RT=1` (+ `LEONIDA_LOG_FAMILIES=1`, `LEONIDA_LOG_STREAM_RATE=1`,
`LEONIDA_LOG_RSS=1`, `LEONIDA_LOG_HOLD=1`, `LEONIDA_LOG_SERVICES=1`, `LEONIDA_PRESENT_PROBE_COLOR=1`,
`LEONIDA_RT_MIDFRAME=1`, `LEONIDA_DSTORAGE_THREADS=N` as needed). Build:
`cmake --build runtime/build/windows --parallel 10 --target gta6rt gta6rt_tls_host`, stage into `run-dirs/a9f-run/`,
run under `timeout N ... gta6rt.sh --run`. Long runs must be background processes (the 600s foreground cap).
