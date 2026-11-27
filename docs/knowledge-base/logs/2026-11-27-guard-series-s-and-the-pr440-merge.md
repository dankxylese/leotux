# 2026-11-27 — null-vtable guard, series-s ruled out, PR #440 + upstream merge in flight

State: the tree plays in-world at the LeoTux-old baseline (~10 fps typical, 15-20 peaks,
series-x at 4K, faces correct, mouths static as upstream-known). run13 = the current
reference run. Next: the avatarmanz-main + PR #440 merge, then the comparison run.

## Added on top (committed)

- **VEH null-vtable survival guard** (`2c4da40`, victory-loader/crash.rs, flag
  `VICTORY_SKIP_NULL_VTABLE` on unless 0): on an access violation that is exactly a
  vtable call whose pointer loaded as null — read fault below 0x10000, the exact
  two-instruction shape (`mov reg,[reg2]` + `call|jmp *disp(reg)`, slot 0..0x78 in 8s),
  the register exactly 0 and the fault exactly the slot — the call is skipped with
  rax=0 instead of ending the process. Per-site loop breaker (64 fixed sites,
  8 skips/60 s), no locks/allocs in the handler, rate-limited `null vtable call skipped`
  warnings, at-exit + crash-report accounting. 43/43 lib tests pass incl. its three.
  Live so far: 0 skips (run13; the race is rare — validation stays opportunistic).
  The crash it survives was root-caused as title-side (dig-7b61a7: a streaming spawn
  descriptor consumed before its object is constructed; layer-stub cause ruled out) —
  the upstream-known "null pointer in own code" family.
- **victory-belobug / victory-dexnom dropped** (`4f10344`): far behind victory-7900xtx
  under Wine (about 5 and 1-4 fps against its 6-10; belobug's renderer patches black the
  picture). History keeps them.
- **Launcher = run-victory-direct.sh** (`63e9d93`, `5645d54`, plus): LeoTux tree's
  victory.exe, series-x, the rbtree wineserver PRE-STARTED with -p (wine 11.21's loader
  checks the install dir before `$WINESERVER`, so the env var alone is a no-op —
  strace + loader.c verified), warm-then-run, logs to LeoTux-logs. Abandoned
  Proton-path scripts (run-manse.sh, proton-umu.sh) deleted.
- **README rewritten** (`1a91852`): our path and numbers — stock distro Wine 11.21,
  no Proton build; GE-Proton11-14's vkd3d-proton dlls (post-3.2; older 3.1.0 crashes
  vkCreateGraphicsPipelines), DXVK dxgi, Microsoft d3dcompiler_47; rbtree server as a
  6-second server-only rebuild; the rejected wines (proton-cachyos services.exe stack
  overflow; GE 11.0-staging breaks skinned geometry — eyes/teeth only).
- **DS4/PS4 controller**: udev uaccess rule installed for the Sony ids (root-only
  /dev/hidraw* was why Wine never saw the pad); wine enumerates hidraw controllers at
  startup, so it takes a (re)start.

## Decision with a constraint attached

