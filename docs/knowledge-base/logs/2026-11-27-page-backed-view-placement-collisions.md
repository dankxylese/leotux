# Session log — 2026-11-27 — The page-backed view placement collisions (dig, then A9cl)

Branch: `feat/arena-single-address-space` (worktree `worktrees/a9f-descriptor-bridge`). Run analysed:
`run-dirs/a9f-run/run-2026-11-27-long3.log` (1 h, zero crashes, 512 presents, asset streaming continuous).
Part 1 (below, "the dig") changed no code. Part 2 ("A9cl, implemented") is the fix, uncommitted in the worktree.

## Part 2 — A9cl, implemented (gap placement)

`shims/d3d12_vulkan_device.cpp` unless stated. Nothing committed; binaries staged into `run-dirs/a9f-run`.

- `bool gap_placement = true` (`:1006`), `LEONIDA_GAP_PLACEMENT=0` turns it off (parsed at `:8428-8441`, one line
  printed either way at device creation). `LEONIDA_PAGE_BACKED_VB` behaviour is untouched: with it off,
  `ensure_title_range` returns at `:1213` before any of this.
- New helpers beside `find_title_range` (`:1041`): `title_range_covered_to` (`:1058`),
  `title_range_next_start` (`:1068`), `title_range_covers` (`:1075`), `mirror_owner_at` (`:1086`);
  `arena_free_extent_at` beside `arena_allocate_at` (`:3013`).
- `ensure_title_range` (`:1212`): exact-match early-out unchanged; then a union-coverage early-out (`:1269`),
  then one `place_span` lambda (`:1296`, the old placement body) called from either the exact path
  (`:1384-1393`, unchanged behaviour, for the A/B) or a gap loop (`:1399-1460`) that hops over mirrors, clips
  each piece to the next mirror's start / the arena free run / the ask's end / the mapping's end, skips gaps
  under 32 bytes (logged), and places one piece per span. A title-thread caller (`relocate_if_blocked == false`)
  places ONE piece and `queue_title_range`s the rest for the submission-thread drain (`:1466-1472`). The
  relocation of our own squatting heaps survives, now reachable from the gap loop too (`:1407-1427`).
- A partial placement does not poison the VA in the refused set (`:1483-1496`): the placed pieces keep
  answering and the missing span is retested once the VA leaves the 8-slot recent ring.
- Diagnostic: the refusal names the overlapping `TitleRange` (VA, size, refreshes, offset) instead of the old
  false "not one of the layer's descriptor heaps or mirrors" (`place_span` at `:1305-1320`); `explain_arena_offset`
  (`:2577`) stays heap-only because the DESCRIPTOR_WRITE diagnostic calls it without the lock, and says so.
- `kMaxTitleRanges` 4096 -> 16384 (`:1030`) with one reservation on the first placement
  (`reserve_title_ranges`, `:1038`) — the vector previously reallocated on append. Why 16384: the count
  **saturates** — the long3 run's 952 asks named 579 distinct VAs, first-seen VAs fell 299/174/106/0/0 per
  10 minutes, and the ring's whole named set is ~5 VAs per 128 KiB block over 27 MiB.
- Validation without a game run: the 952 asks of long3 were replayed through a faithful model of the new
  placement (same free-list and coverage rules) — **0 refusals in the scene phase (was 491/511)**, 579 ranges,
  14.8 MiB, no two ranges overlapping, and every ask's span fully mirrored at the end. The 24 remaining
  refusals are the "VA is not inside a mapping of the title's" asks (no pages to copy), unchanged.
- `runtime/CMakeLists.txt`: `d3d12_vulkan_device_test`, `d3d12_placed_heap_lifetime_test` and
  `d3d12_console_descriptor_heap_test` did not link in this tree at all (undefined `rt::xmem::mapped_extent` /
  `commit_physical_range`); the three targets now carry `shims/xmem.cpp shims/registry.cpp`. With that,
  `d3d12_vulkan_device_test`, `d3d12_placed_heap_lifetime_test` and `d3d12_descriptor_test` pass under wine;
  `d3d12_xmem_placed_test` and `d3d12_console_descriptor_heap_test` still exit 1 on a *pre-existing* assertion
  ("proxy CPU heap start mismatch"), identical with the device file stashed.

## Part 1 — the dig

## TL;DR

