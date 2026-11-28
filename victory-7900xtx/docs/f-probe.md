# The F-probe: a section in place under Wine (`SEC_COMMIT` + `OpenExistingHeapFromFileMapping`)

`UPSTREAM.md` records that a heap opened on a **view** of a `SEC_COMMIT` section
(`OpenExistingHeapFromAddress1`) opens, a buffer places in it, and the card then reads zeros
there and its writes never reach the view. The F-path (VIctory-Fast) is a different entry
point: make the section `SEC_COMMIT`, hand its **handle** to
`ID3D12Device3::OpenExistingHeapFromFileMapping`, and let the runtime map the section itself.
This is the measurement of that path, on this PC, through the same Wine and vkd3d-proton the
launcher runs, with the card (`MESA_VK_DEVICE_SELECT=1002:747e`).

**Answer: it fails, in the same way and for a reason now measured at the driver.** The D3D12
calls all succeed — heap, heap description, placed buffer, GPU address — and the memory the
card reads and writes is **not the section's**: vkd3d-proton's import of the section's pages
fails, and it quietly gives the heap memory of its own. Zeros in, nothing out. No D3D12 entry
point can fix this, because the refusal is RADV's, not the runtime's: `VK_EXT_external_memory_host`
takes **anonymous** pages and refuses every **file-backed** one, and under Wine a section is a
memfd.

## The probe