- **series-s is BROKEN on this stack — do not offer it.** run12 evidence: with
  `--console series-s` the title stalls right after `core_licenses_data.rpf` (13:16:15),
  before the boot-compute burst and audio init: reads flat, threads idle, an empty
  30 fps frame (11 draws) indefinitely. series-x passes the same point in 2 ms.
  Prime suspect: the Series-S memory profile the layer reports (0 GPU-optimal +
  7,776 MiB standard, vs Series X's 10 GiB + 3.5 GiB) — a zero or too-small budget
  wedging the title's pool setup; the device-type read (0x60) is the trigger.
  Upstream never verified series-s either ("play.sh passes no arguments").
  1080p comes from `VICTORY_OUTPUT=1920x1080` (post-merge), NOT the series-s profile.
  README and the launcher comment carry this; docs/linux-progress.md should get the
  run12 line when next touched.

## Reviewed and queued (ports from LeoTux-old; inventory in scratch as cherry-pick-inventory.md)

Prioritized, citations verified by the inventory agent, sizes S/M/L:
- (B, M) the 44-row null-ref skip family — broader than our guard: reads off fields
  never bound, landing at the title's own absent-path; keep our rate-limit/accounting.
- (C, M) hardware write watchpoints (Dr0-3 as one composed resource, multi-address,
  dynamic arm, stay-armed) + (C, M) the title-write ledger with fault-time overlap
  report ("did OUR layer write there?" in one run). Neither exists in the Rust layer.
- (A, S) read-path ledger at the title's ReadFile (per-file bytes/short reads/rate) —
  the instrument for loading-speed work.
- (C, S x3) alloc-ledger exec-bp, int3 shared-stub stack-window print, re-report
  cadence for permanent conditions.
- (B, M x2) survival recipes to take ONLY if the failure mode appears: faulting-free
  epilogue skip; import-boundary copy clamp (log-only first, to find our own heads).
Dropped with reasons (20+): dstorage/BCPack/pipeline/indirect-clamp items the Rust
layer already has or that have no analogue (the C++ VA-mirror work, fence polling,
tint table, present-path substitution).

## Mouths (from the inventory; feeds the later fix)

LeoTux-old never touched mouths at runtime. But the lip-sync payload is decoded:
AWC tag 0x5c facial clips, 90 tracks/clip, keyframes 100% decoded, channels are
translations only (NO quaternion channel — kills the "dropped rotation" hypothesis).
The rig: FB_* bones (jaw, 24 teeth, 11-bone tongue) + the compute blendshape head pass
(ped_morphable_model_cs -> ped_standard_head_cs -> ped_skinning_cs). The unbuilt piece:
track_id_hash -> FB_*/blendshape-control mapping and evaluation. New lead from PR #440:
the face-deform compute pass (970c4fbb3b017c0e) fills a per-character table that was
being DROPPED while its pipeline compiled — same dropped-compute class to check for
mouth motion once the merge lands.

## PR #440 + avatarmanz main (merge agent in flight, scratch repo)

- avatarmanz/main moved 186 commits past our fork (flicker fixes, ambient-RT dark
  rooms, BVH fault bounds, renderer-port-RT switch groundwork).
- PR #440 (borntofrag34:linux-wine-pr, open): their measured path 441 -> 27 ms a frame
  at 1080p on a 7900 GRE. Its pieces and why they matter to us:
  - `VICTORY_WINE_MAP` (mmap title memory): removes wineserver from the map/unmap path
    (their wineserver 76% -> 0.2% CPU); complements the rbtree server we already run
    (that one fixes the walk but keeps the round-trips).
  - `VICTORY_SINGLE_LATCH` + `VICTORY_PIPELINE_WATCH`: the dxil-spirv infinite mutual
    recursion (multi-latch loops + WaveReadLaneAt) that silently kills pipeline worker
    threads — the same bug family as our runs 4-8 vkCreateGraphicsPipelines crashes,
    which we fixed from the other side with GE's newer vkd3d.
  - `VICTORY_COMPUTE_WAIT_MS` + `VICTORY_LIBRARY_NULL`: deterministic faces (17/17 with
    vs 2/5 without) and pipelines kept across runs (first-use drops 400-630 -> 7).
  - `VICTORY_OUTPUT=1920x1080` (25.5 fps vs 20.9 at 4K in the first scene),
    `VICTORY_FRAME_INTERVAL_MS` (lifts the title's 30 fps cap),
    renderer patch 1 under Wine (`VICTORY_PORT_HOME`), card-side buffer copies,
    and the intro-video three-frame repeat explained (VICTORY_JOIN_PARTS joining;
    =1 avoids it at ~3 ms).
- Plan: agent merges in a scratch repo (3-way via synthetic history — the monorepo
  shares no git base with avatarmanz) -> orchestrator reviews resolutions -> apply to
  LeoTux -> rebuild -> run14 comparison (series-x + VICTORY_OUTPUT=1920x1080) ->
  faces determinism, fps, stability -> push.

## Open / next

1. Apply the merge, verify, run14 A/B (1080p, fps, faces, stability).
2. Port the inventory's B/C items, guard-first; A-1 read ledger before any loading work.
3. Mouths: check the facial-clip -> deform-table path against the dropped-compute class;
   the track_id_hash mapping is the data-side dig if runtime is clean.
4. Guard live validation (no pressure: 0 skips in two runs is the race being rare).
