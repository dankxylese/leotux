# VIctory under Wine on Linux: what was learned (2026-11-26 and 27)

One PC: Ryzen 9 9900X (12 cores, 24 threads), 46 GB of RAM, Radeon RX 7900 GRE 16 GB
(RADV, Mesa 26.3.1), Arch Linux, kernel 7.3.6 (CachyOS), GNOME on Wayland with Xwayland.
Wine is Proton Experimental's (`experimental-11.0-20261112`, Wine 11.0, ntsync on), with
vkd3d-proton 3.2.0 and DXVK's dxgi copied into the prefix, started by `scripts/play.sh`.
Series X profile, ray tracing off, diagnostics build.

**In this tree** (the merge of PR #440 with avatarmanz's main and the Zeex64/GTA6-btw pieces,
`UPSTREAM.md`): the settings below are here under their names, but three of the files this
document's work changed are not this tree's versions, so the numbers measured with them are not
promises here. `victory-gpu/src/watch.rs` is this tree's (the title's views write-tracked by the
kernel, `VICTORY_VIEW_WATCH`; the guards held alone): renderer port 6's guards shared with the
faults and port 7's journal in parts are not in it — `VICTORY_PORT_JOURNAL` still turns on port
7's heaps' records (`crate::port_journal`), and its frame line says the journal's parts are
absent; `VICTORY_PORT_GUARD`'s planning is off. The PR's `title_view_space` is here, filled by
`wine_map`. `scripts/play.sh` and `scripts/play-1080p.sh` are here as the PR had them; this
tree's own launcher (`linux/play.sh`, and `scripts/play.ps1` as the run script sets it) keeps
the settings `UPSTREAM.md` lists.

