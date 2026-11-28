# TODO tracker — LeoTux engineering queue

Benchmark scene: THE PROLOGUE (renders real world content; 1-to-1 A/B comparable).
All fps claims reference prologue runs on the 7800 XT. Optimize the prologue first;
the open world comes after.

## Pull from Slaktarn700/SlaktarnVI (scout report 2026-11-27; patches extracted in scratch/slaktarn-scout/patches/)

Parallel effort (~10 commits behind our avatarmanz base, zero PR #440 content;
144/291 files byte-identical). Ranked pull list:

- [ ] 1. VICTORY_CARD_STEPS_LEARN (host/card_steps.rs `mod late` + texture.rs wiring,
      1169+/45-): reads fills back after landing, learns late-written pages; fixes
      skinned characters/boats flung off, random-colour shirts. Their A/B: 88 vs
      94-95 ms/frame, on by default there. CONFLICTS: same file as our 5b9babf/377e9a1
      bounds guard — take the `mod late` block + its landed() call only.
- [ ] 2. Descriptor raw-twin fix (VICTORY_TWIN_EXACT/move_raw, descriptor/host.rs,
      ~790+/85-): raw twins at the view's own dword; fixes trees/palms dropping for
      single frames under RADV, "faces/arms/clothes in coloured squares" (avatarmanz#437
      lineage). CONFLICTS medium-high: 542d441 touched the same raw-view/twin code;
      orthogonal, switch-gated.
- [ ] 3. Bink tick clock (tick_clock.rs + 2-line hooks, 141+/31-): QPC for
      Bink2Scarlett.dll returns the tick's start; kills 30fps video judder
      (3,1,2,3,1,2 ticks -> 2). Trivial conflicts.
- [ ] 4. fastmem (apps/victory/src/fastmem.rs, 394+/2-): own memcmp ahead of Wine
      ucrtbase's scalar one (hottest submit-thread code). No conflicts; A/B measure.
- [ ] 5. Guarded dump (guarded.rs + crash.rs, 319+/24-): VEH-guarded RtlVirtualUnwind
      so a Wine stack dump cannot kill the process. Complements our crash.rs.
- [ ] 6. log-threads (14 lines): thread names on log lines; how they found five
      dxil-spirv worker overflows. Clean.
- [ ] 7. rootstring.rs (465+/5-): serialize the root signature from the HLSL RS string
      — insurance for vkd3d E_FAIL we do NOT currently hit. Take only if our vkd3d
      regresses.
- [ ] 8. VICTORY_UNKNOWN_PIPELINE_DIR diagnostics (~130+): pipelines never made.

