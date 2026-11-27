# Where this tree comes from

`victory-7900xtx/` is [avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)
at `0a43566` (2026-11-25), a speed fork of Marvisak's VIctory for AMD RDNA 3, with GTA6-btw's
Wine/Linux pieces from `victory-dexnom/` merged on top.

Then merged with git: [zeex64/VIctory-7900XTX](https://github.com/zeex64/VIctory-7900XTX) at
`e2b05f4` (2026-11-26), a fork of the same commit about running under Proton on RADV (RX 9070 XT):
28 commits (frame-time work, copy queue, Bink skipping, crash reports, `linux/` scripts and Wine
patches). Two conflicts: `shader_cache.rs` (both setting names kept) and `windows.rs` (its check
of `VICTORY_NO_WATCHDOG`, which also takes `0` as off). Its Wine pieces overlapped ours without
duplicates. Build and `cargo test --release -p victory-shader --lib` (206 passed) checked.

From its `linux/run.sh` the run script takes `RUST_MIN_STACK=64 MiB`: without it the largest
compute pipelines overflow the thread stack inside vkd3d-proton and the thread dies silently under
Wine; that is why three compute pipelines "never finished" here. With it they are made, so nothing
is skipped any more. Measured 2026-11-26 on an RX 7700 XT (victory-ntdll Proton): world at
6.6-10.3 fps (victory-dexnom: 1-4), the intro at a steady 30 fps once the caches are warm.

Looked at and left out: upstream PR #3163 shows that the title falls back to its ordinary
descriptions when device methods 0x1c8/0x218 (its cache of precompiled native pipelines) fail, so
they keep failing; answering them (as 101Leafy's `VICTORY_LOOSE_PIPELINES` does for 0x218) lost
draws upstream. The draws still dropped are the ray-tracing structure builder's, which is off: it
does not run usably under RADV (zeex64), and `VICTORY_AMBIENT` is the stand-in for its light.

## What was merged from victory-dexnom

The difference between `victory-dexnom/` and DexNom/VIctory `5f4c3df` (the commit it was copied
from), 15 files, applied with a three-way merge:

- Wine hooks: `crates/victory-loader/src/hooks.rs` (`VICTORY_NO_WATCHDOG`), `windows.rs`, `lib.rs`.
- Logging to a file under Proton: `VICTORY_LOG_FILE` (`victory-core/src/logging.rs`), crash
  reports (`crash.rs`) and stack dumps (`stacks.rs`, `say`) written to the log as well.
- The 1.0.2.1 int3 NOP and `VICTORY_SKIP_INT3` (`prepare.rs`).
- The early pipeline list kept without a host pipeline library (`pipeline/library.rs`).
- KLBF Lab's half-wave port: `VICTORY_PS_HALVES`, `set_host_pixel_wave`, `VICTORY_HOST_WAVE`
  (`victory-shader/src/emit/lower.rs`, `emit/mod.rs`, `victory-gpu/src/host.rs`, `shader_cache.rs`).
- Wine copy-back into write-watched pages (`host.rs` `write_mapped`).
- `VICTORY_PIPELINE_DUMP=<dir>`: dumps what the title passes to device methods 0x1c8 and 0x218
  (`victory-gpu/src/device.rs`). Title data: keep that folder out of the repository.
- `.cargo/config.toml`: the Linux cross build for `x86_64-pc-windows-msvc`.

Conflicts and how they were resolved:

- `shader_cache.rs`: this fork keys translations by `VICTORY_TRANSLATOR` and `host_settings()`;
  the host pixel wave was added to that key, and `VICTORY_PS_HALVES` to its list of settings.
- `stacks.rs`: this fork's version (thread affinity, `name_of`), with `say` in place of `eprintln!`.
- `host.rs` wave setup: `VICTORY_HOST_WAVE` and `set_host_pixel_wave`, plus this fork's
  `set_host_rdna`.
- `host.rs` `write_mapped`: this fork's retry (`watch::writing`, `again_after_change`); the Wine
  read fallback runs before a copy is given up as lost.
- `emit/mod.rs`: both sets of exports.

Checked: the Windows cross build (`cargo build --profile diag -p victory --features diagnostics
--bins --target x86_64-pc-windows-msvc`) and `cargo test --release -p victory-shader --lib` (203
passed). Under Proton (victory-ntdll, RX 7700 XT): boots with the watchdog hooks and int3 NOP
applied, and the intro runs smoothly with the settings below.

Run it with `VICTORY_TREE=victory-7900xtx scripts/run_victory_proton.sh`; the script then sets this
fork's own settings (`scripts/play.ps1`) instead of victory-dexnom's.

Under Proton the run script differs from this fork's `play.ps1` in `VICTORY_AMBIENT=0` (the
title's own ambient light), and adds zeex64's measured `VICTORY_PIPELINE_BOOK=1`,
`VICTORY_CARD_BUFFERS_MIN_MB=1`, `VICTORY_GAPS_TOGETHER=0` and `VICTORY_POOLS_MORE=0`.
`scripts/run_victory_proton.sh warm` makes the pipelines of earlier runs ahead (`victory warm --used`). `VICTORY_FORK_SETTINGS=0` turns all of this fork's settings off.

`VICTORY_BUILDER_RUN=<hash>,...` (diagnostics, `pipeline.rs`, added here): compute programs taken
for structure builder stages (they read a spilled user-data table) that run as ordinary pipelines
all the same; each one taken is logged once. Found with it (2026-11-26): the 52 such programs in the
opening are the builder's (buffer stores only); with all of them running nothing is dropped, the
phone's screen shows (it needs the finished structures), and the world runs at 3.8-5.8 fps
instead of 6-10. Frozen mouths are not from this: they do not move in victory-dexnom either.

Startup (run script, 2026-11-26): before the title starts, `victory warm --used` makes the
pipelines earlier runs used (from the pipeline book) while a zenity window says "Compiling
shaders...", so the intro waits for none (seconds with warm caches; longer after a new build or
driver). `VICTORY_WARM_FIRST=0` starts the title at once.

"present times" (diagnostics, `host.rs`, added here): every 5 s, the median, 95th and 99th percentile and longest
time between presents, and the share over 50 ms; a 5-second mean frame rate hides a stutter, these show it.

## Merged avatarmanz `main` `014d52d` and PR #440 (`born/linux-wine-pr`)

2026-11-27, in a scratch repo (the LeoTux monorepo shares no history with avatarmanz, so this
was a three-way merge through a synthetic base): this tree as one commit on `0a43566`, then
`git merge avatarmanz/main` (`014d52d`, 186 commits after `0a43566`), then `git merge
born/linux-wine-pr` ([borntofrag34](https://github.com/borntofrag34), PR #440, `deebee8`,
written against `014d52d`: +4,181/-100, 36 files; its `docs/linux-wine.md` has the
measurements). From main: the renderer ports (1, 2, 5, 6, 7), the compiler guard for AMD's
driver, `VICTORY_PORT_CONSTANTS_ORDER`, FSR 4 through AMD's library, the pipeline book. From
the PR: what makes the frame 27 to 28 ms at 1920x1080 there (`VICTORY_WINE_MAP`, the pipeline
watcher for a worker the host ends, `VICTORY_COMPUTE_WAIT_MS`, `VICTORY_LIBRARY_NULL`,
`VICTORY_SINGLE_LATCH`, `VICTORY_CLAMP_SYNC`, `VICTORY_CARD_BUFFERS_MIN_KB`, the frame
intervals, `VICTORY_JOIN_PARTS_DRAWS`), `tests/fixtures/wine-bench`, and `scripts/play.sh` and
`scripts/play-1080p.sh` as they came (this tree's launcher keeps its own settings).

Conflicts, and what stayed where it does something the newer code does not:

- `victory-gpu/src/watch.rs` (19 hunks: both trees reworked it): **this tree's** version, which
  tracks the title's views (`VICTORY_VIEW_WATCH`, `title_memory_views_tracked`, the arenas of
  placeholder reservations) and holds the guards alone with a fault of ours let through under
  the lock (`Held`, `opened_held`). Main's guards are shared with the faults' claims only with
  renderer port 6 on, and with it off a write of ours that faults under the lock waits for
  itself. Ported in: the PR's `title_view_space`/`in_view_space` (its `wine_map` tells which
  reservations have the pages it shows by the host's `mmap`) and, from main,
  `VICTORY_ASK_NORESET`/`reset_found` and `refused_as_view` in the asking's filter. Stubbed so
  that the modules beside it link: `port_journal` (the switch is read; the journal stays
  whole), `port_journal_counts` (says so in the frame line), `port_guard_plan`,
  `port_guard_line`, `alone_line` (off, nothing). Renderer port 6's shared guards and port 7's
  journal parts are therefore not here: by the PR's own runs neither gains anything under Wine,
  and port 6's one measurable effect there (protection changes trebled) is a loss.
- `pipeline/library.rs`: both. Main's span, the PR's `VICTORY_LIBRARY_NULL` (an empty library
  asked for with no address: vkd3d-proton refuses an empty slice's pointer), then this tree's
  early pipeline list saved where there is no library at all.
- `pipeline/make.rs`: main's compiler guard (`warm-bad.txt`, the vectored handler, a worker in
  its place) and the PR's watcher for a worker the host ended, in one `spawn_worker`; the first
  use tries this tree's `VICTORY_LEAVE_OUT`, then `VICTORY_RT_RESOLVE`, then the bad keys, and
  makes the rest on a large stack (`on_a_large_stack`). The worker's stack is
  `victory_shader::emit::PIPELINE_STACK` unless the PR's `VICTORY_PIPELINE_STACK_MB` says
  otherwise; `VICTORY_PIPELINE_LOWEST` and the `LARGE_STACK` mark are this tree's.
- `texture.rs`: the upload is deferred to the submission (`PreludeUpload`, as main and the PR
  both have it); this tree's aliasing barrier before the first upload into a pooled block came
  with it, now recorded in `PreludeUpload::record` where the copy is.
- `ambient.rs`: main's newer shader and its settle plumbing (`probe`, `jump`, `fresh`, `dt`,
  `least`) with this tree's reflections held to the light around in a seventeenth constant
  (`VICTORY_AMBIENT_REFLECT`, `held`); the root signature is 17 constants wide.
- `xmem.rs`: both. This tree's `title_memory_placeholder` (for reservations views come into)
  beside main's `LOW_RESERVATIONS` (the title's 32-bit heap); the PR's `wine_map` `show`/`hide`
  with this tree's `recent`/`count`/`census`; the PR's `free_in_space` in the title's `VirtualFree`.
