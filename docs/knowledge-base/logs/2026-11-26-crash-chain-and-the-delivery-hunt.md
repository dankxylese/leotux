# Session log — 2026-11-26 (part 2) — The world-init crash chain and the delivery hunt

Branch: `feat/arena-single-address-space` (worktree `worktrees/a9f-descriptor-bridge`). Host: CachyOS, RX 7800 XT
(RADV), wine 11.21, KDE Wayland. Game pkg 1.0.2.1 on the SSD. This log covers the second half of the day (part 1:
`2026-11-26-fork-findings-loading-solved-and-world-init.md`). Commits: `fd2cc7406` (the campaign checkpoint),
`73fb5708d` (free path), `1e67cc42e` (test fixtures), `6e2bd2463` (trap/alloc harness), `36622bd01` (length-guard
harness); all pushed. LeoTux docs: `142cfb084`.

## TL;DR

The world-init corruption is **solved and committed**: it was never a race — the title's colour-ramp/LUT builder
does a deterministic unbounded 0x1000-byte memcpy into a block that ends 0xE10 bytes in, overrunning onto a
container object's vtable head. A latent title bug that only crashes in our allocation layout. The fix is the CRT
copy guard (`LEONIDA_COPY_GUARD=1`): clamp any CRT memcpy/memmove covering a live container head. Falsified against
a hardware watchpoint byte-for-byte; zero faults at the old death point for the first time ever. From there the
chain-walk went: free path (one free of a poisoned block, skipped as the title's own free(NULL) + page-map
save/restore) → the title's OOM hook (a bogus 0xFFFF4000 ~4 GiB allocation) → its producer (an unchecked
offset[i+1]-offset[i] in a tag-0x4B record deserializer, one out-of-order pair) → the table's source: **the archive
proves the 512-entry table is COMPLETE in the package** (offset[432] non-zero in the data) — so the record's arena
block was **delivered short** (~1.5 KB). That delivery is the current hunt.

## What was done

### The corruption hunt (LOAD-13, solved)
- Instrument ladder, each excluding a channel decisively: title-write ledger (37-109k writes/run, 0 overlaps),
  xmem-overlap ledger (WATCH clean; one big 605 MB title arena), serial dstorage / per-page commits / page-backed
  off / fence event-only / shared pool / **GE-Proton + ntsync** (corruption identical — wine-invariant).
- Hardware watchpoint arc: static watch (wrong, address drifts) → dynamic arm from the ledger's first fault
  (`LEONIDA_WATCH_FAULT_OBJECT=fault`, zero fires = corruption precedes fault 1) → multi-address watch on the four
  repeating object addresses (`=addr1,addr2,addr3,addr4`) → the writer caught: **host:ucrtbase.dll memcpy, title
  caller game+0x1550e613** (the LUT builder), after my fix to keep write slots armed across fires (trap-class; the
  clear+2s-rearm window was exactly where the corrupting write hid).
- Root cause by disassembly: the LUT builder copies 0x1000 bytes of ramp into a block ending at +0xE10; overrun
  crosses a live neighbour onto the container head, inside the walker's own call tree. Title-side, deterministic.
- Fix: copy guard at the import boundary (pe_loader.cpp, vcruntime140/ucrtbase memcpy+memmove), flag-gated
  (`LEONIDA_COPY_GUARD=log|1`, `LEONIDA_COPY_GUARD_ADDRS`), 2-compare hot path, ctest green.

### The free path (cleared, committed 73fb5708d)
- Next death: the LUT record buffer's free dies (three sites across guard variants). Double-free REFUTED three ways
  (idempotent release, zeroed slots, LUT bytes intact): ONE free of a block whose head + page bookkeeping the
  overrun poisoned.
- `LEONIDA_SKIP_FREE_FAULT=1` (free_skip.h): VEH with a fail-closed predicate (rip/read+0x18/rdi=0/rsi in arena/
  rbx = the arm's lock), releases the free's recursive lock in the title's own order, redirects RIP to the free's
  shared ret epilogue — provably the title's own free(NULL) path; one block leaks, 64-skip cap.
- Page tags: the arena tags every page 0x5FC0 at page_end-0x50/-0x10/0; the copy guard's TAGS mode saves/restores
  them around arena copies (page_end-0x50 = the range MAP, restored unconditionally — the range-mark at
  game+0x515AFC2 reads it back).

### The OOM trap and its producer (pinned)
- Title int3 at game+0x22106E0 = the shared OOM hook: a caller requests 0xFFFF4000 (~4 GiB). Trap reporter
  (`LEONIDA_TRAP_REPORT`) prints the automatic .text frame walk; alloc ledger (`LEONIDA_ALLOC_LEDGER`, exec-bp on
  the provider path) recorded 264k hits.
- Producer: `sub esi,[rax+r14*4]` at game+0x1D92FA8 — entry length = offset[i+1] − offset[i] UNCHECKED in the
  tag-0x4B record deserializer; entry #431 of 512 has offset[432]=0x0 < offset[431]=0xc033 → -0xC033 → +0x10,
  align, +0x20 header → 0xFFFF4000. All four values verified against the dump.
- The title's windowed reader turns "no more bytes" into zeros (the refill's short-read branch memsets and never
  errors) — the table parses 432/512 then zeros.