`apps/seccommit-probe` (workspace member, Windows target only): its own D3D12 device, queue,
fence, readback and upload buffers, and a compute shader (`dst = src ^ 0xFFFFFFFF`, two root
UAVs, signature serialized here because Wine refuses a blob's own). Nothing of a game's is
touched; no game runs.

```sh
cd victory-7900xtx
cargo build --profile diag -p seccommit-probe --target x86_64-pc-windows-msvc
WINEPREFIX=/home/manse/A.Projects/GTA6-Emulation/victory-proton-umu/pfx \
MESA_VK_DEVICE_SELECT=1002:747e DRI_PRIME=1 \
WINEDLLOVERRIDES="d3d12=n;d3d12core=n;dxgi=n;d3dcompiler_47=n" \
VKD3D_DEBUG=warn WINEDEBUG=-all \
wine target/x86_64-pc-windows-msvc/diag/seccommit-probe.exe
```

It runs five batteries: the F-path on `SEC_COMMIT`; the same on `SEC_RESERVE` (how `xmem`
made the page pool before); private `VirtualAlloc` memory by address (the path that works,
`VICTORY_PORT_HOME`'s); a committed section's **view** by address (the path that read zeros);
and, inside the first, the guards' questions (a view made read-only under the heap, a view
unmapped and mapped again).

### Its output (2026-11-28; four runs, the same verdict in each)

```
adapter 0: AMD Radeon RX 7800 XT (RADV NAVI32) (vendor 0x1002, device 0x747e, vram 16368 MB)
device: ID3D12Device3 (OpenExistingHeapFromAddress/FromFileMapping) true, ID3D12Device13 (a part of an allocation) true
rig: queue, fence, readback, upload and the compute shader ready
1. a section of 2048 KiB made SEC_COMMIT, a view of it at 0x78c0000
   OpenExistingHeapFromFileMapping: ok
   heap of 2048 KiB: type 4 (1 = default, 4 = custom), CPU pages 3 (3 = write-back), pool 1 (1 = the PC's memory), alignment 65536, flags 0xe1 (0x1 shared, 0x20 cross-adapter)
  a buffer placed in it (cross-adapter, unordered access): ok
   the buffer's GPU address: 0xffff800100800000
   (a) the card reads the CPU's bytes (copy engine): WRONG (the card read zeros: 2088960 of 2097152 bytes differ, first at 0x0)
   (a2) a compute shader reads the CPU's bytes: WRONG (first at 0x0: the CPU has 0xfe, the card 0xff; 2088960 of 2097152 bytes differ)
   (b) the card's write (copy engine) read at the view's address after the fence: WRONG (first at 0x0: the CPU has 0x5a, the card 0x01; 2097152 of 2097152 bytes differ)
   (b2) the card's write by a compute shader (in place) read at the view's address after the fence: WRONG (the CPU's bytes are not the shader's)
   (c) the CPU writes the view, then the card reads it (no flush between): WRONG (first at 0x0: the CPU has 0x07, the card 0xa5; 2080768 of 2097152 bytes differ)
   (c2) the same with FlushViewOfFile before it (ok): WRONG (first at 0x0: the CPU has 0x08, the card 0xa5; 2097152 of 2097152 bytes differ)
   (d) the guards' calls on the view under the heap: VirtualProtect read-only ok, the card still reading the page false, and back ok; VirtualFree decommit of one page REFUSED 0x80070057 (Невірний параметр.), committed again false
   (e) the view unmapped (ok), the card writes, a view mapped afterwards shows it (ok): false
   (f) the card copies 2048 KiB out of the section in place: 0.317 ms; out of a buffer in the card's memory of today's kind: 0.245 ms
2. a section made SEC_RESERVE, a view of it at 0x78c0000: committed by VirtualAlloc ok
   OpenExistingHeapFromFileMapping on a reserved section: ok
  a buffer placed in it (cross-adapter, unordered access): ok
   a heap on the reserved view by OpenExistingHeapFromAddress1: the card reads WRONG (the card read zeros: 2088960 of 2097152 bytes differ, first at 0x0)
3. private VirtualAlloc memory, a part of it by address (OpenExistingHeapFromAddress1): ok
  a buffer placed in it (cross-adapter, unordered access): ok
   the card reads the CPU's bytes: OK
   the CPU reads the card's write (copy engine) at the memory's address after the fence: true
   the CPU reads the shader's own write (in place) after the fence: OK
4. a committed section's view by address (OpenExistingHeapFromAddress1): ok
  a buffer placed in it (cross-adapter, unordered access): ok
   the card reads the view's bytes: WRONG (the card read zeros: 2088960 of 2097152 bytes differ, first at 0x0)
VERDICT: section SEC_COMMIT + OpenExistingHeapFromFileMapping: reads false, writes false, later CPU writes false, outlives views false, guards still work false
```

### What vkd3d-proton says while it does it (`VKD3D_DEBUG=warn`, once per heap opened on a section)

```
warn:vkd3d-proton:vkd3d_try_allocate_device_memory: Failed to allocate 2097152 bytes on memory type 5, vr -1.
warn:vkd3d-proton:vkd3d_try_allocate_device_memory: Failed to allocate 2097152 bytes on memory type 10, vr -1.
warn:vkd3d-proton:vkd3d_import_host_memory: Failed to import host memory, hr 0x8007000e.
```

Four of these in the run: the `SEC_COMMIT` heap, the `SEC_RESERVE` heap, the reserved
section's view and the committed section's view. **None** for the private `VirtualAlloc` heap,
which reads `OK` — that one is imported.

`0x8007000e` is `E_OUTOFMEMORY`: the import was attempted (`VK_EXT_external_memory_host` is
supported here, so the "not supported, falling back" path is not the one taken) and the driver
refused the pages. vkd3d-proton then gives the heap memory of its own and answers as if it had
not failed — which is why every D3D12 call above says `ok`.

### Why the driver refuses: the pages are file-backed

`apps/seccommit-probe/hostptr.c` asks the driver directly, with no Wine in the way
(`gcc -O2 -o hostptr hostptr.c -lvulkan; MESA_VK_DEVICE_SELECT=1002:747e ./hostptr`):

```
device 0: AMD Radeon RX 7800 XT (RADV NAVI32)
minImportedHostPointerAlignment: 4096
anonymous private (VirtualAlloc)   pointer 0x7fe5d5400000: type 5 (props 0xe, host-visible host-coherent): VK_SUCCESS
anonymous private (VirtualAlloc)     the driver's mapping sees the CPU's first byte as it is (0xa5 vs 0xa5)
memfd section (SEC_COMMIT)         pointer 0x7fe5d5200000: type 5 (props 0xe, host-visible host-coherent): VK_ERROR_INVALID_EXTERNAL_HANDLE
tmpfs file (backed section)        pointer 0x7fe5d5000000: type 5 (props 0xe, host-visible host-coherent): VK_ERROR_INVALID_EXTERNAL_HANDLE
disk file (page cache)             pointer 0x7fe5d4e00000: type 5 (props 0xe, host-visible host-coherent): VK_ERROR_INVALID_EXTERNAL_HANDLE
```

Anonymous pages import; a memfd, a tmpfs file and a disk file (page cache) are all refused
with `VK_ERROR_INVALID_EXTERNAL_HANDLE`. And Wine's sections are memfd: with `PROBE_HOLD_SECS`
the probe waits with its section mapped, and `/proc/<pid>/maps` of the Wine process shows the
view it holds

```
079c0000-07bc0000 rw-s 00000000 00:01 156155                       /memfd:wine-mapping (deleted)
```

— which is the `0x79c0000` the probe printed. So the chain is closed: a section under Wine is
a memfd mapping, RADV will not take a memfd mapping as host memory, and every D3D12 opening of
a section (by handle or by view) is answered with memory of the runtime's own.

## Verdict, per API path

| Path | Heap opens | Buffer places | Card reads the bytes | Card's writes reach the CPU | Usable in place |
|---|---|---|---|---|---|
| `SEC_COMMIT` + `OpenExistingHeapFromFileMapping` (the F-path) | yes | yes | **no** (zeros) | **no** | **no** |
| `SEC_RESERVE` + `OpenExistingHeapFromFileMapping` | yes (Windows refuses this; vkd3d-proton does not check) | yes | **no** (zeros) | **no** | **no** |
| committed section's **view** + `OpenExistingHeapFromAddress1` (today's `xmem` route) | yes | yes | **no** (zeros) | **no** | **no** |
| private `VirtualAlloc` + `OpenExistingHeapFromAddress1` (port 1, `VICTORY_PORT_HOME`) | yes | yes | **yes** | **yes** | **yes** |