- `descriptor.rs`, `descriptor/host.rs`, `cores.rs`, `pool.rs`, `shadow.rs`, `queue.rs`,
  `host/upq.rs`: main's (newer: the ranges a check looks at say whether port 2's fallback looks
  at them whole; `Waiting`'s `since`/`waited`/`away`; `fast_by_class`/`fast_by_cache`;
  `is_multiple_of`), with this tree's `made_at` mark kept in `pool::made_now`.
- `host.rs`: main's frame lines (made by a thread of ours) and its present result, with this
  tree's "present times", its batch retirement and the line that counts fine card passes put
  back into it.
- `inplace.rs`, `pipeline.rs`, `emit/mod.rs`, `emit/lower.rs`, `loader/lib.rs`, `lib.rs`: both
  sides' additions, side by side. `README.md` is this tree's (the Linux fork's).
- `emit/bitcode.rs`: one `single_latches`, the PR's (with `VICTORY_SINGLE_LATCH` and the test
  that checks the latch's one join); this tree's second implementation and its test are gone.

Checked: `cargo build --profile diag -p victory --features diagnostics --bins --target
x86_64-pc-windows-msvc` (clean), `cargo test --release -p victory-shader --lib` (215 passed, 2
ignored), and the Windows test binaries of `victory-gpu` (with `tests/replay*.rs`),
`victory-hle` (`tests/fidelityfx.rs`, `tests/no_dll_planting.rs`) and `victory-loader`
(`tests/title_dlls.rs`, `tests/load_probe.rs`) compile; none of them was run (a Linux host, no
Wine here), and nothing in the title was measured.
