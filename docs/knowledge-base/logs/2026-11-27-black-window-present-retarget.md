# Session log — 2026-11-27 (part 3) — The black window: the present refuses its own retarget

Branch `feat/arena-single-address-space`, worktree `worktrees/a9f-descriptor-bridge`. Run analysed:
`run-dirs/a9f-run/run-2026-11-27-gap.log` (live, 09:58:46–10:29:30, ~500 presents, gap placement on).
Dig only; no code changed.

## TL;DR

The title is **not** gated and its draws are **not** dropped: it submits a 2-draw frame ~every 4 s, the
pipeline pool reports zero drops at every present, and the only skipped work is 48 FSPX compute pipelines with
no SPIR-V. The window is black because **the presenter refuses its own retarget from 10:17:42 on** and then
blits the swapchain buffer nothing draws into.

- `VulkanContext::composite_image_` (`runtime/src/vulkan_backend.h:2403-2404`) is written **only** inside
  `if (index < 12)` in `runtime/src/vulkan_replay.h:4739` → `:4749-4750`. The first 12 composite draws were
  all at 09:59:04–09:59:07, so the record froze on image `7F3A04014A70`.
- `composite_target_` is written on **every** composite draw (`vulkan_replay.h:2340-2344`, ungated), so it
  followed the title to the new target `0x7f2e93c758c0` (image `5555F319D1E0`).
- The present's substitution guard compares the two: `drew_into && drawn->image != drew_into` →
  `runtime/src/vulkan_present.h:344-352` → refused; the present then blits `source` (`:420-427`), i.e. the
  title's presented buffer, which by the layer's own console-model note (`:307-311`) nothing draws into.
- Timeline: id `0xfe87c540`→image A substituted from 09:59:04; at **10:17:42** the id becomes
  `0x7f2e93c758c0`→image B ≠ frozen A, refused 6× (the cap) until 10:18:27, silently refused ever since.
  Six identical prints over 45 s (same id, same image B) = the pair is stable; only the recorded image is stale.

## Per-present inventory (what a frame contains)

| item | count | evidence |
|---|---|---|
| presents | ~500, 1 per ~4 s | `PresentX call` printed at powers of two only (`shims/xbox_present.cpp:139`): 1,2,4,…,256 at 10:10:31 |
| submissions | 1 per present | `refresh_title_ranges` is on `set_before_execute` (`shims/d3d12_vulkan_device.cpp:8146-8149`) |
| draws | **2**, both into the composite target | `COMPOSITE-DRAW 0..11` = 6 frames × 2 at 09:59:04–07 (`vulkan_replay.h:4739-4750`): `count 3, vb0 0x0 stride 0` then `count 3, vb0 <heap> stride 24` |
| dispatches | 0 before 10:17:30, **1** after | the Dispatch instantiation's 12 `BACKING:` lines run 10:17:30–10:18:27, 4.5 s apart, "0 root view(s), 2 table(s)" |
| root-view windows | 2 new per frame after 10:17:42 | 151 × `GAP 0xd00 at 0x…cf40` + `GAP 0xd10 at 0x…f270`, one pair per frame |
| drops | **0** | `pipeline_pool.h:295-310` prints at every present when `drops>0` (called from `d3d12_vulkan_device.cpp:12680`); no such line in ~500 presents |
| present failures | 0 | no "present failed" line |

Note on the earlier readings: every per-draw print is capped — `back_root_views`' report at
`vulkan_replay.h:2208-2210` (12 **per template instantiation**: Draw and Dispatch have separate statics),
`COMPOSITE-DRAW` at `:4739` (12), the retarget prints at `vulkan_present.h:330-336/346-347/365-368/374-375`
(3/6/6/4). "30 draw-ish lines in 23 minutes" is the gating, not the frame.

## Readiness gate: none found in the runtime

- 0 pipeline-pool drops, 0 `DROP` lines, 0 `present failed`, 0 `E_INVALIDARG` for pipelines.
- The 4 mesh routings (10:17:20–21) logged no refusal: the only mesh refusal the layer can print
  (`d3d12_vulkan_device.cpp:6196` "mesh-shader PSOs need VK_EXT_mesh_shader") never fired, so the device's
  mesh path was taken and the PSOs were created.