- **Cause: the per-view mirror window is wider than the spacing between the console VAs the title names.**
  A page-backed root view is given a fixed window at its exact console VA — 64 KiB for a root view
  (`kTitleRangeBytes`, `shims/d3d12_vulkan_device.cpp:967`) or 4 KiB for a tint/table ask
  (`:1697`, `:2081`) — and the title's CBV ring names *several* VAs inside each 128 KiB ring block, only
  0x60…0xD10 apart. Two windows therefore overlap in the address space, and a title range is **never
  released** (`arena_allocate_at`'s contract, `:2738`), so the first window placed at an address wins for the
  process lifetime and every later overlapping window is refused forever. `arena_allocate_at(:2748)` returns
  false the moment a live range starts inside the requested span.
- **Not a leak, not a full arena, not a stale VA→range table.** 432 ranges = 14.2 MiB of a 4 GiB arena (0.35%),
  zero "no room", zero "maximum number of ranges". The VA→range lookup is not stale — it is *exact-match only*
  (`find_title_range`, `:1022-1026`), and the title never re-names a VA exactly: 432 placements, 0 exact repeats.
- **Numbers.** 928 placement attempts: 432 placed, 496 refused (53%). Load phase (05:23:35–05:36:14):
  **412 placed / 5 refused (98.8% ok)**. Scene phase (05:36:14–06:16:26): **20 placed / 491 refused (96% refused)**.
  The 40% is the run average; in the phase that renders, page-backed root views are effectively 100% unbacked.
- **Content: the shader reads the arena at `heap_base + VA`. Where the union of live mirrors covers the address
  the bytes are the title's *current* bytes (correct, refreshed every submission); where it does not, zeros.**
  The hole is at the *head* of the refused window in 494 of 496 cases (median 3328 bytes) — exactly where a
  CBV's transform / a table's first records live. Log proof: heap census, refused tint base `0xd253ff20` reads
  four zero words at base+0x20 (log:570) while the placed sibling `0xd253dc40` reads `3f800000` (log:575).
- **Fix: place the gaps, not a window.** Make `ensure_title_range` coverage-aware (accept an ask the union
  already covers; otherwise clip to the free sub-spans around obstacles). No eviction: eviction frees nothing
  (0 "the title's pages are gone" lines — every mirrored VA stays mapped) and would only move the hole.

## The allocator's lifecycle

| stage | code | notes |
|---|---|---|
| ask | `back_root_views` `src/vulkan_replay.h:2204-2243` → `title_range_at(va, 0)` | per draw/dispatch, every submission; return value ignored (`:2222`) — the command stays recorded |
| wiring | `shims/d3d12_vulkan_device.cpp:8177-8181` | `relocate_if_blocked = true` (submission thread) |
| create | `ensure_title_range` `:1131-1261` | `arena_allocate_at(va, size)` at the *exact* console VA (`:1196`), copy of the title's pages (`:1239-1244`), `title_ranges.push_back` (`:1256`) |
| refresh | `refresh_title_ranges` `:1890-2054`, called from `set_before_execute` `:8146-8149` | re-reads each range from its own VA (or a page-alias sibling), `memcpy` only on change (`:2004-2021`); logs "the title rewrote these pages" once per range (`:2014-2018`) |
| destroy | **none** | no `title_ranges.erase/clear/pop` anywhere; no `arena_release` for a title range (release sites: `:1222,:1234` failure paths, `:2321,:3938,:4025,:5113,:5809` — heaps/resources/shaders only). Deliberate: `:2738` "the range is never released, so it stays out of every later allocation's way". |
| pages gone | `:1991-1999` | logs "the title's pages are gone; the arena keeps the last copy" — **0 occurrences in this run** |

Second lifecycle that is *not* the cause: `place_host_pages` (`:1272-1312`, offsets from the general freelist) had
zero calls this run, and the page-backed VB path (`src/vulkan_backend.h:1523`, 432 lines) allocates its own
`VkBuffer`, not an arena range.

## Why 40% collide — evidence

1. **Geometry.** Named VAs per 0x20000 ring block (low-19-bit histogram over the run): `0x1CF40` (96-record
   table, 4 KiB ask), `0x1DC40` (480-record table, 4 KiB ask), `0x1F270` (root view, 64 KiB ask), `0x1FF20`,
   `0x1FF80` (root view, 64 KiB). Windows: `[0x1CF40,0x1DF40)` swallows `0x1DC40`; `[0x1F270,0x2F270)`
   swallows `0x1FF20`/`0x1FF80`. Refusals concentrate on the losers: `0x1CF40` 345×, `0x1F270` 146×, vs
   `0x1DC40` 0× and `0x1FF80` 2×.
2. **Blocker age.** The range that blocks a refusal was created a median **1163 s** earlier (349 in 10–30 min,
   142 > 30 min, 5 < 60 s) — the ring laps in ~14 min, so these are the *previous lap's* windows, still resident.
3. **Obstruction shape.** 494/496: an existing range starts 0x60…0xD10 *above* the refused VA (the new ask
   starts below it); 2/496 the ask starts inside an existing range. In all 494 the refused VA's own first byte
   is unbacked.
4. **Load vs scene.** 412/417 asks succeed during load (fresh ring addresses, nothing to collide with) and
   20/511 during the scene phase (the ring recycles addresses the load phase already mirrored). The refusals
   are not the arena filling: `arena.in_use` is 14.2 MiB, and there is no "no room" line in the log.
5. **The equality lookup cannot help.** `find_title_range(:1022)` matches `r.va == va` only; the ring returns
   at a shifted offset, so the fast path never fires and the overlap is only discovered by `arena_allocate_at`.

## What the title sees after a refusal

`back_root_views` ignores the result (`vulkan_replay.h:2222`), so the draw is recorded and the translated shader
reads `heap_base + VA` → arena bytes at that offset:
- covered by a live mirror → the title's **current** bytes (every mirror is a linear copy of the same address
  space at its own VA, refreshed per submission) — correct by construction, so overlap is *content-benign*;