### The archive verdict (decisive)
- 161 GB of rpf scanned read-only (counts only): the 512-entry 16-bit table is **complete in the package** — six
  identical copies in data.rpf, one at the exact 0xF6D4 base the run parsed, 512/512 monotone, terminator 0xDD7F,
  offset[432] non-zero in data. No 512-entry 32-bit record exists.
- Verdict: NOT a data truncation. The arena block behind the reader's source was **delivered short** (source dry
  1,484 B before block end, 162 B before table end). The hunt is now: which request filled that block short
  (dstorage request/DestinationSize/completion) or whether its tail pages arrived wrong. Our dstorage queue is
  structurally exonerated (EOF answered properly, short inflate rejected, no memset, no failure lines).

### Sidelines closed
- d3d12_mesh_test/d3d12_draw_test regression: TEST-SIDE (4d86061b0 moved the user-SGPR layout to s8; the GLSL
  fixtures read old offsets). Fixed in 1e67cc42e; both pass 4096/4096. Bisect in a dedicated worktree; the
  first-parent linear-search method + configure recipe recorded in docs/testing.md.
- AS-slot-is-mesh CONFIRMED in-run: all four type-24 blobs are DXIL kind 13 (mesh), routed, built.
- dim_2d_array growth confirmed (0 "array full" lines) BUT the grown set coincides with a 15-20× slower bulk-load
  phase (mechanism unknown — the title then does the work the refusals skipped); `LEONIDA_BINDLESS_CUBE_GROWTH=0`
  is the fast-iteration setting, growth stays the correctness default.
- LOAD-6 negative result: the overlay holds only our own winedbg minidumps + a 2 MB preallocated telemetrycache.dat
  that nothing reads — the gate mechanism is NOT in the overlay content; recorded honestly.

### Harness family (all committed, flag-gated, default-off)
title-write ledger · xmem-overlap ledger · watch fault object (multi-address write watchpoints, dynamic arm,
stay-armed) · watch constructor (exec-bps) · copy guard (+ALWAYS/TAGS) · free-skip · trap report · alloc ledger
(+BIG bypass) · length guard / offset-table ledger / short-read (the last three implemented, uncommitted pending
build-verify). Debug-slot budget (4) composed across watchers with named overflow.

## Decisions made

- Clamp a title-side unbounded copy at the import boundary, never emulate a publish fault; survival emulation
  (RIP redirect) only for operations whose skip is harmless (a free). Clamp against the WHOLE overrun (stopping at
  the first victim exposes the next one crash site deeper).
- Gradual commits per fix after the campaign checkpoint; verify in-run before committing a harness.
- Subagents are throttled when the user shares the box: nice -n 19 ionice -c3 prefixes, halved parallelism,
  relief-first two-prong on impact reports (recorded in the project skill).
- Timing-sensitive runs (a source that may answer "nothing yet") need a quiet disk — an I/O-heavy scan during such
  a run flips the verdict toward "timing" falsely.
- The mesh-stage SGPR base gap (console mesh reads params from s16; our pipelines lay out at s8) stays OPEN as
  SCN-17: enable the dual placement when a title mesh draw is MEASURED reading wrong, not before.

## Pending items (in order)

1. **The deciding run** (`run-dirs/a9f-run/run-lengthguard2.sh`, disk-quiet): dual-arm both offset stores +
   `LEONIDA_SHORT_READ=1`. A short-read from the 16-bit fill caller at element 432 = chase the dstorage
   request/DestinationSize that filled the block short; zeros already in the block's bytes = the page delivery
   filled wrong. (Build-verify length_guard_test first, then commit the harness, then run.)
2. The delivery chase → the shim fix (likely dstorage request completion or the direct-read path's short/late
   answer under our slower-than-console speeds).
3. Continue the crash chain toward the main menu; SCN-17 when a mesh draw is measured.
4. Parked: the fair stock-vs-Proton load-speed A/B (scripts ready: run-geproton.sh), the cube-growth slowdown
   mechanism, LOAD-7 direct-read profiling, LOAD-8/9 wine-layer candidates.

## Changed architectural constraints

- **The title carries latent heap bugs our allocation layout unmasks** (unbounded copies, unchecked subtracts,
  silent zero-fill on dry sources). Expect one per layer; the pattern is guard/emulate surgically, never patch the
  title, never relax host rules without evidence.
- **The title's windowed reader converts "no more bytes" into zeros silently** — a data-integrity hazard: any
  short/late delivery becomes valid-looking data. Delivery completeness is a hard requirement, not a performance
  one.
- **Debug registers are a budget of 4 across all watchers** (write watchpoints, exec breakpoints): composed
  centrally, named per slot, overflow printed once.
- **Handler discipline**: no stdio/malloc/locks in a VEH/fault handler; stack buffer + WriteFile, IsBadReadPtr
  first, EXCEPTION_CONTINUE_SEARCH; write watchpoints stay armed across fires (trap-class).
- **Console flag/enum semantics ≠ host** (DS|UAV is normal; stream word 24 carries mesh): settle from evidence
  (the blob's DXIL kind), log every relaxation, keep host rules elsewhere.

## docs/ touched this half-day

docs/testing.md (second-worktree recipe), docs/discovery/TRACKER.md (living), docs/knowledge-base/logs/ (this
file). Reports: leonida-reviews/{crash-0x13360a3, corruption-writer, crash-0x115acf5, crash-free-path,
crash-0x22106e0, alloc-4gib, offset-table-source, archive-offset-table}.md (outside the repo by policy).
