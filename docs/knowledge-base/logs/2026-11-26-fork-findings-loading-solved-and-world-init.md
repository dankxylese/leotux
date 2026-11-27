# Session log — 2026-11-26 — Fork findings, the loading stall solved, and the world-init crash chain

Branch: `feat/arena-single-address-space` (worktree `worktrees/a9f-descriptor-bridge`). Host: CachyOS, RX 7800 XT
(RADV), wine 11.21, KDE Wayland. Game pkg 1.0.2.1 — now **on the SSD**: the dump at
`GTA6-Emulation/GTA6.Xbox.v1.0.2.1-CYBERLEEK/`, the work folder at `GTA6-Emulation/leonida-work/` (Sontse HDD is
out of all paths; run-dir pointers updated).

This era ran from "stuck at the loading screen, maybe a dstorage problem" to "the title reaches world init and
crashes inside it, one blocker at a time". Everything below is uncommitted in the worktree; the commit proposal is
at the end. The execution tracker is `LeoTux/docs/discovery/TRACKER.md` (ID'd items, statuses live).

## TL;DR

- **The loading stall is solved, and it was never dstorage.** The new POOL telemetry proves the title stops
  *submitting* (depth 0, no failures). The gate was **overlay state**: a fresh overlay gives the full slow-load
  profile (16-18 GB RSS, 11+ GiB read), the accumulated overlay parks the title at ~3 GB. The bulk loader is the
  title's **own direct-read path** (4-9 MiB/s, CPU-bound), not the dstorage queue (which serves one fast burst).
- **The Wine threadpool pile-up is our own FenceMirror**: `SetThreadpoolWait` re-armed at 10 ms from its callback,
  ~100 wakeups/s through Wine's `waitqueue.cs`, starving the title's pool. Callback confirmed inside gta6rt.exe's
  `.text` (base 0x10000000000, RVA 0xAD890; memfd-backed pages read as "anonymous"). Fix: `LEONIDA_FENCE_POLL_MS`
  (default 250, 0 = event-only, 10 = old behaviour).
- **World-init crash chain, three blockers cleared**: `GetResourceAllocationInfo` format 118 → D32 substitution;
  `CreateCommittedResource` flag combo 0x6 (which is **DEPTH_STENCIL|UNORDERED_ACCESS**, not RT|DS) accepted as
  depth-only; detailed self-describing refusal logs. Now blocked at a **NULL read at game+0x1e24e80** after the
  768x768x6 R16 shadow cube creation (candidates: refused amplification/mesh stage, refused depth UAV view,
  bindless image array full, T# null ×1024). Root-cause dig in flight.
- **Eight fork/peer reviews processed** (dexnom, belobug, translator-crossref, avatarmanz, rojikku-linux-layer,
  cool_fox, jokiii-btw, human) and two implementation waves landed from them. All knowledge filed in
  `LeoTux/docs/discovery/` + `leonida-reviews/`.

## What was done (by theme)

### Loading stall solved (LOAD-5, LOAD-6, LOAD-7)
- dstorage pool A/B: serial (`THREADS=1`) and pool both stall after one 256 MiB burst — pool exonerated. Telemetry
  added (5 s POOL line: completions, rates, busy/reading workers, staging waits, window, depth, live queues) shows
  the title simply stops submitting: **the blocker is title-side**.
- **The title opens 17 DirectStorage queues, all priority 2, each with a 12-thread pool = 204 threads.**
  `LEONIDA_DSTORAGE_SHARED_POOL=1` (one pool for all queues, 29 threads) added; A/B pending.
- Fresh-overlay discriminator: fresh overlay = full load; old overlay = park at ~3 GB. `overlay.bak` kept for
  analysis (LOAD-6). At 200-500 MB/s SSD reads to ~18 GB, then the CPU-bound tail (2-6 MB/s).
- Everything moved to the SSD: game dump, leonida-work (shaders), Wine prefix (`GTA6-Emulation/leonida-wine`),
  Mesa cache (`GTA6-Emulation/mesa-cache`, private + protected from eviction), `MESA_SHADER_CACHE_MAX_SIZE=10G`
  (RADV's 1 GB default was pruning world pipelines).

### Threadpool pile-up root-caused (LOAD-11)
- `WINEDEBUG=+threadpool`: pool threads time out (60 s) on `waitqueue.cs` while one thread churns wait callbacks
  (TpSetWait re-arm loops). Callback resolved: inside gta6rt.exe `.text` at base 0x10000000000 RVA 0xAD890
  (memfd-backed image pages = the "anonymous r-xp" region; address stable across runs).
- Cause: FenceMirror's 10 ms `SetThreadpoolWait` re-arm (~100/s per mirrored fence through Wine's pool lock).
  Fix: `LEONIDA_FENCE_POLL_MS` (250 default, 0 = event-only). Expect waitqueue timeouts gone + faster load tail.

### Wave 1 landed (fork findings → code; reviewed, tested)
- **dstorage**: 32 MiB staging window (`LEONIDA_DSTORAGE_MAX_INFLIGHT_MB`), POOL telemetry, shared-pool flag;
  per-queue ordered completion verified already correct; per-queue first-failure semantics verified.
  `dstorage_test` 331 checks under Wine.
- **xmem**: Wine whole-run commit (`LEONIDA_XMEM_WHOLE_COMMIT`, default on under Wine; per-page commits become
  region commits; Belobug's wineserver-round-trip finding). `xmem_test` 661+8 checks.
- **FSSM decode**: root-layout table (record+0x28) and used-resource table (+0x30) decoded; full-corpus census
  (26,503 blobs; 5,195,311 + 445,393 entries; old `binding_table()` framing corrected — it read 8 bytes into the
  first entry). Seven doc corrections applied: design.md:360 (**the enable word IS at 0x248**, `ac_spi_ps_input`
  mask, 12,087/12,101 ISG1 cross-check), PS-system-inputs, user-data row (heap bases: **PS s15/s16, MS s23/s24
  confirmed; CS is s7/s8, contradicting the s10 assumption**), export patch table row (29,290 sites, 10 format
  variants, shipped block is a placeholder), superseded-blocker note, `fssm_crosscheck.md` tail row,
  `d3d12_vulkan.md` §3.5. `test_fssm_header` 63 OK.
- **Translator**: PS quad-group lowering (quad-local DPP/DPP8/ds_swizzle → `OpGroupNonUniformQuad*`, wave shuffles
  kept for cross-quad) + helper-lane `v_readfirstlane` = own value. Census: 3,357 PS quad-local (36,086 instr),
  6,141 PS ds_swizzle QUAD_PERM, 5,189 PS readfirstlane. Full suite: **468 files, 6,119 tests, 0 failures**;
  byte-identical for unaffected stages.

### Wave 2 landed (reviewed, built, shim-tested)
- **Async pipeline creation** (`LEONIDA_PIPELINE_BUDGET_MS=500`, workers, slow quarter, warm list on disk):
  placeholder PSO + drop-with-reason; kills the >30 s inline-compile stall the forks warned about (and our
  deserialize freeze matched).
- **Indirect-arg GPU clamp** (`LEONIDA_INDIRECT_CLAMP`): ONE pass per maximal run of indirect executions (never a
  per-draw barrier — the forks' 190→72 ms lesson); count-buffer aware; per-list budget; offender counter behind
  `LEONIDA_LOG_INDIRECT` = first-scene flicker diagnostic.
- **Page-backed per-page aliasing guard** (`LEONIDA_PAGE_BACKED_ALIAS`): tracking was per-VA; now per backing page
  via `/proc/self/maps` identity (device+inode+offset) — a write through any view refreshes all (the forks'
  vehicle-stretch bug class).
- **Integer-RT blend bypass** (`LEONIDA_INT_BLEND_BYPASS`, per-target expansion first) + **per-kind null
  descriptors** (`LEONIDA_LOG_NULL_DESCRIPTORS`; the forks' seconds-into-world device loss).
- CMake link fixes: device gained `rt::xmem` refs → three test targets needed `xmem.cpp`+`registry.cpp`.

### Wine-layer findings
- Wine bug 60868 (wineserver O(n) view list): measured — wineserver CPU *declines* as views grow (~5k) — **not the
  stall**. Patched wineserver built anyway (`GTA6-Emulation/wine-patches/wineserver-11.21-rbtree`, protocol 971
  verified, README written); A/B queued as a loading speedup while views accumulate.
- Rojikku's world: no threadpool/esync/fsync work exists anywhere — the value was the 60868 lead, the
  `virtual_mutex` lead (unmeasured, LOAD-9), ReBAR/view-cap checks, cache-key traps, and the perf kit.

### World-init crash chain (LOAD-13)
1. `GetResourceAllocationInfo format not supported yet: 118` + slot-25 size -1 → title int3 (+0x344196). Fix:
   unknown depth code on a DS resource → widest depth format (D32, else D24), logged, both `texture_format` paths.
2. `CreateCommittedResource refused (resource flag combination)` at +0x14A6390. Three iterations (unknown-code
   test defeated by `render_target_family`'s default-accept; then detailed refusal logging added — format/flags/
   geometry/state — which showed **flags 0x6 = DEPTH_STENCIL|UNORDERED_ACCESS**: the console's shadow targets
   written from compute). General fix: ds+uav accepted as depth-only (no Vulkan depth STORAGE anyway); host rule
   kept for rt&&ds.
3. Now at **game+0x1e24e80 NULL read**, ~3 s past the shadow cube. Candidates in the log: amplification/mesh stage
   refused (×4 — the "the game has none" comment is disproven), depth-cube storage view refused, bindless image
   array full, T# null ×1024. Root-cause dig (title disassembly + shim-correlation) in flight.

### Reviews processed (all in `leonida-reviews/`, distilled in `docs/discovery/`)
- avatarmanz: submission-lane/thin-lists/flights architecture; per-kind null descriptors; texture slab costs;
  warm lists; cache keyed on translator version; 40-46 GB memory model; GI stand-in.
- cool_fox: 8 GB claim debunked (repo runs 32 GiB); residency loop + hysteresis; four stall candidates (memory-
  budget reporting, fence waits, monitorx/mwaitx, starved request); story enablers = saves + suspend + cutscene
  two-plane + ambient + streaming.
- rojikku-linux-layer: 60868 wineserver lead; virtual_mutex lead; no threadpool work exists; ReBAR hard req for
  them; cache-key traps; no LICENSE (techniques only).
- jokiii-btw: their Leonida line died *before* ours; world via VIctory+vkd3d; **the FenceMirror find** (LOAD-11);
  VIctory reaches world with FSPX/upscaler programs unmade (**our 44 slot-67 failures are not world-blocking**);
  MESA cache 10G; 64 KiB-alignment audit; watchdog — do NOT adopt theirs (ours is better).
- human (CGBJ+): GPU BCPack offload — the batched shape is the design (N chunks of a format family → 1 rANS + 1
  rebuild dispatch → 1 timeline semaphore; 32 KiB floor/24 MiB ceiling); per-chunk form is a regression by the
  SDK's own numbers; SDK is MIT/Apache-2.0, 14 precompiled Vulkan SPIR-V kernels, rANS on GPU. **Cheapest correct
  first step: wire our vendored CPU bcpack-rs into dstorage (M1-5), then the GPU batch path (M1-6)**, with the
  differential oracle (M1-7).

## Decisions made

- **Everything lives inside `GTA6-Emulation/`** (prefix, Mesa cache, wine-patches, run-dirs, fork clones, reviews,
  build dirs). Sontse HDD is out of all paths.
- **Execution tracking by ID'd items** (`docs/discovery/TRACKER.md`), prose stays in `leonida-reviews/` (user).
- **Small changes land directly; anything that needs digging goes to a subagent** (user).
- Host-validation rules are relaxed **only where the console provably differs** (DS|UAV), every relaxation logged;
  host rules kept elsewhere (RT|DS).
- **Refusal logs must be self-describing** (three iterations were lost to a refusal that didn't name its desc).
- Overlay is title-visible experiment state: runs declare overlay freshness (fresh vs continued).
- FSPX gap: parked as not-world-blocking (VIctory evidence); the two leads (self-contained VOP3P opsel decode;
  host-FidelityFX shim) await an architecture decision from the owner.
- GPU BCPack: batch design only; CPU decoder first; vendored MIT/Apache SPIR-V kernels as oracle before production.

## Pending items

- **Crash dig result + fix** (game+0x1e24e80; then continue the crash chain toward the first world frame).
- A/B queue: `LEONIDA_DSTORAGE_SHARED_POOL`, pipeline pool (deserialize-phase wall time, first-frame drops),
  indirect clamp counters, alias-guard lines, patched wineserver, `LEONIDA_FENCE_POLL_MS` (waitqueue timeouts
  gone? load tail faster?).
- `overlay.bak` vs fresh: name the state file that gates the title's path (LOAD-6).
- Direct-read bulk-path profile: where the 4-9 MiB/s goes through `file_system.cpp`; decompress-vs-shim split
  (LOAD-7). Then M1-5 (CPU BCPack into dstorage).
- Full corpus retranslation for the ps-quad translator change (required before any PR).
- This session's commit (proposal below), then continue.

## Changed architectural constraints

- **The title has two loaders**: the dstorage queue (one burst) and its own direct-read bulk path (the real load).
  Speed work targets the direct path + BCPack; the queue is healthy.
- **The overlay gates title behaviour.** Experiment hygiene: declare overlay freshness per run; a stalled profile
  is a state question first, a code question second.
- **Console flag semantics ≠ host**: DEPTH_STENCIL|UNORDERED_ACCESS is normal on console depth targets. Relax host
  rules only on evidence; log every relaxation.
- **Wine's threadpool is a shared-lock hazard**: no high-frequency re-arm loops through `SetThreadpoolWait`;
  backstop polls at ≥250 ms or dedicated watcher threads.
- **The console's FSSM tables name the heap bases**: PS s15/s16, MS s23/s24 (confirmed), **CS s7/s8** (contradicts
  the old s10 assumption; the compute side still needs the exe reading).
- **"the game has none" comments age badly**: the title presented four amplification/mesh stages. Every
  unsupported-class comment is a hypothesis until the title disproves it.

## Run env + probes (current)

`WINEPREFIX=GTA6-Emulation/leonida-wine MESA_SHADER_CACHE_DIR=GTA6-Emulation/mesa-cache
MESA_SHADER_CACHE_MAX_SIZE=10G LEONIDA_D3D12=vulkan LEONIDA_ALLOW_MISSING_FSPX=1 LEONIDA_PAGE_BACKED_VB=1
LEONIDA_ARENA_MB=4095 LEONIDA_PRESENT_RT=1 LEONIDA_LOG_STREAM_RATE=1 LEONIDA_LOG_DSTORAGE=1 LEONIDA_LOG_INDIRECT=1
LEONIDA_LOG_FAMILIES=1` (+ `LEONIDA_FENCE_POLL_MS`, `LEONIDA_PIPELINE_*`, `LEONIDA_INDIRECT_CLAMP`,
`LEONIDA_PAGE_BACKED_ALIAS`, `LEONIDA_XMEM_WHOLE_COMMIT`, `LEONIDA_DSTORAGE_SHARED_POOL`,
`LEONIDA_DSTORAGE_MAX_INFLIGHT_MB`, `WINESERVER=GTA6-Emulation/wine-patches/wineserver-11.21-rbtree` as needed).
Scripts: `run-dirs/a9f-run/run-long-load.sh` (60-min cap). Build:
`cmake --build runtime/build/windows --parallel 10 --target gta6rt gta6rt_tls_host`, stage into `run-dirs/a9f-run/`.

## docs/ touched this session

`docs/design.md` (7 edits), `docs/d3d12_vulkan.md` (§3.5, §4.7, A10), `docs/fssm_crosscheck.md`,
`docs/directstorage.md`, `docs/discovery/` (new tree + TRACKER.md), `docs/knowledge-base/logs/` (this file),
`runtime/CMakeLists.txt` (test-target link fixes). leonida-reviews/ (8 reviews, outside the repo by policy).

## Commit proposal

Eleven commits on `feat/arena-single-address-space`, in dependency order (each builds + its tests pass):

1. `dstorage`: staging window + POOL telemetry + queue-creation lines + `LEONIDA_DSTORAGE_SHARED_POOL` (A9ci)
2. `xmem`: Wine whole-run commit path (A9cj)
3. `tools`: FSSM root-layout/used-resource decode + census + the 7 doc corrections (A9ck)
4. `translator`: PS quad-group lowering + helper-lane readfirstlane + fixtures + contract rows (A9cl)
5. `runtime`: budgeted async pipeline pool + warm list (A9cm)
6. `runtime`: indirect-arg GPU clamp, one pass per run (A9cn)
7. `d3d12`: page-backed per-page aliasing guard (A9co)
8. `d3d12`: integer-RT blend bypass + per-kind null descriptors (A9cp)
9. `d3d12_xs`: fence-mirror poll backstop `LEONIDA_FENCE_POLL_MS` (A9cq)
10. `d3d12`: depth-code substitution (118) + DS|UAV depth targets + self-describing refusal logs (A9cr)
11. `cmake`: test-target link fixes for the device's rt::xmem refs (fold into 5-8 or standalone)

`python3 -I scripts/check_no_game_data.py` over every touched/new file before committing; the new fixtures are
hand-written (allowed). Then push the branch.