- not covered (the head sliver) → whatever was there: zeros on a fresh arena, never another VA's data.

Measured: census `0xd253ff20` (refused, 96 records) base+0x20 = `00000000 00000000 00000000 00000000`
(log:570), placed sibling `0xd253dc40` = `00000000 00000000 3f800000 00000000` (log:575). The tint-fill path
reads that same word back to build its descriptor (`:1697-1707`), so a refused tint base yields a zero colour.

## Recommended fix (file:line)

1. **Coverage-aware placement** in `ensure_title_range` (`shims/d3d12_vulkan_device.cpp:1131`):
   - replace the exact-match early-out at `:1153-1156` with a union-coverage query (new
     `title_range_covers(va, size)` beside `find_title_range`, `:1022`) → `note_recent_backed(va); return true`;
   - at `:1183-1195` clamp each placement to a *free sub-span* of the ask instead of the whole ask: walk gaps
     from `va` using the next range start above and the arena free block containing the cursor (add
     `arena_free_extent_at(offset)` beside `arena_allocate_at`, `:2740`), placing one `TitleRange` per gap
     (≥32 bytes, the existing guard at `:1194`); two or three entries per ring window.
   - keep the lock order (`:1002-1004`) and do the multi-gap version only for `relocate_if_blocked` callers
     (submission thread). The title-thread callers (`:3840` console descriptor heap, `:4286` imported buffer)
     stay single-placement + `queue_title_range` (`:1081`), per the no-alloc-on-the-title-thread rule.
2. **Fix the refusal diagnostic** (`:1203-1218`, `explain_arena_offset` `:2332-2353`): it only knows
   `heap_ranges`, so all 496 refusals print the false fallback "not one of the layer's descriptor heaps or
   mirrors - a buffer, a shader or a heap range holds it". Print the overlapping `TitleRange` (VA, size,
   refreshes) that is known under the lock at the refusal site.
3. **Rejected: eviction.** 0 "the title's pages are gone" lines — every mirrored VA stays mapped, so a
   liveness-based eviction frees nothing; evicting a still-needed window just moves the hole to its VA.
   **Rejected: shrink the window** — a 480-record table needs ~0x9000 bytes and is already under-covered at
   4 KiB; a 0x60-byte window would avoid the collision and lose the data.
4. **Follow-on ceiling:** `kMaxTitleRanges = 4096` (`:1018`) at ~0.14 new VAs/s is ~8 h; gap-splitting roughly
   doubles the entries per window, so the cap (or an LRU for ranges whose pages really are unmapped) needs a
   decision before a 10 h run.
