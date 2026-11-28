# 2026-11-28 — the born-merge regression, the publish-race crashes, and the rollback

Morning state (run40, the reference): 09f7e4c7 = guards-v3 (8 shards, ViewsStep). Prologue 66.3 ms median / p90 77.6 / 15.1 fps, stable, mouths+faces working.

## What landed and what it cost

- Five cherry-pick batches (padmask, xavi1+2, vtports, L1 mapped batch): all wins. Map waits 18.6 -> 6.5 s/run (run35).
- born/linux-wine bd98961 (139 commits, the successor after avatarmanz's deletion): merged 340d4446. run36 REGRESSED (80.4 med / p90 195 / stdev 46). Dig: guard-mutex serialization amplification (295->376 us/acq); born's land_before_remap was the top suspect (launcher default =0 since).
- Mirror picks (#475 PAGEMAP_SCAN, #469+#405 translator v2 (0x4c00_0006), #338 rootstring + #334/#81/#410/#476): merged clean.
- perfbatch (release_many, third-state compare, chunked reads, S0 probe) + memtrim (#72 VRAM GC/upload pool, #205 RAM trims): run41 CATASTROPHIC (0.2-0.7 fps, recording 53.8 s/frame = #72's RESOURCE_SLOTS global-mutex futex convoy; gated off unless GC on). Then a long regression tail (~78 ms vs 68.6).
- hotfix (partial epoch, protect coalesce, land sub-pieces, 16 MiB shards, land jobs, one-pass merge): run47 93 ms; its per-item costs (epoch-partial bookkeeping, coalesce per-span) named by review.
- The third-state compare was convicted of COST (refreshes 22.2 -> 97 ms) and defaulted off; release_many exonerated (a gain); the probe removed.

## The recurring title AVs (runs 44/46/48/52/56/58 + the pre-existing 01:30 crash)

Root cause (title-av.md): the title's own publish-then-read race on its job system — records published header-first (sizes/counts in what becomes the pointer slot), a consumer job reads mid-publish. On console the timing never allows it; our layer stretches the producer side (guard faults on title stores at tens of us, destroy+recreate covering views at 20k-25k views/s). Small-integer derefs (0xf0/0x100/5) = header words read as pointers; null vptrs = never-constructed objects; wild non-canonical pointers = unwritten pages.
- crashguard (merged): vtable_load walk-back <=16 B with register-setup proof, chain_load movzx/movsx word+byte forms, small-write skip, sentinel loads; then VEX (vmovups/vmovsd) stores (run56) and wild non-canonical loads (run58) added with tests. 75 loader tests green. Exit counters (NULL_VTABLE_SKIPS/NULL_CHAIN_SKIPS/WRITE_SKIPS) measure the race rate. VICTORY_WATCH_RANGE = one-page write journal.
- viewsplit (merged): placeholders cut at lo/hi instead of destroy+recreate; joins opt-in (VICTORY_JOIN_VIEWS=1). Removes the stall + ~50k splits/joins of churn.
- fallback validation (merged): a fallback view record is made only if its words still are one (re-read compare + view_at + is_resource) — later gated to the check path only (not every made record).

## Profiling and the rollback

perf 7.3.6 installed (user-approved). SLOW (master) profiled: descriptor-heap keep path 23% of layer CPU, beside::serve 3.39% (0 in FAST), deflate_quick 1.00% (#205 compression, 0 in FAST), long tail of small increases. FAST capture completed by the orchestrator after the agent's attempts failed (cs2 contention + the no-launch rule). Conclusion: the regression is spread across the born merge's many small additions + memtrim trims + hotfix land changes — no single culprit; every single-branch revert/flip failed to restore.

Decision (user): roll back to 09f7e4c7 and re-apply only the good work. New master = 09f7e4c7 + crashguard (all shapes), viewsplit, fallback validation (gated), release_many. Old master parked at master-before-rollback for mining (born, mirror picks, memtrim, hotfix, the compare, the trims).

## Parallel track (VIctory-Fast = maren-halv, fxceless lineage)

Scout: the 40+ fps headline is ASPIRATIONAL (unreachable SHAs, no harness; own notes show 46-65 ms = our numbers). Applicable pull list in TODO.md (F1-F10 + SEC_COMMIT probe). Two port agents on the GOOD tree (09f7e4c7) — fast-land (F2: AVX2 one-pass copy-back merge + own-view writes) and fast-light (F3: nothing_new pre-check, TLS FOUND cache, seqlock mirror, is_dead) — merge straight onto the new master when they report.

## Conventions now in force

- Subagents NEVER launch the game (hard rule) — runs via orchestrator with the user's greenlight only. Ask before installing software. Agents hold builds while a run is active (CARGO_BUILD_JOBS=2 nice -n 19 when free).
- Perf claims need workload controls (title copies/draws/refreshed GB); scene noise at 13-20 samples is ±6 ms. ABBA protocol for A/Bs.