- The 48 `slot 67 … failed with 0x80070057` lines are `DeserializeComputePipelineStateX` on **FSPX compute**
  shaders with no SPIR-V (the message right after each is "no translated compute shader for FSPX kind-6 and
  kind-0"): the known parked FSPX gap, and the only work the run skips.
- Streaming stopped at 10:17:21 (`dstorage: POOL 59.3 s … 36 requests`), so the title is not waiting on I/O;
  it keeps presenting a static 2-draw frame at 0.25 fps.

## Readback, available but off in this run

`LEONIDA_RT_READBACK=1` (`vulkan_present.h:190-207`, `record_source_readback` `:1653`, line at `:1724`
`source readback %ux%u fmt %d mean r g b, nonzero %`, sampling the image the blit **reads**, i.e.
post-substitution). Not in `run-gap.sh`. Inference without it: the composite's content is a 3-vertex rect with
a stride-24 VB; its colour comes from the tint table, and the 09:59:04 diagnostic read that table slot
`(unreadable)` with four zero words right after the fill (`vulkan_replay.h:4786-4794`), so a fixed present may
still be transparent black — the readback separates the two layers.

## Fix (recommended, not applied)

1. `runtime/src/vulkan_replay.h:4749-4750` — move `note_composite_image(...)` out of the `index < 12` gate to
   the `if (into_composite)` body (`:4736`), so the image record is live. One atomic store; no logging.
2. Keep the guard at `vulkan_present.h:345`; with a live record it becomes a real per-frame check (the A9bd
   "the title creates and frees its render targets per frame" hazard stays covered).
3. Re-print the refusal on a cadence (every 64th present, or once per 30 s) instead of 6 per run — a permanent
   refusal is otherwise invisible after 45 s, which is how this looked like "the title draws nothing".
4. Verify with `LEONIDA_RT_READBACK=1 LEONIDA_RT_READBACK_EVERY=8 LEONIDA_RT_READBACK_LIMIT=24` +
   `LEONIDA_PRESENT_RT=1`: `nonzero%` on the presented source proves the composite's pixels reach the window;
   still 0 with the composite target sampled means the tint table is the next layer.

## Part 4 — implemented (A9cl)

- `vulkan_replay.h:4737-4755`: `note_composite_image(...)` moved out of the `index < 12` gate into the
  `if (into_composite)` body, and it now records the colour attachment whose id IS the composite target (falling
  back to slot 0, and never recording 0 - the guard reads 0 as "unknown"). One atomic store per composite draw.
- `vulkan_present.h:352-372`: the refusal line prints on the first refusal, then every 64th present, and one line
  when the substitution is accepted again (`refused_presents` counter, reset on acceptance).
- `vulkan_present.h:400`: the `PRESENT_DRAWN_FROM` arm now requires a non-empty setting. Before, with
  `LEONIDA_PRESENT_RT` unset/0 and no `PRESENT_DRAWN_FROM`, the arm was entered and `std::strcmp(nullptr, ...)`
  dereferenced 0 on the first present - `d3d12_present_test` crashed exactly there (see below). The whole retarget
  block is now inert in that configuration, as the flag's contract says.
- `vulkan_present.h:861`: `submit_now` uses the binding present() chose (**the substituted render target**), not
  `resolve(source)`. Resolving `source` there threw the substitution away: the blit always read the title's
  presented buffer, so the retarget never actually changed what the window showed (and the readback measured the
  same wrong image). `source` still keys the tracker/rollback (A9bl). A queued present (empty binding flag) still
  resolves `source`.
- `vulkan_present.h:487` + `:659`: `set_retargeting` is per present (`drawn_ok`), and cleared for a deferred
  present. It was sticky-true after the first accepted substitution, so a later refused present took the
  render-target barrier path for the title's buffer.
- `vulkan_present.h:1090` + `:1143` (+ `vulkan_backend.h:1283 relayout_after_blit`): the retarget's
  GENERAL→TRANSFER_SRC transition moved ahead of the readback (so the probe samples a legal layout), and a
  TRANSFER_SRC→GENERAL transition follows the blit so the composite's image is back where the tracker believes it is.
- Readback sampling point: **no move needed** - `record_source_readback(commands, binding, ...)` is fed the same
  `binding` the blit reads, so it measures what is actually presented in both arms. Its `src init` flag now reports
  0 on a retargeted present (nothing was initialised by the presenter) instead of the title buffer's first-use flag.

### Tests (wine, project prefix)

| target | result |
|---|---|
| `d3d12_vulkan_device_test` | pass |
| `d3d12_placed_heap_lifetime_test` | pass |
| `d3d12_descriptor_test` | pass |
| `d3d12_present_test` | **baseline crashed**, null read (`rcx=0`) in a system module at the first present - the `strcmp(nullptr)` of the third arm; now runs 60 presents + resize + capture (frames 1272x686, mean rgb `255 0 0` / `51 153 229` alternating = the clear colours), one pre-existing assert left: `line 478: first.width == width && first.height == height` (1272x686 vs the requested 320x240 - the Wayland compositor's window size, not the layer's) |