SKIP (with reasons): their wineserver patch (we build our own rbtree vs 11.21;
theirs is Proton-protocol), valhalla.py/s70.sh launcher stack (we have
run-victory-direct.sh + warm-up.sh), 64 MiB worker stack (ours 256 MiB),
VICTORY_LOOK_GUARDS (documented regression, superseded by CARD_STEPS_LEARN),
VICTORY_PORT_PRIVATE / xmem.rs +1134 (head-on conflict with our PORT_HOME+wine_map
path — not additive), RECLAIM_DYNAMIC half of their descriptor work (~600 lines;
our sizing/bookkeeping already matches), PIPELINE_WAIT_MS=2 advice, FSR4 framegen
(RDNA4-only; we're RDNA3), their Proton run scripts, cosmetic scrubs.
Nuget shader compiler: NO ACTION — our prefix's d3dcompiler_47.dll is Microsoft's
real compiler (sha256-verified vs winetricks' payload, forced native); their claim
covers only Wine's builtin. dxil.dll unneeded at runtime (vkd3d carries dxil-spirv).

## Scout pull lists (2026-11-28, reports in scratch: xavi-rdna-scout.md, vice-tay-scout.md)

- [ ] X1. THIN_THREADS=14 (Xavi preset, measured +2.8 fps warm on 7900 XT): A/B first (run34), default if it holds.
- [ ] X2. Descriptor shape-index PADDING MASK (Vice-Tay f07e54a+ce91490, apply clean; 18bc367 manual): identical views stop becoming new shapes — fixes the 2^22-shapes ~30-min / 16.7 GB / fps-decay leak. LONG-SESSION CRITICAL.
- [ ] X3. Xavi clean picks: 6072c37 (AVX2 Shadow::take), f8eb38d (BufferSweep dedup+span cache), 61477a8 (CPU strides/view_addr shadows), 13e1f5e+b424c8b (DynamicBits packed allocator, 12k locks->1), 8abd367 (coalesce un-guard VirtualProtect — Wine round trips), 5663897 (Batch::last_ahead cache, WIDE_PLAN_STEPS 4096->1024), e1f8b65 (WIDE_JOB 256 KiB).
- [ ] X4. Vice-Tay tier A/B: 1f69d67 (ticket loose/exact allocations, shadow table lock), 4f2f58f (AMD mul-add zero products — correctness), 3c83a33 (RDNA2 scalar min/max flags), 369f940 (due texture slots one traversal), 3e3fd57 (head blend-shape queued before first dispatch — the heads/faces hitch).
- [ ] X5. L1 barrier batching (card-memory-waits.md): mapped() fence wait once per submission, not per allocation — ~22 s of map waits -> 6-8 s projected. The biggest single remaining item.
- SKIP (recorded): Xavi PORT_BUFFERS/INPLACE=2 (opposite memory model to our guards; their own community A/B ~1-3 ms), Vice-Tay Nvidia families (DLSS/frame-gen, NVIDIA memory profiles, heap-identity), paired-lane wave64 (their own #378 shows 32-lane regressions), 83ba8e2 (needs ASK_SPREAD we lack), their Tk launcher/DS4 items, Xavi headline 20.8 (their own 37-min README: 15.35 median declining — best case, not steady state).
- Vice-Tay's Linux change = the rbtree wineserver: WE ALREADY HAVE IT (nothing to pull).
- Banding lead (when we hit it): pipeline library x RTV-format mismatch, VICTORY_RTV_FORMAT_LOG=1 to check.

## Perf levers from our own telemetry (perf-levers.md 2026-11-27; prologue ~75-100 ms/frame, GPU ~0-10 ms)

- [ ] P1. Relay fetch-ahead: fetch landing N while merging N-1; join pieces across
      64 KiB blocks (host.rs:5014 relay_card_back blocking SetEventOnCompletion;
      26.7 of 48.6 ms copy-back). ~12-16 ms/frame.
- [ ] P2. Guards sharding: one global guards mutex held across VirtualProtect
      (watch.rs:2387; 13.6 ms protections + 17 ms lock-waits/frame). Shard by the
      64 MiB region index; lock for set-lookups only; coalesce protections.
      ~6-10 ms/frame.
- [x] P3. PORT_HOME decouple (DONE 2026-11-27, wine_map::available() split):
      PORT_HOME=1 A/B in run20b. ~15 ms if it holds.
- [x] P4. Cheap switches: VICTORY_WIDE_MORE=1 + VICTORY_TAKE_FINE=1 (in runs 20a/b;
      no visible change at the 5s-line granularity — re-evaluate with same-scene A/B).
- [ ] P5. Lazy read-fault copy-back (structural): ~16.6 MB/frame fetched and never
      CPU-read = pure waste; fault-triggered fetch is the documented ceiling-remover.
- [ ] From PR #440 thread: #446 "clear stale constant-buffer descriptors" (stabilises
      PORT_CONSTANTS on other cards); VICTORY_WC_NO_BACK=1 (Belobug#477, shader writes
      stay write-combined on-card; ~5 ms/frame on their submitting thread — our
      copy-back is 33-54 ms, so this is the next big switch to try after the A/Bs);
      VICTORY_PC_FSR=1 + FSR4.x acceptance (future sharpness option).

## Later digs

- [ ] RT/BVH tree format (the console's ray-tracing structures): document the layout
      by measurement (captures via the layer's own replay/poke + the BVH builder's
      fault-bounds knowledge from the merge) so the RT path (dark scenes, phone,
      ambient-RT) runs on standard DXR trees. Big, not started; the merge already
      carries avatarmanz's BVH-fault bounds groundwork.

## Diagnostics / quality of life

- [ ] LeoTux-old cherry-picks (cherry-pick-inventory.md): 44-row null-ref skip family
      (B, M), debug-register watchpoint + title-write ledger (C, M/M), read-path
      ledger (A, S), 3 small diagnostics (C, S each).
- [ ] DS4: udev rule installed; verify in a run (wine enumerates hidraw pads at start).
- [ ] Guard live validation: 0 skips so far (the race is rare); keep counting.
