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

## Merge landed + runs 14-16 (evening update)

- Merge committed as `dcc6c64` (reviewed: 215 shader + 55 loader tests verified
  independently, guard intact; resolution notes in the commit and pr440-merge.md).
- **VICTORY_WINE_MAP is poison on this stack** (runs 14/15): the mmap path's kernel
  write-tracking misses writes through the views, so 10-13 GB/frame of title memory
  read as changed — 0.2-0.5 fps, 793 slow kernel askings at 6161 ms/frame, 21.6 min
  CPU in 4.5 min wall. `VICTORY_WINE_MAP=0` is now the launcher default (`5645d54`+);
  the rbtree server already covers the map/unmap cost it exists for. run16 with it
  off: back to 10-20 fps, intro rubberband gone (VICTORY_JOIN_PARTS_DRAWS=500).
- **series-s broken** (above) and **run14's big UI/subtitles** were just 1080p-native
  layout, not a bug.
- **Muddy textures: SOLVED — the merge imported main's over_cap()** (resource.rs:1303,
  after vitesse 7ada43b6): with VICTORY_TEXTURE_MAX unset, every streamed placed
  texture's top mips are capped at 1024 px (512 at ≥92% of the DXGI budget, 256 at
  ≥97%) — base_level() forces first = max(first, over_cap) and views are re-based so
  sampling stays correct but the top mips are never on the card. The texstore can't
  rescue them (staging refuses first != 0). Verified in-tree + absent pre-merge.
  Launcher default now VICTORY_TEXTURE_MAX=0 (16 GB card; revisit if VRAM stalls).
  Caveat: the PR's 27 ms numbers were measured WITH the cap — not comparable to
  our pre-merge frames. Two minor merge defects noted by the agent (duplicate
  aliasing barrier in PreludeUpload::record; claim_upload flag race on a failed
  prelude list) — filed, harmless, unfixed.
- Warmup-with-progress (zenity) being built by a subagent: pre-translation +
  `victory warm --used` BEFORE the title launches, so cold starts don't play out
  on screen. Lands before the next run.
- run17 (the judgment run) waits for the user's ready: texture verdict + warmup in.

## Late-evening: mouths root-caused (upstream fix landed), perf levers measured

- **Mouths = fork B** (title DID evaluate facial clips; delivery died in translation):
  the face network's 16-bit accesses were lowered as f32 through INTEGER views
  (table views never got their format-kind word; d16 loads/stores lowered as
  bufferLoad.f32 — UB through an integer view — plus a RADV post-barrier texel-load
  hazard read through the raw view). Network output = zeros = face at rest.
  Upstream fix 542d441 (+3721030 test fix; = e47cce0 on born/linux-wine): pull 114 +
  fxceless 927's half views + raw typed loads. Translator version bumped
  (VSH2 -> VSL3 cache magic) -> ONE full re-translation; known-pipelines stay valid.
  VICTORY_RAW_TYPED_LOADS must stay on (default); new default VICTORY_FIRSTLANE_OWN=1
  for spilled user-data programs (=0 if regressions). warm --used before the
  judgment run or dropped boot compute fakes the old static face.
- **Perf levers, measured from our own logs** (in-world ~72-77 ms/frame, GPU ~0-10 ms):
  1. STRUCTURAL: relay's per-landing BLOCKING PCIe fetch (64 KiB pieces, own copy
     queue, SetEventOnCompletion+wait; 26.7 ms of the 48.6 ms copy-back). Fix:
     fetch landing N while merging N-1; join pieces across 64 KiB blocks.
     ~12-16 ms/frame.
  2. SMALL CODE: one global guards mutex held across VirtualProtect (13.6 ms
     protections + 17 ms lock-waits per frame; 388 blocks ~35 us each, Wine server
     round trips). Fix: shard by the 64 MiB region index, lock for set-lookups only,
     coalesce protections. ~6-10 ms/frame.
  3. PORT #1 IS INERT FOR US: home_on() = port_on && wine_map::on && PORT_HOME!=0 —
     our VICTORY_WINE_MAP=0 kills the 174 MB constant heap's in-place use (it placed
     in run14 with mmap on: "1 regions (185 MB), 1 title buffers placed"). Fix:
     decouple the 32-bit graphics stretch's home from the global mmap switch.
     ~13-15 ms preparing + ~2 ms copy-back. That heap: 3 GB/frame inspected,
     16.6x/frame refreshed, 45% of the up-bus, ~4,667 small CBV fills/frame.
  4. Cheap switches (evidence-backed, set for the next run): VICTORY_WIDE_MORE=1
     (refresh threads, 35 -> 13 ms measured once), VICTORY_TAKE_FINE=1. ~4-6 ms.
  Ruled out by measurement: LAND_SKIP, ASK_EARLY, ASK_NORESET, WIDE_MERGE, CARD_FINE.
  Copy-back answers the title's CPU reads of GPU-written buffers (union of UAV view
  extents, compare-narrowed; ~16.6 MB/frame fetched and never read = pure waste —
  lazy read-fault copy-back is the documented ceiling-remover).
  Combined expectation: ~75 -> ~45-52 ms/frame (19-22 fps), port#1 on top -> 22-28.
- PR #440 thread (25 comments) read in full: YaBulochka 8 GB data (PORT_CONSTANTS
  int3+ring-timeout on small cards; #446 "clear stale constant-buffer descriptors"
  stabilises it), VICTORY_PC_FSR=1 + FSR4.x acceptance (RDNA3 FSR4 under Wine),
  COMPUTE_WAIT_MS default now 500 ms (1bf01b5; 10 s costs ~60 s of 5 s frames COLD —
  ours is warm so =10 is inert), UwUCammie's worker-watch handle-group fix (bd71120,
  >64-handle waits rejected by Wine — our pools are big), RADV driver-side pipeline
  creation is 52% of samples in the first long stall.
- Slaktarn700/SlaktarnVI scout dispatched (its nuget shader-compiler claim vs our
  winetricks d3dcompiler_47; cherry-pick list pending).
