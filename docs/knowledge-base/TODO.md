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

- [x] X1. THIN_THREADS=14: run34 A/B neutral; kept as default (owner call, denser scenes).
- [x] X2. PADDING MASK merged (VICTORY_SHAPE_PADDING_MASK default on; hash-keyed shape index; tests). NOTE: it is also upstream now (born's line, re-authored) — skip the duplicate subject in the born merge.
- [x] X3. Xavi batches 1+2 merged (AVX2 take, BufferSweep, strides, DynamicBits, VP coalesce, null templates, Batch caches). FOLLOW-UP: wire HostHeap::release_many at its 3 call sites (descriptor.rs + 2 in draw.rs) — the packed scan is in, the 12k-locks->1 batched hold is not wired yet.
- [ ] X4. Vice-Tay tier A/B remaining: 1f69d67 (ticket allocations, shadow table lock), 369f940 (due texture slots one traversal), 3e3fd57 (head blend-shape queued before first dispatch). DONE: 4f2f58f + 3c83a33 (vtports, translator bump 0x4c00_0003 — one full re-translation next run).
- [x] X5. L1 merged (VICTORY_MAPPED_BATCH default on: fence wait once per submission; ~28,600 waits/run -> per-submission barriers; textures/pool blocks keep inline waits).
- Born upstream pull (139 commits since 542d441, the living line since avatarmanz's repo was deleted 2026-11-27): merge agent next round. Hot items: WINDOWS_BAR_MB=4096 (A/B in run35 env), CARD_TOUCHED=1, own memcpy under Wine (30x), skip-clean-shards, fxceless 1506/1507 (one shared translator bump with vtports), UI_PLANE_720 (2x UI at 1080p). Skip: the 14 WC duplicates, the padding-mask duplicate subject, 29 in-range duplicate subjects (apply by subject, never as a range).
- [ ] S0 read probe (lazy-stage-a-spec.md): PAGE_NOACCESS sample of merged/WC blocks, count reads, unprotect — verifies WC card_owns ranges are never CPU-read (~2 ms/frame sampled; correctness, not speed). Lazy A/B/C killed by the numbers (waste 2-4 ms vs fault cost 7-40x).
- [ ] Sub-block THIRD STATE (subblock-precision.md): guard-answered blocks COMPARED not blindly copied (~22.7 MB of 40.9 MB blind copy, -3..-6 ms) + parts snapshot per flush (~1,000 journal read locks/frame) + 3 counters. shadow.rs/watch.rs — sequence after the born merge.
- [ ] Perf round-2 L1: chunked dirty-page reads in Shadow::take (shadow.rs:5005-5045, 0.7 us/page -> 64-256 KiB chunks, -6..-10 ms) + L2 guard-lock-off-VirtualProtect (-4..-7).
- SKIP (recorded): Xavi PORT_BUFFERS/INPLACE=2 (opposite memory model to our guards; their own community A/B ~1-3 ms), Vice-Tay Nvidia families (DLSS/frame-gen, NVIDIA memory profiles, heap-identity), paired-lane wave64 (their own #378 shows 32-lane regressions), 83ba8e2 (needs ASK_SPREAD we lack), their Tk launcher/DS4 items, Xavi headline 20.8 (their own 37-min README: 15.35 median declining — best case, not steady state).
- Vice-Tay's Linux change = the rbtree wineserver: WE ALREADY HAVE IT (nothing to pull).
- Banding lead (when we hit it): pipeline library x RTV-format mismatch, VICTORY_RTV_FORMAT_LOG=1 to check.

## Mirror sweep yield (pr-mirror-sweep.md, 305 PR heads; sequence AFTER the born merge — same files)

- [ ] M1. #475 PAGEMAP_SCAN from the runtime (kernel write-watch asks past Wine's VM lock, ~20 ms/GB walk avoided; new wine.rs+pagemap.rs; take #475's wine.rs only, #129/#166 create it differently).
- [ ] M2. #469 translator v2 (17 commits: helper-lane readfirstlane, g16 gradients, annotateHandle AMD crash fix, decode fixes; rebase translator version onto ours, one bump with the fxceless/born line).
- [ ] M3. #405 SCALAR_SPLITS to every widen caller + divergent-branch scalars + RDNA2 fuzz oracle harness.
- [ ] M4. #338 built-in shaders' root signatures serialized (vkd3d E_FAIL insurance; rootstring.rs; touches our ambient.rs).
- [ ] M5. #205 host RAM footprint (VM decommit + ram_trim.rs + trims; heavy rebase, land after #72; measure RSS on a matched run).
- [ ] M6. #72 opt-in 90/85% LRU VRAM GC + upload pool + retry-on-evict (off by default).
- [ ] M7. Cheap follow-ons: #334 (DXBC hashing), #81 (wave64 unknown block index), #410 (operandless VOP1 decode + invalidation), #476/#474 scratch chunks (take one — shared membench.rs).
- Mirror otherwise confirmed redundant (305 heads swept; the 167 remainder are Windows-side D3D12-layer work vkd3d replaces for us; 13 shader decode hardenings catalogued as a batch if a decode-side crash appears).

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