Every figure is of one run of `measure.py` (see Tools; `scripts/play.sh --start`, 200 s, the
title's first scene from about second 50 on), read over seconds 100 to 200 with
`frame-stats.py`: medians of the report lines. (`--start` pressed Enter from second 12 in
those runs and the world's first frame came at second 33; in the runs of 2026-11-27 before
"heads-2" it pressed from second 52, and the same stretch of the scene is seconds 140 to
240 there.) Scenes are not the same from run to
run; read differences under about 5 % as nothing. Logs are in `~/Games/victory-logs/`.

## Where it stands

| Run | What | Frame (median) | fps | The card busy |
|---|---|---|---|---|
| 9 | before (a9400a6) | 441 ms (292 to 845) | 2.3 | 12 % |
| 20 | this tree, `VICTORY_WINE_MAP=0 VICTORY_PIPELINE_WATCH=0` | 359 ms (299 to 1,627) | 2.8 | 12 % |
| 11 | pages shown with the host's `mmap` (`VICTORY_WINE_MAP`, on under Wine) | 287 ms (243 to 314) | 3.5 | 18 % |
| 21 | workers watched, pages shown as views (`VICTORY_WINE_MAP=0`) | 279 ms (56 to 759) | 3.6 | 15 % |
| 19 | both (the default under Wine now) | **47.9 ms** (40.5 to 71.6) | **20.9** | 90 % |
| 26 | both, and renderer patches 2, 5, 6 and 7 (`VICTORY_PORT_DESCRIPTORS`, `_CBV`, `_GUARD`, `_JOURNAL`) | 48.2 ms (38.1 to 59.6) | 20.7 | 91 % |
| 27 | both, and `VICTORY_OUTPUT=1920x1080` | **39.2 ms** (37.2 to 55.0) | **25.5** | 78 % |
| 29 | as 27, the six programs converted and running (`VICTORY_SINGLE_LATCH`, on under Wine) | 38.8 ms (37.2 to 59.6) | 25.8 | 79 % |
| 30, 36 | as 29, on 2026-11-27 (runs of 240 s read over seconds 140 to 240, as all from here on) | 40.5 and 41.3 ms | 24.7, 24.2 | 80 %, 77 % |
| 33 | as 30, and renderer patch 1, its pages shown as private memory (`VICTORY_PORT_CONSTANTS=1`, `VICTORY_PORT_HOME`) | **33.4 ms** (32.6 to 41.8) | **29.9** | 80 % |
| 34 | as 30, and renderer patches 1, 2, 5, 6 and 7 | **33.3 ms** (32.5 to 38.3) | **30.0** | 82 % |
| 40 | as 34, the title let above its 30 frames a second (`VICTORY_FRAME_INTERVAL_MS=16.667`) | **31.2 ms** (27.3 to 51.8) | **32.0** | 91 % |
| 61, 69 | as 40, every title buffer's copy in the card's memory (`VICTORY_CARD_BUFFERS_MIN_KB=64`) | **27.4, 27.3 ms** | **36.6** | 95 % |
| 72, 73 | what `scripts/play-1080p.sh` sets: as 61 without patches 6 and 7, a constant faint light; and (73) the submissions joined in the world only | 27.1, 28.1 ms | 36.9, 35.6 | 95 %, 94 % |
| 55, 66 | the test tree (avatarmanz's main 014d52d merged), the five patches and above 30 (55); `play-1080p.sh`'s settings but nothing joined (66) | 35.5, 33.8 ms | 28.2, 29.6 | 96 % |

The README's figure on Windows with an RX 7900 XTX is 45 ms. What set the frame under Wine
on 2026-11-26 was what sets it there: the card (90 % busy) and the thread that submits (47 ms
a frame). With renderer patch 1 (runs 33 and 34) the frame is the title's own 33.3 ms, and
let above it (run 40) the card is the wall: 91 % of the frame with something of ours queued,
the submitting thread working 28.8 ms of the 31.2 ("Above the title's 30 frames a second").
The night of 2026-11-27 took the first scene at 1920x1080 from 40.5 ms a frame (24.7 frames
a second) to 27 or 28 (36): patch 1 made to work under Wine, the title's own limit lifted,
and the title's buffers' copies moved to the card's memory.

There were two causes, one behind the other: with either one left in, the frame is about
280 ms (runs 11 and 21). Neither was the cost of `GetWriteWatch`.

## Cause 1: every view mapped or unmapped walks a list in Wine's server

The title shows its physical pages one 64 KiB view at a time (`XMemMapPhysicalPagesScatter`
and the GPU's page-mapping copies): 2,000 to 5,500 `MapViewOfFile3` and `UnmapViewOfFile2`
a second in the first scene, with more than 50,000 views there (53,000 mappings in
`/proc/<pid>/maps`, 8 GB of shared memory).

Wine's server keeps a process's views in a linked list (`server/mapping.c`). `map_view`
walks all of it (`is_valid_view_addr` looks for an overlap), `unmap_view` walks it to the
view (`find_mapped_view`; new views are at the end). The calling thread holds ntdll's
`virtual_mutex` across the request (`virtual_map_section`, `unmap_view_of_section` in
`dlls/ntdll/unix/virtual.c`), and that lock is the one every `NtProtectVirtualMemory`,
`NtQueryVirtualMemory`, `NtGetWriteWatch` and every fault (`virtual_handle_fault`) of the
process takes too.

Measured:

- `tests/fixtures/wine-bench` (`bench1-proton.txt`): a map and unmap of one view takes 7 us
  with no other view there, 49 us with 10,000, 132 with 30,000, 253 with 60,000, 498 with
  120,000. Beside a thread that maps and unmaps, with 60,000 views there, a write fault
  opened by a vectored handler takes 195 us on another thread (2.4 alone), a protection
  change 48 us (1.5 alone), a refused `GetWriteWatch` 18 us (0.2 alone).
- In the title (run 10, `VICTORY_DSTORAGE_LOG`, seconds 70 on): 1,061 maps a second of 418 us
  and 1,017 unmaps of 319 us, 768 ms of every second in those calls on one thread.
- Runs 9 and 10, `/proc`: wineserver at 79 and 76 % of a processor (it has one thread); the
  whole process at 1.8 processors of 24; the thread that maps asleep in `read` on the
  server's reply for 31 % of the time; the submitting thread on one futex for 33 % of its
  (taken to be ntdll's lock: 0.2 % once the mapping calls were gone).
- The report lines of run 9: 176 ms a frame "asking about title memory", 171 ms waiting for
  the guards, 164 ms of texture refreshes, the submitting thread at work 253 ms a frame.

What was changed (`crates/victory-hle/src/modules/wine_map.rs`, `VICTORY_WINE_MAP=0` turns
it off): under Wine the server is not asked. Where the host would have a placeholder the
reservation is a plain one. A page is shown by mapping the section's file there with the C
library's `mmap` (`MAP_SHARED | MAP_FIXED`; the descriptor is the server's own, from ntdll's
`wine_server_handle_to_fd`: the same pages as any view of the section) and then committing
the range, which under Wine sets its books and the protection and nothing else. It is taken
away by `MEM_DECOMMIT`, which under Wine maps fresh inaccessible memory over the range.
Protection changes, queries and faults go through Wine as before. A query calls such a page
private, not mapped, so the GPU side is told where these reservations are
(`victory_gpu::title_view_space`) and takes what is committed there for a view. The title's
own `VirtualFree` of placeholders (it splits them: kind 0x8002) is done on pieces kept of
the reservation.

`mmap` is called in the C library, not as a system call of our own: Wine turns on syscall
user dispatch for every thread and lets system calls through only from the C library's
mapping; one made from our code would be taken for a Windows system call. The library is
found in this process's `/proc/<pid>/maps` (the process is found by a marker file it maps:
Wine's server opens the files, so `/proc/self` is the server) and `mmap` in its dynamic
symbols. A page of a section of our own is shown this way at the start and read, written,
protected and taken away; if any of that fails, pages are shown as views, as before.

After (run 11, same log): 1,223 maps a second of about 3 us and 1,305 unmaps of 7 us, 13 ms
of every second; wineserver at 0.2 % of a processor; "asking about title memory" 3.9 ms a
frame, texture refreshes 10.8 ms, the submitting thread at work 42 ms a frame (253 before).
The frame: 287 ms, and now the same from line to line (243 to 314; 292 to 845 before).

The xmem tests pass under Wine both ways (20 of 20). With views, and all tests of the crate
running at once, `a_title_thread_goes_on_using_the_rest_of_a_view` failed once.

## Cause 2: six pipeline workers ended, and their pipelines were waited for in every frame

With cause 1 gone the frame stood at 287 ms with the card 18 % busy and the submitting
thread at work 42 ms. The timeline (`VICTORY_TIMELINE`, run 13): of four frames two took 35
and 75 ms and two 490 and 483 ms, and in those 475 ms were one wait of 500.05 ms on a
`victory-list` thread that nobody ended: `pipeline::make::wait`, the half second a frame
that a first use waits for its host pipeline. The frame line said "6 being made" from the
scene's first second to the end of every run, and 2.0 draws a frame were dropped as "being
made".

Six of the 18 pipeline workers were gone (a dump of all threads, run 14: numbers 2, 5, 6,
8, 12 and 14 missing). With Wine's `err` channel on (run 15) six lines say why:
`virtual_setup_exception stack overflow ... (0x28020000-0x28021000-0x28220000)`, all within
6 ms, 32 s after the start, while a batch of compute pipelines is made. Wine ends a thread
whose stack has run out and says so only on that channel, which `WINEDEBUG=-all` hides;
Windows would end the process. The pipeline stays "being made" for good, every frame that
uses it waits its whole budget, and the worker is lost.

Where the stack goes: in vkd3d-proton (`d3d12core.dll`), which turns the shader into the
host's form on the calling thread. A dump taken as it happens (run 18) shows such a thread
with two code addresses of `d3d12core.dll` in the 32 KiB above its stack pointer and
nothing else. The same six programs end their worker with a stack of 64 MiB (run 16) and of
1 GiB (run 18) as with 2 MiB: it is not a matter of a larger stack. They are compute
programs `17829ac71aab4e8d`, `a1b20bc47e3c9597`, `afeea48bbb9913ad`, `e8c962358d862e3f`,
`559d9f0184e15fe3` and `32ed072b9f845048`.

What was changed (`crates/victory-gpu/src/pipeline/make.rs`, `VICTORY_PIPELINE_WATCH=0`
turns it off): under Wine a thread watches the workers. When one ends, the pipeline it was
making is given up (its draws and dispatches are dropped at once, as those of a pipeline
the host refuses), a warning names its shaders, and another worker is started in its place.
On Windows nothing is watched and nothing changes. `VICTORY_PIPELINE_STACK_MB` sets the
workers' stacks, for looking into this; unset, they are as before.

After (run 19): 47.9 ms a frame, the card 90 % busy, the process at 10 processors.

### Why the stack runs out, and the programs made to convert (`VICTORY_SINGLE_LATCH`)

Reproduced without the title or Wine (`~/Games/victory-logs/dxil-work/`, outside the
repository): vkd3d-proton 2576c0b4 has dxil-spirv 114e664e, and that commit built on Linux
ends with SIGSEGV on each of the six DXIL containers in 0.04 s, with a stack of 2 MiB, 8 MiB
or 1 GiB. The backtrace is 174,285 frames of `CFGNode::find_common_dominator` and
`CFGNode::recompute_immediate_dominator` calling each other with the same two nodes, entered
from `CFGStructurizer::flatten_subgroup_shuffles`. A build with asserts stops before, at "For
now, only support one back edge. DXIL seems to obey this."

Two things together bring it about:

- a loop that several blocks branch back to. Our lowering keeps the console's graph, so a
  console loop with two jumps back has two latches (DXC gives every loop one);
- a call of `WaveReadLaneAt` or `QuadReadLaneAt`, which makes dxil-spirv run that pass
  before its structuriser. The structuriser makes the back edges one first; the pass does not.

Of the title's 25,790 translations 624 have both: 41 compute programs (its traced shadows,
screen probes and reflections), all but one of which overflow, and 583 pixel programs, which
convert but, for 340 of 592 in the cache, to other SPIR-V than a dxil-spirv mended in that
pass gives. dxil-spirv's master (b540c8a2) converts all of them; a vkd3d-proton with it
needs none of what follows.

What was changed (`crates/victory-shader/src/emit/bitcode.rs`, `single_latches`): a program
that reads lanes gets one latch a loop before it is written, the blocks that branched back
going through it. `VICTORY_SINGLE_LATCH` (`emit/mod.rs`) says for which programs: 1 compute,
2 the others, 3 both; unset it is 3 under Wine and 0 on Windows, where the translation and
the cache's key stay as they were. With it all 25,790 convert with that dxil-spirv, and the
41 compute programs get host pipelines from the prefix's vkd3d-proton (`translate
--pipelines`). The cache keys on it: the first run under Wine translates everything again
(`scripts/play.sh --cmd translate --warm all --jobs 20` does it beforehand, 89 s).

In the title (two runs of 130 s with captures, `VICTORY_OUTPUT=1920x1080`): with
`VICTORY_SINGLE_LATCH=0` six workers end and Wine reports six stack overflows; without it
none. The mean brightness of the world's frames goes from 17.7 to 26.8 (of 255) and no frame
is black (5 of 68 were): lamps light people and the street ("How dark the picture is"). The
frame is as long as before: 38.8 ms (run 29) against 39.2 (run 27).

People other than the heroine have no head in about every other run, and that was first
laid to this change: the lamps' light shows it. It is as often so without it (below, "People
without a head"), so the switch stays on under Wine.

## The README's switches under Wine

The picture was looked at in each of these: `VICTORY_CAPTURE` saves frames as they are
presented (with `VICTORY_CAPTURE_WORLD`, every eighth of the world's), and their mean
brightness was taken (0 to 255).

- **`VICTORY_PORT_CONSTANTS=1` (renderer patch 1): a black picture, and why.** Every frame of
  the world is black (mean 0.2; 21 to 27 without it), with pages shown by `mmap` and with
  `VICTORY_WINE_MAP=0` alike. The log says the stretch of title memory was "opened as a host
  heap through a view of ours" with no complaint. Runs 22 to 24 had it on and measured 33.3 ms
  (the title's own limit) and 25.8 ms (`VICTORY_FRAME_INTERVAL_MS=16.667`) of black frames: of
  no use but to say the submitting thread needs about 24 ms a frame when the card has little
  to do.

  The cause, measured without the title (`heap-probe`, Tools; and the test
  `under_wine_a_stretch_at_home_is_opened_where_it_is_and_a_view_only_if_shared` of
  `crates/victory-gpu/src/port.rs`, which runs on the card): vkd3d-proton opens a heap on
  memory by importing it into Vulkan (`VK_EXT_external_memory_host`), and where the import
  fails it gives the heap memory of its own and answers as if it had not failed
  (`vkd3d_import_host_memory` in its `memory.c`: "Failed to import host memory, hr
  0x8007000e", a warning that `VKD3D_DEBUG=warn` shows). With RADV the import works for
  private memory (`VirtualAlloc`, write-watched or not) and fails for a view of a section. On
  a view the heap opens, a buffer is placed in it, the card reads zeros there and what it
  writes does not reach the view. The port opens its heap on a view of ours of the title's
  pages: the card read zeros for every constant of the title. (The kernel's driver takes
  memory by address only as one mapping of the process's own: "In 3 launches of 13 the
  card read zeros in patch 1's heap", below.)

  Measured there too: with a heap of 185 MB opened on private memory every submission takes
  0.05 ms longer (one small copy: 0.036 ms without, 0.083 with, whether or not it names the
  heap); and while one page of such a heap is read-only the kernel refuses every submission
  ("The CS has been rejected") and the device is lost.

  What was changed (`VICTORY_PORT_HOME=0` turns it off; `home_on` in `xmem.rs`, `HOMES` in
  `crates/victory-gpu/src/inplace.rs`, `open_at_home` in `port.rs`): under Wine, with the
  port on, the title's 32-bit graphics pages are shown as private memory at the one address
  the title maps them at (committed in the reservation, where every other page is its
  section's file; the section takes their bytes back when they are unmapped, and all of them
  for good if the title shows one at a second address). The heap is opened right there, with
  no view of ours; the guards leave that memory alone; and a stretch that is a view is
  refused where the card would read zeros, so that it is copied as before and not black. On
  Windows nothing changes. The tests pass under Wine (those of `xmem` with
  `VICTORY_PORT_CONSTANTS=1` set, and the one on the card).

  In the title (2026-11-27, at 1920x1080): the log says the stretch was "opened as a host
  heap where the title has it", `card_reads_it=Some(true)`; the picture is there (the frames
  kept of runs 33, 34 and 40: the street at night; people with their heads in 33 and 34,
  and not in 40, which is not this patch's doing: "People without a head"); no submission
  was refused in the launches with it. (In 3 of the first 13 the card read zeros in the
  heap: below, with what was changed.) The frame is the title's own 33.3 ms with patch 1
  alone (run 33: 33.4 ms where run 30 has 40.5) and with all five (run 34: 33.3 ms), the
  submitting thread working 29.0 and 26.3 ms of it (37.7 without): the 4,960 fills a frame
  "done on the card" from the host copy of that heap are 68, and four of the thread's steps
  (the title's bytes put under the card's writes, snapshots, copies in the card's memory,
  planning the fills) take 7.9 ms a frame in run 33 where they took 16.6 in run 30. Not
  looked at: unmapping (the title unmapped none of these pages in the runs), and scenes
  other than the first.
- `VICTORY_PORT_DESCRIPTORS=1`, `VICTORY_PORT_CBV=1`, `VICTORY_PORT_GUARD=1`,
  `VICTORY_PORT_JOURNAL=1`: the picture is there, and the frame is the same (48.2 ms in run
  26 against 47.9 in run 19 at 4K; 38.7 ms in run 32 against 40.5 in run 30 at 1920x1080).
  Under Wine patch 6 trebles the time all threads spend changing protections (7.7 ms a frame
  in run 30, 22.8 in run 32) and halves their waits for the guards (11.0 to 5.5 ms).
- `VICTORY_OUTPUT=1920x1080`: 39.2 ms a frame (25.5 fps), the card 78 % busy, the picture
  there (run 27). The largest gain left that was measured.
- `--console series-s`: not run (`play.sh` passes no arguments to `victory run`).

## The intro videos go forwards and back (`VICTORY_JOIN_PARTS`)

Seen by eye first: the opening videos "go forwards and back" at a steady 30 frames a second.
Measured on 100 frames in a row as they are presented (`VICTORY_CAPTURE_FRAMES`, each
compared with the six before it, `frame-repeats.sh`): three new pictures, then the same
three again, and so on (of 65 frames 33 new and 32 the picture of three frames before).

- It was there before this work: the same with `VICTORY_WINE_MAP=0 VICTORY_PIPELINE_WATCH=0`,
  and with or without `VICTORY_OUTPUT`.
- `play.sh --safe`: every frame new. Of what the fast way adds, one at a time:
  `VICTORY_JOIN_PARTS=1`: every frame new; `VICTORY_THIN=0`, `VICTORY_UPLOAD_AHEAD=0`,
  `VICTORY_UNDER_WIDE=0`, `VICTORY_LAND_ECHO=0`, `VICTORY_SHOW_AFTER=0`,
  `VICTORY_TEXTURE_NO_WAIT=0`: as before. So it is the joining of a frame's submissions
  (the videos' frames have three, two of them joined to the one before). Why was not looked
  for. Whether Windows shows it too is not known (the launcher there sets the same 8).
- The world's frames are not touched: 81 frames in a row of the opening cutscene, joined, are
  each a new picture (but the title card's, which is one picture).
- `VICTORY_JOIN_PARTS=1` costs about 3 ms a frame in the scene: 42.3 ms (23.6 fps) in run 28
  against 39.2 (25.5) in run 27, both with `VICTORY_OUTPUT=1920x1080`.

## How dark the picture is

At night the title's people and the inside of its cars are near black here. Looked at in the
opening cutscene, the same moments of it side by side (`VICTORY_CAPTURE_WORLD`):

- This is what `docs/fork-ray-tracing.md` says of the title without its rays: what no lamp or
  sun reaches directly is black. Nothing measured says Wine makes it darker than Windows; no
  picture from Windows was at hand to compare.
- `VICTORY_AMBIENT=2` (the launcher has 0.8): no difference to the eye. The estimate is
  made from what is on screen, which is dark.
- `--rt` (`VICTORY_BVH_BUILDER=1`): no brighter in these shots, and 61 to 82 ms a frame.
- `VICTORY_AMBIENT=constant:0.3`: people, faces and the car's inside are plain to see, and the
  night looks like an overcast day. `constant:0.08`: they can be made out and it is still
  night; the lamp-lit street is duller than with the estimate. A matter of taste, between
  the two.
- The six programs that end their workers are all traces of shadow rays, by their names in
  the title's archive: `CS_TraceShadowRays_Checkerboard` (shadowmask),
  `CS_LaserLightShadowRaytracing`, and `CS_RTDistantShadowTrace32`, `...TemporalUpsample4X32`,
  `...LeakPlug32` and `...LeakPlugTemporalUpsample4X32` (rtdirectionalshadows). On Windows they
  run and, with no structures, every ray misses; here they did not run at all, and that is
  what left the lamps' light out: with them converted (`VICTORY_SINGLE_LATCH`, cause 2) the
  same moments have lit faces, people and street, brightness 17.7 -> 26.8. The heads that
  are missing in about every other run were missing before it, in the dark (next section).

## People without a head, in about every other run

**Found and put right on 2026-11-27 ("The cause", next); what follows it is the search as
it went before, kept for what it ruled out.**

### The cause: a compute pass the title runs once was dropped

Two frames recorded in two runs at the same shot, one with the man's face (run 68) and one
without (run 75), run again without a screen (`victory replay`, docs/replay.md), and read
back step by step:

1. The draw of the face's skin is there in both (`MS_VS_PedTransformPreSkinD`, mesh
   `f841ad74ec6e7367`), at the same place of the same list. Its indirect arguments ask for
   94 groups of meshlets in the frame with a face and for 10 in the other; every other draw
   of that pass asks for the same in both.
2. The title's culling program (compute `9f017d9011377efa`) writes those counts. It keeps a
   meshlet if one of its vertices lies in the picture, and takes the vertices where the
   skinning program put them (title buffer 0xfc3be0000, the same place, +0x145100, in both
   runs).
3. The skinning program (compute `36e010b53acffd77`) has the same bone matrices, the same
   mesh and the same placing in both runs. Before the bones it takes each vertex of a face
   from what the title's face deformer (compute `970c4fbb3b017c0e`) wrote for the frame; in
   the run without a face that holds one and the same value for every vertex: the whole
   skin lies in one point.
4. The deformer's inputs are alike in both runs but for one: a table of 26,744 bytes that
   each character has (`title buffer ...+0x6878`). With a face it holds its entries; without,
   every word of it is 0xFFFFFFFF on the card, "no entry".

That table is filled once, on the card, when the character's face comes up. The layer drops
a draw or dispatch whose host pipeline is not made within half a second a frame
(`crates/victory-gpu/src/pipeline/make.rs`; "A pass the title runs only once and whose
pipeline takes longer is lost", its notes say, and passes over the screen have ten seconds
of their own for it). Under Wine no pipeline library was ever kept (below), so every run made
all of a world's pipelines at the world's first frames: with a line in the log for each
first use dropped, 709 to 772 compute programs lost theirs in each of five runs. In the
three of them with heads the face's programs were first used at the world's first frame,
for the heroine, and were made by the time the man came; in the two without (h04, h07), and
in none of the three, they and three more compute programs (`2a3522dae6b66c17`,
`512416d975ace48b`, `ebe4e53f031f5249`) were first used, and dropped, at the frame the man's
face came up, 111 to 114 frames into the world. Which of the three fills the table was not
looked for.

What was changed:

- **`VICTORY_COMPUTE_WAIT_MS`** (`compute_budget` in `pipeline/make.rs`; half a second
  under Wine, `0`: as before; on Windows as before unless set): the first use of a compute
  pass waits for its pipeline by a budget of its own, all of a frame's waits ending together,
  and its pipeline is made before the others'. In the runs below it was 10 s: no compute pass
  was dropped in any of them, and none waited a tenth of a second, vkd3d-proton's cache being
  there. With every cache empty (the layer's, vkd3d-proton's, Mesa's; runs 101 and 102, and
  UwUCammie's on pull request 440) 10 s froze the start: 95 waits of a second or more, the
  longest 10 s, some 25 s of frames of 5 s, a minute to a usual frame; half a second: none
  over 501 ms, 35 s. Faces were there in both, one run each; a pass that runs once can still
  be dropped in such a first run (798 compute first uses were, at half a second).
- **`VICTORY_LIBRARY_NULL`** (`open` in `pipeline/library.rs`; on under Wine, `0`: as
  before): an empty pipeline library is asked for with no address. vkd3d-proton refuses a
  blob of no bytes at an address that is not null, which is what an empty slice has: "no host
  pipeline library: pipelines are not kept" was in every run's log, and nothing was started
  early in the next run. With it the second run loads the library (315 kB), starts 1,776
  pipelines during loading and drops 7 first uses where the runs before dropped 400 to 630
  (runs e07 and e08).

Counted on frames kept of the shot where both raise their hands, 25 to 30 s into the world
(`VICTORY_AMBIENT=constant:0.3` to see faces by; lists `heads-1.txt` and `heads-2.txt`):

| Compute passes | The world's first frame | Runs | Without heads |
|---|---|---|---|
| dropped as before (`VICTORY_COMPUTE_WAIT_MS=0`), by turns with the next | 73 s after the launch | 5 | 2 (h04, h07) |
| waited for | 73 s | 9 | 0 |
| waited for | 33 s (Enter from second 12, where 22 runs of 45 were without) | 8 | 0 |

Of the runs of the night before whose frames show the man (runs 32 to 84, all at 73 s, none
with the wait) 6 of 18 were without heads: waiting a minute before the first Enter did not
put it right, as the table below had it from 13 runs. (Run 40, taken for whole above from its
early frames, is one of the six.)

The picture and the frame are as before with both: the frames of the world kept of a run
with them, set as run 52 was, are the same shots in the same light (mean brightness 32.9
and 33.0 over 818 and 816 frames, run 86), and the launcher's settings give 27.1 ms a frame,
37.0 frames a second (run 87; 27.1 to 28.1 before).

Not known: why the heroine's face came up at the world's first frame in some runs and not in
others; whether Windows has it in a first run, with no library yet. Draws are still dropped
while their pipeline is being made, 400 to 630 first uses in a run with no library: one that
draws once into something kept would be lost the same way (passes over the screen are not).

### The search before (2026-11-26)

In about every other run nobody but the heroine has a head: its skin is not drawn, from the
first shot of the opening cutscene that shows one (14 s after the world's first frame) to the
end of the run. Eyes, teeth, hair, glasses and caps are drawn, and the inside of the collar
and the wall behind show through; her head and everybody's arms are whole. The same command
line gave either. It was seen once lamps lit people (cause 2), and was there before: in a
near-black picture a head that is missing does not show, so every count below is of frames
brightened where they are dark (the first count, "none of 5 runs without the programs", had
taken dark runs for whole).

**What decides it is how soon after the launch the scene starts.** The title waits at a
screen that asks for the profile; from the Enter that answers it to the world's first frame
it is 21 s (the intro videos skipped, the menu, the loading), whenever it is pressed. Only
the time of the presses changed (`VICTORY_KEYS`), runs of 82 to 135 s:

| The world's first frame, after the launch | Runs | Without heads |
|---|---|---|
| 27 s (Enter from second 6; G1 to G4) | 4 | 4 |
| 33 s (from second 12, as `play.sh --start` did; every run of the next table) | 45 | 22 |
| 41 s (from second 20; H1 to H4) | 4 | 1 (H4) |
| 73 s (from second 52; Y1 to Y6, then `--start` itself, C5 to C11) | 13 | 1 (C6) |

So `play.sh --start` now presses from second 52. Who starts by hand does well to let a
minute pass before the first press. It is not all of it: C6 had no heads at 73 s, the one
run of the 13 (a monitor was switched 5 s after its launch, and Wine's `BadRROutput` is in
its log; whether that is why is not known). What it is that takes those seconds is not known. At
the profile screen the process is at rest from second 4 on: 0.1 of a processor, 0.1 MB read
a second (`busy-run.py`, run Z1, 85 s there); it opens no internet socket in its first 50 s;
the log has no line of its own between seconds 10 and 51. Nor is it known which of the
three matters, the profile's Enter, the menu's or the world's first frame: they moved
together.

Before that was found, with the world's first frame at 33 s in every run: counted in the
opening cutscene, runs of 85 to 130 s with `VICTORY_OUTPUT=1920x1080` (`capture-run.sh`; the
man's head beside the heroine's, about 1,160 frames into the world):

| What ran | Runs | Without heads |
|---|---|---|
| `CS_LaserLightShadowRaytracing` (the default under Wine since cause 2) | 26 | 14 |
| not it: `VICTORY_SINGLE_LATCH=0`, this tree, one batch (N1 to N6) | 6 | 3 (N1, N3, N5) |
| not it: `VICTORY_SINGLE_LATCH=0` earlier (L0, L4), the tree before the change (W1, W2, D3, D4), the program left out with `VICTORY_SKIP_PIPELINES` (B2) | 7 | 3 (L0, W1, D4) |
| `play.sh --safe`, the programs running (E1 to E6) | 6 | 2 (E2, E5) |

So it is not the six programs' doing, nor that of the launcher's faster ways, and
`VICTORY_SINGLE_LATCH` stays on under Wine: without it the heads are missing as often, in a
picture too dark to show it.

What `CS_LaserLightShadowRaytracing` does in that scene, from its translation and from title
memory read during four runs (`peek-run.py`, P1 to P4): the structure it is to trace (its
address in two root constants) is all zeros, the builder being off; its 48 bytes of lanes in
use are zeros; so every lane stores "nothing hit" (0x7f7fffff) in its slot of a title buffer
of 3,072 bytes at 0xd9e9c9620, 64 slots at first and up to 288 as lights come, and title
memory has them. The programs that light read that buffer (table `s0+120`); with the zeros
it held while the program never ran, no such light reached anything. The run without heads
(P4) and the three with are alike in all of it. The 64 KiB of title memory around, taken
20 s before the end, differ from run to run in some 1,100 words, 78 of them in P4 alone:
values of a scene in motion, and no buffer left at zeros in one kind only.

Looked at, runs without heads against runs with at 33 s, and not it:

- What the layer drops: the same in both (the builder's stages, 180 to 270 a frame; nothing
  "being made" past the start). Of the report lines' numbers none separates three runs from
  three (R1, F1, F2; R2, F3, F4) by half as much again.
- The title's draws and dispatches by program, in the operation lists of two frames of the
  shot (`VICTORY_CAPTURE_WORLD_ALSO=1560,1620`; F1, F2, P4 against F3, F4, P1, P2, P3): no
  set of programs is in every run of one kind and in none of the other. Whether the head's
  draw is there and draws nothing was not found out. (A frame's list is not one frame of the
  title's: its lists are recorded ahead, on several threads. Two runs against two had the
  wind's finer grid and the vehicles' plainer programs on one side only; with eight it was
  chance.)
- Copies of title memory that missed a write (`VICTORY_UPCOPY_CHECK=1`; four runs without
  heads, U1, Q2, Q5 and Q6, and one with, Q4): none stale in 28 or 29 looks at rest a run,
  350 MB compared in each. What else it says is the same in all five: of the 24 MiB at
  0xff69e0000, below the 36 MiB the skinning programs store vertices in, up to 1.4 million
  words stand on the card that are zeros on the CPU's side and in title memory, and "no
  shader was declared to write the buffer". Not looked into.
- Addresses that change from run to run: the 185 MB reservation's (0xa9c0000 to 0xb6e0000)
  and the traced structure's (0xdefea9900 to 0xdefeb0600) are spread alike in both kinds.
- Textures streamed in by the end of a run: 7.0 to 8.6 GB in both kinds.

One thing goes with it, and may be chance: where the title put that program's 48 bytes of
lanes, one of its small buffers handed out as its systems start. At the end of a 64 KiB page
(0xd9c10ffb0, 0xd9e53ffd0) in 20 of the 26 runs, 14 of them without heads; lower in the page
(0xd9e53fd20, 0xd9c7ede50) in 6, none without. Found in the first 11 runs; the 15 after them
have 2 of the lower kind. If it holds, the heads go with the order the title's systems come
up in, which the layer's speed at the start would decide.

Not known: what the title, or the layer, has not done yet in a run that starts too soon;
whether the head's draw is absent or empty; whether Windows shows it (`play.ps1 -Start`
presses as early as `play.sh --start` did).

## Above the title's 30 frames a second (2026-11-27)

Runs 30 to 79: 240 s each, read over seconds 140 to 240, the title rendering at 1920x1080,
ray tracing off. "The five patches" are renderer patches 1, 2, 5, 6 and 7. Several runs turn
one setting on and off every 8 or 10 s (`VICTORY_AB=<seconds>,<switch>,...`, which the log
sums up phase by phase; `ab-stats.py`, Tools): both sides in the same run and scene, where
two runs of the same thing lie 2 % apart (40.5 and 41.3 ms, runs 30 and 36).

### What the card says of itself

`measure.py` now writes down the card's own counters (sysfs and the process's `fdinfo`):

- Its "busy" count (`gpu_busy_percent`) reads 100 % in every run, and that is the launcher's
  doing: `VICTORY_CARD_AWAKE=1` (as `play.ps1` sets it) runs a program that counts on a
  compute queue so that the card never idles. With it off (run 39) the count is 84 % and the
  compute engine works 8 % of the time for the process, where it "works" 107 % with it; the
  graphics engine 76 % either way, which is what our own count says (77 to 80 %).
- Keeping the card awake neither costs nor gains here: 41.8 ms with it off for a whole run
  (run 39) against 40.5 and 41.3 with it on; by turns of 20 s in one run (run 38,
  `VICTORY_CARD_AWAKE=turns`, `turns-stats.py`) 40.9 ms on and 43.8 off, over 8 and 6 frame
  lines whose ranges overlap. Left on.
- The card draws 122 to 128 W at 30 to 32 frames a second, and 114 W where it is "busy" 94 %
  of a 52 ms frame (run 35, below): busy with waiting.
- The pixels are not where its time goes: with the title rendering at 1280x720 the frame is
  38.6 ms (run 37) against 40.5 and 41.3.
- The processors run at 5.3 to 5.4 GHz on every core during a run (5.66 at most): the
  frequency governor (`powersave`, `amd-pstate-epp`) is not holding anything back.

### Let above 30

(The opening animation and videos are made for 30: ticked at 60, 7 frames of 100 in a row of
the animation were the frame before again, and 58 of 100 of a video of 24 pictures a second
(run 98). `VICTORY_FRAME_INTERVAL_DRAWS=500` ticks at the title's own 33.3 ms until a frame
runs 500 draws, and again after thirty frames in a row with fewer: 98 new frames of 100 and
none twice, the world as before (run 100). `scripts/play-1080p.sh` sets it.)

With the five patches the frame was the title's own 33.3 ms (run 34). With
`VICTORY_FRAME_INTERVAL_MS=16.667` as well (run 40): 31.2 ms a frame (27.3 to 51.8), 32.0
frames a second, the picture there. The card then has something of ours queued 91 % of the
frame (its graphics engine works 84 % of the time), and the submitting thread works 28.8 ms
of the 31.2: both are close to being the wall.

### More of the title's buffers' copies in the card's memory

The host copy of a title buffer lies in the card's memory, and what shaders wrote in it is
looked for there, from a least size on (`VICTORY_CARD_BUFFERS_MIN_MB`, `_MIN_KB`): 4 MiB by
default on a card of 16 GB, 1 MiB on one of 24. Smaller copies lie in the PC's memory, and
the submitting thread snapshots and compares them: 96 MB a frame in this scene, 66 of them
in copies of 1 MiB and more. With the five patches and the title let above 30:

| Run | Least size | Frame (median) | fps | The submitting thread | Copies in the card's memory |
|---|---|---|---|---|---|
| 40, 50, 53 | 4 MiB (the default here) | 31.2, 31.5, 30.8 ms | 32.0 | 28.8 to 29.3 ms | 46, 655 MB |
| 51, 60 | 1 MiB (`VICTORY_CARD_BUFFERS_MIN_MB=1`) | 28.8, 28.5 ms | 35.0 | 24.0, 24.1 ms | 91, 733 MB |
| 61, 69 | 64 KiB, every title buffer (`VICTORY_CARD_BUFFERS_MIN_KB=64`) | 27.4, 27.3 ms | 36.6 | 23.6, 23.5 ms | 3,481, 972 MB |

The process holds 13.0 to 13.2 GiB of the card's 16 either way. The card has something of
ours queued 95 % of the frame with it (89 to 91 % without) and draws 131 to 134 W.

### The waits around our clamps of indirect arguments (`VICTORY_CLAMP_SYNC`)

avatarmanz's main (014d52d) clamps every indirect execution whose arguments lie in patch 1's
in-place memory alone, right before it runs (`VICTORY_PORT_CONSTANTS_ORDER`, for palms and
lights that came and went): 1,360 clamps alone a frame in this scene, 2 to 5 ms on Windows by
its README. Under Wine, in the test tree (main merged with this branch) with the five patches:
51.9 ms a frame (run 35) where this branch, without that order, has 33.3 (run 34); the
submitting thread stood waiting for the card 18.6 ms a frame, the card "busy" 94 % and
drawing less power than in run 34.

Read in the sources, then measured:

- A clamp is a dispatch of ours between two waits for every earlier shader write (a UAV
  barrier of no resource), and it moves its scratch buffer to unordered access and back.
  vkd3d-proton makes of each of the four a barrier whose source is every shader stage of
  the queue (`command.c`), and RADV of such a barrier a wait for all pixel work before it,
  taken at the front of its command processor where what follows reads indirect arguments
  (`radv_stage_flush`, `radv_dst_stage_to_acquire_point`, Mesa 26.3.1): the card runs empty
  twice a clamp. The log counts those waits: 170 clamps a frame on this branch, 1,480 with
  the order, and 800 more a frame for the title's own barrier groups.
- `VICTORY_CLAMP_SYNC` (Wine only, on there; `=0`: as on Windows; `narrow` in
  `crates/victory-gpu/src/indirect.rs`): the clamp's two waits are recorded as barriers of
  the newer kind (`ID3D12GraphicsCommandList7::Barrier`) that name what they wait for: the
  compute passes before the clamp and the execution that last read the scratch buffer, then
  the clamp's own dispatch. By turns in one run of the test tree with the order on (run 43,
  phases of 8 s from second 145 on): 34.0, 35.0, 40.4, 39.2, 32.6 and 35.1 ms with it, 48.3,
  57.5, 73.5, 46.0, 59.4 and 56.4 without (medians 34.0 and 56.4). No submission refused.
  Each clamp at a place of its own in the scratch buffer, so that it waits for no execution
  at all, gained nothing more (the same run) and was taken out again.
- On this branch, with its 170 clamps a frame and its batches, the same switch changes
  nothing that can be measured (run 41, by turns: 30.1 ms with, 30.2 without).
- Whole runs with it: the test tree with the order 35.5 ms a frame (run 55, 28.2 frames a
  second), this branch 30.8 (run 53): the order's clamps cost 4 to 5 ms here, as on Windows.
- The picture: some 800 frames of the world kept of a run of each tree with it (runs 52 and 54,
  every eighth frame of the opening cutscene; `sbs-52-54-a.png` and `-b.png` beside the
  logs) are whole: people with their heads, the street and its lamps, palms, cars. The two
  runs are not at the same moments frame for frame, and what the order is for (palms and
  lights that come and go for a few frames, in another scene) was not looked for.

### The title's own barrier groups cost nothing here

The host records a wait for every earlier shader write for each call of the title's
barrier groups, 800 a frame: the same kind of wait as around the clamps. Counted in the
first scene (`VICTORY_BARRIER_GROUPS_LOG=1`, diagnostics, run 50: the groups read as the
PC's structures): four barriers a call; a fifth of the calls name no barrier that follows
unordered access; one in twenty-five waits for compute passes by name; the rest name every
shader stage on both sides, as the title does throughout. Tried under Wine: no wait for the
first kind, a wait for compute alone for the second, and for the rest too where no draw of
the list since its last full wait has a shader that can store to a buffer or a texture.
Four calls in five were recorded lighter that way, and the frame was the same in both trees
(by turns in one run: 30.1 ms with, 30.2 without on this branch, run 53; 33.5 and 32.5 in
the test tree, run 55). These waits fall where the card has to finish anyway; the clamps'
fell between draws. The change was taken out again; the count stays.

### In 3 launches of 13 the card read zeros in patch 1's heap

Runs 42, 44 and 49 ran at 2 frames a second (500 ms a frame, 7 GB of title memory copied in
every one of them). In all three the port had opened its heap on the pages at home and the
card read zeros there, so the stretch was refused, as it should be ("the card reads zeros
in the heap the host opened on the title's pages" in the log, 0.4 s after the pages were
shown); the ten other launches with patch 1 of that night read the title's bytes.

The cause, read in the kernel's source and then measured without the title:

- The kernel's side of a heap opened on memory (amdgpu's `amdgpu_ttm_tt_get_user_pages`)
  takes the memory only if one mapping of the process's own holds all of it: it answers
  `-EPERM` if the mapping at the first byte is a file's, or ends before the range does. The
  first is why a view of a section never worked ("a black picture, and why", above). The
  second is why private memory worked only most of the time: the title maps this heap in
  five pieces, the runtime committed each as it came, and the kernel keeps pieces apart
  that were written to before what lies between them was committed (each has books of its
  own from its first write on). The title's threads write as soon as a piece is there.
- `heap-probe.sh repeat` (Tools; `heap-probe-pieces.txt` beside the logs): 185 MB reserved,
  committed in the title's five pieces and order, each written to before the next: the card
  reads zeros in 44 tries of 44, vkd3d-proton saying "Failed to import host memory" where
  it is asked to (`VKD3D_DEBUG=warn`). Committed in one call: the bytes, 12 of 12. The whole
  stretch committed in one call and then the five pieces as before, four threads writing
  all over it meanwhile: the bytes, 16 of 16.
- That is also what a read-only page in such a heap does (above): a protection change
  cuts the mapping in two, and the kernel asks again at every submission. (Not measured
  apart from the protection itself.)

Changed:

- `whole_home` in `crates/victory-hle/src/modules/xmem.rs`: before a section's first piece
  is shown at home, all of its stretch is made one mapping (one `mmap` of the host's, then
  committed). The pieces the title has not mapped yet can be read and written from then on,
  where they were out of reach.
- `open_at_home` in `crates/victory-gpu/src/port.rs`: if the card reads zeros all the same,
  the heap is let go and opened again, four times at most, and then the stretch is given up
  for good: copied as before, its pages guarded as views are (`home_given_up` in
  `inplace.rs`). Before, nothing could say what the title had written in pages the guards
  left alone, and every copy of them was taken whole at every refresh.
  `VICTORY_PORT_HOME_FAIL=1` (diagnostics) has every try read zeros, to see what a stretch
  given up costs: 39.4 ms a frame (run 58), what the scene takes without patch 1.

In the ten launches with the change that were not made to fail or to record (runs 52 to 57
and 60 to 63) the heap was opened at the first try and the card read the title's bytes.

### Where the processors' time goes now (`VICTORY_PROFILE`, run 63)

With patches 1, 2, 5, 6 and 7, copies of 1 MiB and more in the card's memory and the title
let above 30 (29.1 ms a frame while sampled every 5 ms; `profile-sym.py`, Tools), by where
each thread stood when it was looked at:

- The submitting thread: parked 27 % of the time (`NtWaitForAlertByThreadId`: waiting for
  its helpers or for work), 28 % in our code with no function over 3.3 %, 8.6 % in
  `NtGetWriteWatch`, 4.2 % in `NtQueryInformationThread` (the diagnostics' `GetThreadTimes`
  twice a submission, `copystat::submission_begins` and `_ends`: a call to Wine's server
  each; not there in a build without diagnostics), 3 % in `NtProtectVirtualMemory`.
- The eight threads that translate list calls: parked 41 % each, 40 % in our code, 8 % in
  vkd3d-proton, 4 % in Wine's Vulkan thunks.
- The title's thread that presents: 73 % in two waits, 20 % in the title's code.

No thread is the wall: the card is (95 % of the frame with something of ours queued).

### What the card's time is made of (`VICTORY_GPU_TIMING=1`, run 67)

Each list sent and waited for on its own (127 ms a frame that way; patches 1, 2 and 5, every
title buffer's copy in the card's memory): of the 65.7 ms a frame the card then takes for
312 lists, the title's 218 lists take 49.5, our 63 preludes (what changed in the title's
buffers sent up to their copies) 9.7, and our 31 lists of compares (what shaders wrote in
those copies, looked for on the card) 6.5. A quarter of the card's time is ours, in the
passes that stand for the snapshots and compares the processor did before. A list takes
83 microseconds at the least from its sending to its signal.

### The frame recorder under Wine (`VICTORY_RECORD`)

It faulted at the first submission (runs 47, 48 and 59: a read of 0x2b20 in d3d12core, from
`replay::record::execute`). The recorder gives each list a method table of its own, and
vkd3d-proton knows a list of its own by its table: `d3d12_command_list_from_iface` answers
null for another, and `ExecuteCommandLists` read through that. Under Wine the lists now go
to the host under their own tables and get their shims back after
(`crates/victory-gpu/src/replay/record.rs`). Run 68: a frame of the world captured at
second 200 (77 submissions, 375 lists, 71,442 resources, 6.8 GB of the card's memory;
8.5 GB on disk in `rec/fast`), the title standing still 8 s and going on after. Its notes
say what a replay will miss: the 686 waits of the newer kind around the clamps
(`VICTORY_CLAMP_SYNC`; the recorder does not record that call), so a capture meant to be
replayed is better made with `VICTORY_CLAMP_SYNC=0`.

### The captured frame run again (`victory replay`, without the title)

`replay.sh` and `replay-set.sh` (Tools) run a capture again under Wine in a headless
gamescope, in four seconds a try, with loops that agree to a tenth of a millisecond. The
frame of run 75 (patches 1, 2 and 5, every title buffer's copy in the card's memory, the
clamps' waits as on Windows; 74 submissions, 340 lists; `rec/legacy`, outputs in
`replay-legacy.*.txt` beside the logs):

- Sent without a wait between submissions it takes the card 25.0 ms (the run it is from has
  27 to 28 ms a frame): the card is the wall, and this is how far the frame is from it.
- By kind of command (`--timing detailed`; shares of the list, as draws overlap): the
  title's 1,540 dispatches 6.2 ms and 1,053 indirect ones 2.8, its 2,233 indirect mesh
  draws 3.0, 535 draws 2.6, 2,225 barriers 2.3; ours: 590 compare passes over copies in the
  card's memory 1.7, 181 argument clamps 1.3, 207 copies from the PC's memory 1.1. No
  pipeline of the title's takes more than 0.5 ms; 30 of them together 5.
- All buffers in the PC's memory (`--buffers sysmem`): 65.5 ms. Where the buffers lie is what
  the card's time hangs on. Moving what is left in the PC's memory to the card
  (`--buffers vram`, all, or only those under 64 KiB) makes it slower, 27.4 to 38.8 ms, with
  13.7 of the card's 15.4 GB in use: there is no room for more on a 16 GB card.
- Left out or changed: our hash and compare passes -0.6 ms (`--skip-hash-passes`); every
  unordered-access barrier -1.1 ms (`--no-barriers`); barrier calls joined, nothing
  (`--merge-barriers`); one submission of all the lists -1.2 ms (`--one-submit`); a wait of
  half a millisecond after each submission 62.6 ms, and 58.9 with the card kept awake
  (`--pace 0.5`, `--awake`).
- The driver's and vkd3d-proton's options, each within 0.5 ms of the 25.0: `RADV_PERFTEST=`
  `cswave32`, `pswave32`, `gewave32`; `RADV_DEBUG=` `nohiz`, `nodcc`, `nonggc`, `nobinning`;
  `VKD3D_CONFIG=` `no_upload_hvv`, `memory_allocator_skip_clear`.
- `--skip-clamps` does not run on this capture ("clamp capture has no completed scratch
  transition in its command list").

### The unit tests under Wine after these changes

In a headless gamescope (the card is needed, no screen): the `xmem` tests of `victory-hle`,
18 of 18 with `VICTORY_PORT_CONSTANTS=1` and without, the one of pages at home now mapping
its pieces with writes in between and asking the host's own list of mappings whether the
stretch is one. Of `victory-gpu`'s 468, the same seven fail at this branch's last commit
(70cf2c8) and with these changes: three of `watch` (holes and mappings between askers),
`descriptor::host` (views made beside the heap), `host::stamps` (a submission's times),
`pipeline::library` (a rejected blob) and `timeline::report` (who ended a wait); and
`indirect`'s wide dispatches (over 65,535 groups) fails or passes from one run to the next,
before as after. None was looked into.

### Other things by turns, on this branch with the five patches

- `VICTORY_JOIN_BARRIERS` (off by default): 30.9 ms with, 31.1 without (run 45).
- Renderer patches 6 and 7 (with 1, 2 and 5 on; three phases of 8 s each): neither
  30.7 ms, patch 7 alone 30.7, patch 6 alone 31.8, both 32.7 (run 56); and with every title
  buffer's copy in the card's memory 24.5, 25.9, 26.4 and 27.8 (run 71), while the scene's
  own load moves the frame by 5 ms: nothing gained by either under Wine, where patch 6
  trebles the protection changes (above). `play-1080p.sh` has 1, 2 and 5.
- A frame's submissions joined by eights or not (`VICTORY_JOIN_PARTS`, five phases of 10 s
  each side): with copies of 4 MiB and more in the card's memory 31.2 ms joined, 31.7 not
  (run 62); with every title buffer's copy there 25.9 ms joined, 28.7 not (run 70), and whole
  runs not joined 30.9 and 30.2 ms where joined ones have 27.4 and 27.3 (runs 64, 65, 61, 69):
  unjoined, the submitting thread flushes its refreshes 61 times a frame where it does 35
  times joined, and works 30 ms where it works 24. Joined, the intro videos show three new
  pictures and then the same three again ("The intro videos go forwards and back").
  `VICTORY_JOIN_PARTS_DRAWS=<n>` (`join_from_draws` in `crates/victory-gpu/src/host.rs`,
  unset by default) joins only after a frame that ran `n` draws or more: the videos' and
  the menus' frames, of a dozen draws, go part by part, and the world's joined. With 500
  (run 73) against joined throughout (run 72), 95 frames in a row of the intro videos at two
  moments, each set beside the six before it (`frame-repeats.sh`): joined throughout, 42 of
  the first 95 are the picture of three or four frames before, and 9 of the second 95 that
  of five before; with the switch none of the first 95 and one of the second. The world's
  frame is 28.1 ms with it and 27.1 without, the submitting thread working 23.4 and
  23.6 ms.
- The submitting thread alone on one core (its two processors, every other thread of the
  process kept off them; `pin-turns.sh`, Tools; run 57, five phases of 10 s each side):
  29.8 ms with, 30.4 without, the means the same (31.4): nothing.

## FSR 3.1 and 4 through AMD's PC library (the test tree, 2026-11-27)

avatarmanz's main has a profile that hands the title's upscaling to AMD's PC library
(`VICTORY_FSR_4K=quality|balanced|performance`, `VICTORY_FIDELITYFX_DIR`): 3840x2160 put out
from a smaller picture. It is not on this branch; tried in the test tree (main 014d52d
merged), with what this PC had already:

- `amd_fidelityfx_upscaler_dx12.dll`, signed, from another game's folder; Proton
  Experimental's `amdxc64.dll` (in the prefix since it was made) and its
  `contrib/amdxcffx64.dll`, copied into the prefix's `system32` as Proton does for its own.
- The bridge asks for version 4.1.2 and nothing else; here the library offers "4.1.1 *,
  3.1.5, 2.3.4" (Proton's `amdxc64` logs "Device supported" for this card and "Replaced FSR3
  with FSR4!"), and the title ended at once (runs 76 and 77). `VICTORY_FSR_VERSION=<version>`
  (`wanted_version` in `crates/victory-hle/src/modules/fidelityfx.rs`; in the test tree only,
  not committed) asks for another.
- 3840x2160 from 1920x1080 (`performance`), patches 1, 2 and 5, the title let above 30:
  FSR 3.1.5 43.5 ms a frame (run 78, 23.0 frames a second); FSR 4.1.1 with
  `DXIL_SPIRV_CONFIG=wmma_rdna3_workaround` (vkd3d-proton's, for this generation of cards,
  which has not the matrix instructions FSR 4 is made for) 47.4 ms (run 79, 21.1) and 56.1
  (run 80, with frames kept). The log has "upscaler context on the host provider=4.1.1" and
  the title's dispatches recorded; the frames of the world kept of run 80 are whole pictures
  (`sheet-80-fsr4.png` beside the logs). No submission refused, no hang, in three launches.
- On a monitor of 1920x1080 this gains no frame: the picture is made at 1920x1080 where
  `VICTORY_OUTPUT=1920x1080` makes it at 1280x720, and put out at four times the monitor's
  pixels.
- Putting out 1920x1080 (the test tree only, not committed; the changes are kept as
  `patches/fsr-output-and-version.patch` beside the logs): with a profile `VICTORY_OUTPUT`
  is applied as without one, the picture made at the preset's share of it, and a preset
  `native` makes it at the output's own size. The launcher's settings, main's order of
  clamps off as on this branch (`VICTORY_PORT_CONSTANTS_ORDER=0`), runs 81 to 85:

  | Upscaler | Picture made at | Frame (median) | fps |
  |---|---|---|---|
  | the title's own | 1280x720 | 27.3 ms | 36.6 |
  | FSR 4.1.1 `quality` | 1280x720 | 27.7 ms | 36.1 |
  | FSR 3.1.5 `quality` | 1280x720 | 27.6 ms | 36.2 |
  | FSR 4.1.1 `performance` | 960x540 | 26.3 ms | 38.0 |
  | FSR 4.1.1 `native` | 1920x1080 | 32.8 ms | 30.5 |

  All five ran to their end. Whether FSR 4's picture is better than the title's own or
  than 3.1's was not judged; in the one frame looked at of `performance` the heroine's face
  is white, which was not looked into.

## What the calls cost under Proton's Wine

`tests/fixtures/wine-bench`, microseconds a call, one thread and eight at once (each on
memory of its own):

| Call | 1 thread | 8 threads |
|---|---|---|
| `GetWriteWatch` with reset, 64 KiB, nothing written | 0.37 | 4.4 |
| `GetWriteWatch` with reset, 64 MiB, nothing written | 5.6 | 21 |
| `GetWriteWatch` with reset, 64 MiB, 256 pages written | 9.5 | 86 |
| first write to a write-watched page after a reset | 0.18 | 0.33 |
| `GetWriteWatch` of a view (refused) | 0.20 | 0.90 |
| `VirtualQuery` of a view | 2.8 | 12 |
| `VirtualProtect` of 64 KiB, read-only and back (the two) | 3.0 | 33 |
| a write to a read-only block, opened by a vectored handler | 2.4 | 44 |
| `MapViewOfFile3` into a placeholder and `UnmapViewOfFile2` (the two), few views | 7.5 | 77 |

Asking about 1 GiB with all of it written takes 2.1 ms, with nothing written 0.09 ms.
Proton's write watch is the kernel's (userfaultfd with `UFFD_FEATURE_WP_ASYNC`, and
`PAGEMAP_SCAN`): a call is cheap. Every call takes the one lock and masks signals around
it, so eight threads go one at a time (9 to 18 times the cost of one). That is true of the
title's frame too, and was not what made it slow: it became so when one thread held the
lock for a third of a millisecond at a time, 2,000 times a second.

## Measured and not the cause

- The implementation of `GetWriteWatch`: Arch's Wine (run 4, write watch by faults) and
  Proton's (run 7, the kernel's) gave the same frames, and under Proton's a call costs
  microseconds (the table above). The 1 to 25 ms "askings" of run 7 fit waits for the lock
  behind the mapping calls: in run 11 the askings of a frame take 3.9 ms in all. Arch's
  Wine was not run again.
- A larger stack for the pipeline workers (above).
- `VICTORY_CARD_AWAKE`, MangoHud, the swap: not looked at; none of them showed in what was
  measured (no swap in use by the process in any run: `VmSwap 0.0 GB`).

## What is not known, and what is next

1. People without a head: found and put right ("The cause: a compute pass the title runs
   once was dropped"). Left of it: draws dropped while their pipeline is being made could
   lose a pass that draws once the same way; whether a first run on Windows has it.
   (`--start` presses from second 12 again.)
2. The card hung in run C11, 31 s into the world, with nothing set but the output size, the
   ambient constant and the monitor: `amdgpu: ring gfx_0.0.0 timeout` naming the title's
   `vkd3d_queue` thread, "Illegal opcode in command stream", and the ring's reset failed
   (kernel log, 2026-11-26 21:03:53). The desktop went with it: gnome-shell lost its context
   and ended the session. Once in some 110 runs of that day, and not once in the 50 launches
   of the night after; not looked into. Whoever runs batches on a desktop in use should know.
3. Wine's server could keep views in a tree; that is theirs to change. With it, the
   Windows way would do under Wine as well.
4. The frame's "0.0 on the GPU": not looked at. "The card about 90 % busy" is our own
   count of the time something of ours is queued on it.
5. `VICTORY_PROFILE` and `VICTORY_STACKS` under Wine: unwinding another thread's stack
   faults in Wine's `RtlVirtualUnwind` (run 12 ended the title as the profile began). Under
   Wine they now give the thread's instruction and the words of its stack that point into
   images, nearest first: enough to see where a thread is, not a true chain of callers.
6. Renderer patch 1 with its pages shown as private memory ("The README's switches under
   Wine") ran in the first scene only. Not seen yet: the title unmapping or mapping again
   pages of that heap (the heap opened on them would outlive them), and whether anything
   makes a page of it read-only in another scene (the device would be lost: "The CS has been
   rejected" in the log; none in the runs so far).
7. The picture against Windows: the frames looked at are whole pictures (the title cards, the
   street at night with its lamps, cars and people, subtitles). People away from lamps are
   near black; whether that is the README's known darkness or more than it was not compared.
8. Renderer patch 1 on this branch has not what main added for it
   (`VICTORY_PORT_CONSTANTS_ORDER`): by main's README, palms and trees drop out for a few
   frames and lamp-lit rooms swing between dark and bright without it. Not looked for here
   (the first scene's frames are whole). It comes with the merge of main, at 4 to 5 ms a
   frame with `VICTORY_CLAMP_SYNC` ("The waits around our clamps").
9. The card is the wall at 25 ms a frame, and its time is the title's own four thousand
   small dispatches and draws, none over half a millisecond ("The captured frame run
   again"); ours is a sixth of it (compares of copies in the card's memory, clamps, copies
   from the PC's memory). Nothing found that makes it shorter by a setting: not the pixels,
   not the driver's options, not the barriers. Next there: fewer and wider compare passes;
   what the 1,540 dispatches are; a card with room for every buffer.
10. Everything of 2026-11-27 was measured in the title's first scene, a cutscene at night.
    Not in play, not by day, not in traffic.

11. Lips did not move when people spoke: the heroine's mouth was shut in 21 frames in a row
    of three of her lines, eyes and head moving (run h02). In the frame recorded in run 68 the
    face deformer's shared input (`title buffer 0xda62c8580+0x104000`, 1 MiB) was all zeros on
    the card and in the copy of title memory, and nothing wrote it in the frame. Not the
    heads' cause (there the card and title memory differed).

    Put right on 2026-11-27 with three changes of others, taken together from
    bordersky/victory-linux (its tree at 2294b83 against avatarmanz main 9576c45, the
    translator's and the descriptors' files only): avatarmanz pull request 114 (secure-silicon:
    d16 accesses ask for the kind of a view in a table too), fxceless/victory-fork pull
    request 927 (tippythetaidum: halves moved through a 16-bit buffer view keep their bits),
    and the raw typed loads (`VICTORY_RAW_TYPED_LOADS`, on; `0` off). With them the lips follow
    the lines and close between them, and eyes blink (run 117, one run of 100 s, the frames
    of three lines looked at); the frame is as long as before, 28.9 ms against 28.8 over the
    same stretch (runs 117 and 108). The translations change (`TRANSLATOR_VERSION`, the shader
    cache's format): every shader is translated again at the first start, or with
    `scripts/play.sh --cmd translate --warm all`.

    What was tried before, each one run, on this card (RX 7900 GRE, RADV 26.3.1):
    - pull request 114 alone: faces deform, mouths wide open and the same in every frame
      (run 103); with avatarmanz pull request 329's shader commit as well: mouths change but
      stay agape, with specks by the nose (run 104).
    - with those two: d16 typed loads and stores of an integer view through a handle of
      integers (run 105), every shader translated for waves of 32 (`VICTORY_HOST_WAVE=32`,
      run 106), none of the float shortcuts for RDNA (`VICTORY_RDNA_FLOAT=0`, run 108): the
      same picture each time.
    - not the driver's compiler: a recorded frame's network replayed with RADV's ACO and with
      its LLVM backend (`RADV_DEBUG=llvm`) gives the same result word for word.

## Tools

In `~/Games/victory-logs/tools/`, outside the repository (its policy takes no Python files):

- `measure.py <name> [--seconds 200] [--from 100] [KEY=VALUE ...]`: one run, its log, and what
  `/proc` says of the process's threads over the window (processor time of each, and what
  they sleep in, 50 times a second), and of wineserver; and of the card: how busy it says it
  is, the watts it draws, how long each of its engines worked for the process and the memory
  the process holds on it. `VICTORY_REPO=<checkout>` in the environment: another checkout's
  `play.sh`.
- `perf-batch.sh <list>`: measured runs one after the other (`--seconds 240 --from 140`,
  `VICTORY_SCREEN=HDMI-1`), a line each: `<name> [REPO=<checkout>] [SIDE=<tool>] [KEY=VALUE
  ...]`; a dozen frames of each are kept (`cap-<name>/`), and it stops if the kernel's log
  says the card hung. `perf-1.txt` and `perf-2.txt` beside the logs are the lists of runs 30
  to 51 (`perf-3.txt` to `perf-6.txt` those of runs 52 to 79), `perf-<n>.out` what they printed.
- `ab-stats.py <log> [from]`: a run with `VICTORY_AB=<seconds>,<switch>[,<switch>]` (the
  layer's own: switches on and off by turns in one run, diagnostics builds), its phases by
  what was on. `turns-stats.py <log> [from] [card log]`: the same for
  `VICTORY_CARD_AWAKE=turns`.
- `pin-turns.sh <log>` (as `SIDE=pin-turns.sh` of a batch line with
  `VICTORY_AB=10,VICTORY_NOTHING`): in every other phase the submitting thread has one core
  to itself, set from outside with `taskset`.
- `card-log.sh <file> [seconds]`: the card's watts, clock, busy count, memory in use and
  temperature every second.
- `warm-set.sh [KEY=VALUE ...]`: every shader translated into the cache with those settings
  (a switch that is part of the cache's key, `VICTORY_PORT_CBV=1`, needs its set before a
  measured run), in a headless gamescope: no screen is touched.
- `profile-sym.py <profile> <victory.exe>`: a `VICTORY_PROFILE` file by function
  (`llvm-symbolizer` reads the build's `victory.pdb`; the exe must be the one that ran).
- `replay.sh [KEY=VALUE ...] -- <capture folder> [switches]`: `victory replay`
  (docs/replay.md) of a frame recorded with `VICTORY_RECORD`, in a headless gamescope: no
  screen is touched; not while the title runs. `replay-set.sh <capture> <out prefix>`: ten
  of them in a row.
- `frame-stats.py <log> [from]`: medians of the report lines.
- `profile-top.py <file>`: a `VICTORY_PROFILE` file by thread.
- `capture-run.sh <name> <seconds> [KEY=VALUE ...]` and `brightness.py <folder>`: a short run
  with the presented frames saved, and how bright they are.
- `frame-repeats.sh <folder> <first> <last>`: which earlier frame each captured frame repeats.
- `busy-run.py <name> <seconds> [KEY=VALUE ...]`: a capture run with, every second, the
  processors the title's process used, the bytes it read and its busiest threads.
- `peek-run.py <name> <seconds> [--also <address>+<length> ...] [KEY=VALUE ...]`: a capture
  run during which stretches of title memory are read from outside (`/proc/<pid>/mem`, as an
  ancestor of Wine's process) and written down each time they change: the buffer
  `CS_LaserLightShadowRaytracing` stores in, the structure it traces, its lanes in use.
- `side-by-side.sh <capture A> <capture B> <out.png> <offset> ...` and `sheet.sh <capture>
  <out.png> <first> <last> <step>`: the same moments of two captures side by side, and a
  contact sheet of one (offsets in frames after the first of the world).

- `heap-probe/` and `heap-probe.sh [megabytes]`: a program of 250 lines that opens host heaps
  on private memory and on a view of a section and asks the card what it reads there (no
  argument), or says what a heap of that many megabytes costs each submission and what a
  read-only page in it does; `repeat <rounds> <writers> <wait ms> <mode>` opens the title's
  185 MB committed in one call or piece by piece, again and again, and counts what the card
  reads. Run in a headless gamescope: no screen is touched. Its outputs are
  `heap-probe-views.txt`, `heap-probe-185mb.txt` and `heap-probe-pieces.txt` beside the logs.

In the repository:

- `tests/fixtures/wine-bench`: the table above; build with
  `cargo xwin build --release --target x86_64-pc-windows-msvc` in its folder, run with Wine.
- `WINEDEBUG=err+all,fixme-all` as a `KEY=VALUE` of `play.sh`: what Wine itself complains
  of. Worth a look in any run that is slow for no reason seen.
- `VICTORY_SCREEN=<output>` in the environment or as a `KEY=VALUE` of `play.sh` (an output's
  name as `xrandr --listmonitors` has it, `HDMI-1`): the title's window covers that monitor
  and not the primary one, which changes as monitors are switched on and off (`play.sh`
  hands the runtime its rectangle, `VICTORY_FULLSCREEN_AT`). A monitor switched on or off
  while the title runs ends it or leaves it frozen: Wine's X error `BadRRCrtc` on
  `RRGetCrtcInfo` (run Q1), no more frames and no word in the log (run Q3).