The `SEC_RESERVE` result is worth keeping: on Windows the runtime refuses a reserved section
outright, so `xmem`'s `SEC_COMMIT` under Wine looked as if it were what made the opening
possible. Under vkd3d-proton it changes nothing — both open, both read zeros. No flag choice on
the section can rescue the path.

Coherence, for the one path that works (private memory, measured here): **the fence is the
whole boundary**. A CPU write is read correctly by the card in the submission that follows it,
with no call in between (no `FlushViewOfFile`, no flush of any kind) — test 3's `OK`, and the
same for a shader reading it. The card's write is read correctly by the CPU once the
submission's fence has been waited for, by the copy engine and by a compute shader writing the
memory in place (`the CPU reads the shader's own write (in place) after the fence: OK`). No
barrier beyond the submission, no cache maintenance either side: the memory type is
`DEVICE_LOCAL | HOST_COHERENT` on this card, and vkd3d-proton's submission does the rest.

## What this means for our placed heaps

1. **Do not wire the F-path in.** It is worse than a refusal: `OpenExistingHeapFromFileMapping`
   succeeds for a section under Wine, so an implementation built on it would run, read zeros
   and put out black frames (`VICTORY_PORT_CONSTANTS=1`'s failure of 2026-11-27, again). No
   amount of flag or entry-point choice changes it: the pages a section is made of are the
   problem.
2. **Port 1's in-place memory is unaffected and is the only in-place memory there is.** Title
   memory that is `VirtualAlloc`'d private — `XMemVirtualAlloc`'s, and the constants pages
   `VICTORY_PORT_HOME` shows as private at the title's own address — imports and is coherent
   both ways with the submission fence as the only sync. That is what `place_for`'s
   `private_whole` check already selects, and the probe now has a second measurement of it on
   this card under this Wine (the first being `port.rs`'s `under_wine_a_stretch_at_home...`
   test).
3. **The copies-up bucket cannot be shrunk by the page pool.** The 20-25 ms a frame of
   shadow-copy refresh is spent on section-backed memory — the page pool's views. Those bytes
   stay copied: `inplace.rs`'s census keeps counting `KINDS[1..4]` (view memory, one run /
   scattered / with holes) as "copied", and `place_for` keeps turning view memory down
   (`STATS[4]`). The only way that bucket shrinks is to make more of the title's memory
   private, i.e. to extend port 1's home (`VICTORY_PORT_HOME`, `xmem::home_on`) to title
   buffers whose pages are shown at one address: `place_for(start, len, window = false)` at
   `VICTORY_INPLACE=2` already does exactly this for private buffers, and `private_whole` is
   the guard that keeps it honest. Each byte moved from the pool to a private home is a byte
   out of the refresh; every byte that stays a view of the section is not.
4. **The guards' census should say so**: a section-backed range can never be in place, not
   "it depends on the flag". `inplace::census_of`'s wording for view memory ("reserved:
   refused as a heap until it is made with `SEC_COMMIT`") is wrong under Wine — committed or
   reserved, it is refused by the driver — and the counters should carry that, so that no one
   reads a `SEC_COMMIT` change as an opening.
