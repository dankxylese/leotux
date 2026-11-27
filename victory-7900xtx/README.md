# VIctory under Linux, on an RX 9070 XT

A fork of [avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX),
itself a fork of [VIctory](https://github.com/Marvisak/VIctory), a compatibility
layer that runs an Xbox Series title natively on Windows from files the user
supplies. This one is about **running it on Linux through Proton, on AMD RDNA 4
with Mesa's RADV**, and about speed there: the world at 30 frames a second,
which is also the most the title asks for (it paces itself to 33.3 ms a frame).
It is not there yet.

**This repository contains no game files, code or assets.** You need your own
copy of the game. It is not affiliated with or endorsed by the game's developer
or publisher.

## Where it stands (2026-11-26)

Tested on one machine only: Linux 7.3 (CachyOS), Ryzen 9 9950X3D, 64 GB RAM,
Radeon RX 9070 XT (16 GB) with RADV, Proton Hotfix `hotfix-20261110` (vkd3d-proton
3.2.0), the Series X profile at 1440p. Frame times are medians of five-second
stretches of one play session on the release build, by how much there is to
draw.

| | |
|---|---|
| Menus, loading screens | 30 fps (the title's own cap) |
| Open world, 1,000-2,500 draws a frame | 36 ms (28 fps) |
| Open world, 2,500-4,000 draws | 42 ms (24 fps) |
| Open world, 4,000-5,500 draws | 45 ms (22 fps) |
| Open world, 5,500 draws and more | 47 ms (21 fps) |
| From the start to the world | 16-27 s |
| Shaders | 25,782 translate, 203 do not; 88,711 pipelines made ahead in 102 s |
| Memory | about 14 GB of RAM and 11-12 GB of the card's 16 in use |
| Longest session | 56 minutes with no crash on an unchanged Proton (see below) |

As it came, the fork this is made from ran the same scenes at 245-455 ms a
frame under Proton and took 112 s to reach the world.

Where a frame goes, and what does not help, is measured rather than guessed:
[linux/README.md](linux/README.md) says how (switches flipped inside one run,
a timeline of whole frames, a sampling profile). In short: the title's own
work is about 26 ms of a 39 ms frame, and it waits the rest at each present
for the one thread of ours that prepares its submissions; the card is idle
about half of every frame behind that thread.

What is known to be wrong:

- **The title crashes with the patched Wine server.** [linux/wine](linux/wine)
  makes mapping calls ten times cheaper (150-200 us to 14-17 us with 20,000
  views alive), which is what makes loading and streaming quick. With it the
  title has crashed six times in about 28 runs, each time reading through a
  null pointer in its own code, at another place; never in twenty-odd runs
  without it. The cause is not found. Crash reports now say what the
  registers point at and what was mapped there just before.
- Trees, plants and far buildings flicker outdoors in some places. Four
  switches that were suspected did not change it.
- Large magenta bursts of light in some scenes. Three fixes from other forks
  are in for it (`VICTORY_NAN_GUARD`, `VICTORY_AMBIENT_REFLECT`,
  `VICTORY_RT_RESOLVE`); whether they help has not been seen.
- Rooms with little direct light are too dark: the ray-tracing structure
  builder is off here (under RADV it runs at over a second a frame or hangs),
  and `VICTORY_AMBIENT` stands in for the light it would give.
- Textures of what just came into view appear late while the title streams.
- The first run with a new driver or a new build stutters while the driver makes
  pipelines (`victory warm` makes them ahead).
- A Vulkan layer that hooks instance creation can leave the title without a
  picture (white window, sound): `lsfg-vk` did when its profile matched the
  process. `DISABLE_LSFGVK=1` in front of the launch turns that one off.
- Recording a frame in the world makes the title's own watchdog end it.

## How to run it

**On Linux:** [linux/README.md](linux/README.md). `linux/build.sh`
cross-builds `victory.exe`, `linux/run.sh` and `linux/play.sh` run it under a
Proton build through `umu-run`; the rest of this section is the Windows way,
as the fork this comes from has it.

You need Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with shader
model 6.6 and mesh shaders, Rust (the toolchain is pinned), the MSVC build
tools, and your own dump of the title.

```powershell
git clone https://github.com/avatarmanz/VIctory-7900XTX
cd VIctory-7900XTX
copy victory.example.toml victory.toml      # then set your paths, outside this folder
cargo build --locked --profile diag -p victory --features diagnostics
scripts\play.ps1 -Exe <path of your copy's executable>
```

`scripts\play.ps1 -Safe` leaves out the newest, faster ways; try it first when
something is wrong. `-RayTracing` turns the title's ray-traced indirect light on.

For PC FSR 4.1.2, set `[paths].fidelityfx_dir` in `victory.toml` or
`VICTORY_FIDELITYFX_DIR` to the folder containing AMD's signed
`amd_fidelityfx_upscaler_dx12.dll`. Use the DirectX 12 upscaler in
`Kits/FidelityFX/signedbin`. [AMD FSR SDK 2.4.0](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/tree/v2.4.0/Kits/FidelityFX/signedbin)

`scripts\play.ps1 -Exe <path> -Fsr4K quality` selects 2560×1440 input for
the retail 1.0.2.1 Series X 4K SDR profile. `balanced` selects 2258×1270
and `performance` selects 1920×1080. All three output 3840×2160. With
`victory run` directly, set `VICTORY_FSR_4K` to the preset name.
The log reports the provider at context creation and the actual input and
output sizes at the first successful temporal dispatch. The title profile
and PC FidelityFX bridge were ported from [DexNom/VIctory](https://github.com/DexNom/VIctory/commit/ab34a8b21477c94da2a6cf07249138e8bc84602e).

On a hybrid-graphics PC, the runtime and offline pipeline checks prefer a
high-performance adapter with Shader Model 6.6. The runtime logs its adapter
name, PCI vendor and device IDs, and dedicated VRAM. Set
`VICTORY_D3D12_ADAPTER` to a zero-based DXGI index or part of the adapter's
name to select it explicitly. With the launcher, pass
`-More @{VICTORY_D3D12_ADAPTER='NVIDIA'}`.

Before the first run, ahead of time (both can be stopped and go on later):

```powershell
target\diag\victory.exe shaders <the shader archives>   # every shader translated, minutes
target\diag\victory.exe warm --used                     # pipelines earlier runs drew with, for a new driver or build
```

Commands, debugging switches and upstream's own description:
[docs/README-fxceless.md](docs/README-fxceless.md). Every `VICTORY_...` switch is
described in a comment where it is read.

## Linux (Wine)

The same `victory.exe` runs under Wine with vkd3d-proton, started by
`scripts/play.sh`, the counterpart of `play.ps1`. Measured on one PC (Ryzen 9
9900X, Radeon RX 7900 GRE 16 GB with RADV of Mesa 26.3, Proton Experimental's
Wine 11, kernel 7.3), Series X profile, ray tracing off, in the title's first
scene: 2.3 frames a second before this work, 21 at 3840x2160, and 36 to 37 with
`scripts/play-1080p.sh` (the title rendering at 1920x1080, renderer patches 1,
2 and 5, the title let above its 30 frames a second). Everything that is
Wine's alone is behind `on_wine()` or a switch and leaves Windows as it was.

You need a Vulkan driver with mesh shaders, Proton (its Wine, vkd3d-proton and
DXVK's `dxgi.dll` are copied into a prefix of their own; `PROTON=<folder>`
names another than Steam's Proton Experimental), Microsoft's 64-bit
`d3dcompiler_47.dll` (`D3DCOMPILER=<path>`; Wine's own cannot compile the
runtime's shaders), Rust with the `x86_64-pc-windows-msvc` target and
[cargo-xwin](https://github.com/rust-cross/cargo-xwin), and your own dump of
the title.

```sh
git clone https://github.com/avatarmanz/VIctory-7900XTX
cd VIctory-7900XTX
cp victory.example.toml victory.toml        # then set your paths as Wine sees them (Z:\home\...), outside this folder
cargo xwin build --locked --profile diag -p victory --features diagnostics --target x86_64-pc-windows-msvc
scripts/play.sh --cmd shaders                        # every shader of the title translated, once
scripts/play.sh <your copy's executable>             # as play.ps1 does
scripts/play-1080p.sh                                # GAME_EXE=<executable>: 1920x1080, the fastest settings measured
```

`scripts/play.sh --safe`, `--rt`, `--start` (into the first scene without the
keyboard) and `KEY=VALUE` settings are described at the top of the script;
`VICTORY_SCREEN=<output>` puts the window on another monitor than the primary
one.

What was found under Wine, with the runs behind each figure, the switches and
what is still open: [docs/linux-wine.md](docs/linux-wine.md). In short:

- Wine's server walked its list of views at every map and unmap: the title's
  pages are shown with the host's `mmap` instead (`VICTORY_WINE_MAP`).
- Programs with loops of several back edges ran vkd3d-proton's dxil-spirv out
  of stack; they get one latch a loop (`VICTORY_SINGLE_LATCH`), and a pipeline
  whose making ends its worker is given up (`VICTORY_PIPELINE_WATCH`).
- Renderer patch 1 gave a black picture: amdgpu takes memory by address only as
  one anonymous mapping, so the title's graphics pages are shown as one
  (`VICTORY_PORT_HOME`).
- People other than the heroine had no face in every second or third run: a
  compute pass the title runs once was dropped while its pipeline was being
  made. Compute passes wait for theirs (`VICTORY_COMPUTE_WAIT_MS`), and the
  pipeline library is kept under Wine as on Windows (`VICTORY_LIBRARY_NULL`).
- The waits around the layer's own clamps of indirect arguments emptied the
  card under RADV; they name what they wait for (`VICTORY_CLAMP_SYNC`).

Not looked at: any scene but the first, play, ray tracing, other cards and
drivers.

## What is different here

In short; the reasons and the measurements are in [docs/findings.md](docs/findings.md).

- **List calls are written down, not carried out, on the title's threads**
  (`VICTORY_THIN`): eight threads of ours translate them.
- **Submissions are prepared by a thread of ours and joined**
  (`VICTORY_ASYNC_SUBMIT`, `VICTORY_JOIN_PARTS`), with what can be spread over
  threads spread: refreshes, compares (AVX2), descriptor records, landings.
- **What shaders wrote comes back beside the write guards**, by the dwords that
  changed, on a thread of its own (`VICTORY_RELAY`, `VICTORY_LAND`).
- **Buffer fills that need bytes still on the card are done on the card**
  (`VICTORY_CARD_STEPS`) instead of waiting for it.
- **The title's writes are caught more cheaply**: one view opened at a fault,
  blocks that rest, places in the title's code that look before they write.
- **Textures go up on a copy queue of their own** and a slot shows one only after
  its upload ran, which also ended a device loss that came every 15 minutes or so.
- **Boats float**: textures that shaders write and the title's CPU reads go back
  to title memory (from fxceless and Geckomacabre, on by default here).
- **Frame record and replay** with per-pass times and shader variants compared
  turn by turn ([docs/replay.md](docs/replay.md)).

## Credits

This is a tree of other people's work with more on top. The history is kept, and
commits taken from elsewhere name the commit and its author.

- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and its
  contributors: the loader, the runtime services, the Direct3D 12 back end and
  the shader recompiler.
- **[avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)**:
  the fork this one is made from; what the section "What is different here"
  lists is its work.
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)**: the fork
  that one started from, and the people whose pull requests it carries.
- **[DexNom/VIctory](https://github.com/DexNom/VIctory)**: ray tracing fixes,
  cube gradients, sparse textures.
- **[Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD)**:
  the ambient light idea, pipeline workers, and work by romainhedouin.
- **[FQY55kai/ZJOU-VICTORY](https://github.com/FQY55kai/ZJOU-VICTORY)**.
- **[myokex/victory-amd](https://github.com/myokex/victory-amd)**: the guard over
  every view of a page (with Spexivus, joroyt and tightfall), and Joshua Odom's
  reads into title memory.
- **Geckomacabre** and **cxy0072019-sketch**: the texture read-back.
- **[XCL](https://github.com/ivall/xcl)** (GPL-2.0-only) was read for how the same
  title behaves under another layer; no code of it is here.
- This fork's changes were made with [Claude Code](https://claude.com/claude-code).

## License

GPL-3.0-or-later, as VIctory. See [LICENSE](LICENSE); the one file that is not
covered is named in [docs/asset-policy.md](docs/asset-policy.md).
