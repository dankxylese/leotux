//! The F-probe: can the card read title memory **in place**, through a
//! *section*, under Wine?
//!
//! `UPSTREAM.md` has the earlier measurement: a heap opened on a **view** of a
//! `SEC_COMMIT` section (`OpenExistingHeapFromAddress1`) opens, a buffer is
//! placed in it, and the card reads zeros there and its writes never reach the
//! view — so section views had to be copied. The F-path (VIctory-Fast) is
//! different: make the section `SEC_COMMIT`, hand its **handle** to
//! `OpenExistingHeapFromFileMapping` (`ID3D12Device3`), and let the runtime map
//! the section itself. This program asks, on the real card through the same
//! Wine + vkd3d-proton the launcher uses, what that path gives:
//!
//! 1. does the heap open, and a buffer place in it;
//! 2. does the card read the bytes the CPU wrote (copy engine and a compute
//!    shader), and does it read them again after later CPU writes;
//! 3. do the card's writes (copy engine and shader) reach the CPU's view, and
//!    what has to happen in between (the fence is the boundary measured);
//! 4. does it survive the guards' calls on the view: `VirtualProtect`
//!    read-only and back, the view unmapped and mapped again;
//! 5. what it costs, against a copy out of a shared buffer of today's kind.
//!
//! For contrast the same battery runs on `SEC_RESERVE` (how `xmem` used to make
//! the page pool), on private `VirtualAlloc` memory (the known good path,
//! `VICTORY_PORT_HOME`'s) and on a **view** + `OpenExistingHeapFromAddress1`
//! (the known bad path).
//!
//! Build and run (from `victory-7900xtx/`):
//!
//! ```sh
//! cargo build --profile diag -p seccommit-probe --target x86_64-pc-windows-msvc
//! WINEPREFIX=/home/manse/A.Projects/GTA6-Emulation/victory-proton-umu/pfx \
//! MESA_VK_DEVICE_SELECT=1002:747e DRI_PRIME=1 \
//! WINEDLLOVERRIDES="d3d12=n;d3d12core=n;dxgi=n;d3dcompiler_47=n" \
//! VKD3D_DEBUG=warn \
//! wine target/x86_64-pc-windows-msvc/diag/seccommit-probe.exe
//! ```

use std::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_FEATURE_LEVEL_12_0, ID3DBlob};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIFactory1};
use windows::Win32::System::Memory::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{Interface, s};

/// The unit a heap and a placed buffer are cut on.
const K64: usize = 64 << 10;
/// The section's size: 32 pages, the least that shows a run of pages.
const SIZE: usize = 32 * K64;

/// The whole account: printed as it is found, and kept for the verdict.
#[derive(Default)]
struct Report {
    /// (a) the card reads the CPU's bytes through the file-mapping heap.
    reads: bool,
    /// (b) the CPU reads the card's writes at the view's address.
    writes: bool,
    /// (c) the CPU's later writes reach the card with no call in between.
    later: bool,
    /// The heap outlives its views (unmap, GPU write, map again, read).
    outlives: bool,
    /// The guards' calls on the view still work under a standing heap.
    guards: bool,
}

impl Report {
    fn say(&mut self, line: impl Into<String>) {
        println!("{}", line.into());
    }
}

/// `Ok`/`REFUSED 0x…`, for a line about one call.
fn hr<T>(result: &windows::core::Result<T>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(e) => format!("REFUSED {:#010x} ({})", e.code().0, e.message().trim()),
    }
}

fn fill(at: *mut u8, len: usize, seed: u8) {
    for i in 0..len {
        // SAFETY: the caller's own committed memory of `len` bytes.
        unsafe { at.add(i).write((i as u8).wrapping_mul(31).wrapping_add(seed)) };
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

/// What the CPU's memory holds, against what it should: `OK`, or how it differs
/// (`the card read zeros` when every byte the card gave back is 0).
fn judge(want: &[u8], got: &[u8]) -> String {
    if got == want {
        return "OK".to_owned();
    }
    let first = want.iter().zip(got).position(|(a, b)| a != b);
    let zeros = got.iter().all(|b| *b == 0);
    let differ = want.iter().zip(got).filter(|(a, b)| a != b).count();
    match first {
        Some(at) if zeros => format!("WRONG (the card read zeros: {differ} of {} bytes differ, first at 0x{at:x})", want.len()),
        Some(at) => format!("WRONG (first at 0x{at:x}: the CPU has 0x{:02x}, the card 0x{:02x}; {differ} of {} bytes differ)", want[at], got[at], want.len()),
        None => "WRONG (the card gave back a different length)".to_owned(),
    }
}

fn describe(heap: &ID3D12Heap) -> String {
    // SAFETY: a plain query.
    let d = unsafe { heap.GetDesc() };
    format!(
        "heap of {} KiB: type {} (1 = default, 4 = custom), CPU pages {} (3 = write-back), pool {} (1 = the PC's memory), alignment {}, flags {:#x} (0x1 shared, 0x20 cross-adapter)",
        d.SizeInBytes >> 10,
        d.Properties.Type.0,
        d.Properties.CPUPageProperty.0,
        d.Properties.MemoryPoolPreference.0,
        d.Alignment,
        d.Flags.0
    )
}

fn buffer_desc(size: u64, flags: D3D12_RESOURCE_FLAGS) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Width: size.max(1),
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: flags,
        ..Default::default()
    }
}

/// The shader the probe reads and writes memory with: `dst[i] = src[i] ^ 0xFFFFFFFF`
/// over the whole buffer, one thread to four bytes, `src` and `dst` root
/// unordered-access views. In place (`src == dst`) each thread owns its word.
const SHADER: &str = "\
RWByteAddressBuffer src : register(u0);
RWByteAddressBuffer dst : register(u1);
[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint i = id.x * 4u;
    dst.Store(i, src.Load(i) ^ 0xFFFFFFFFu);
}
";

/// A queue, a list, a fence, a readback and an upload buffer, and the shader,
/// all of the probe's own: nothing of a game's submissions is touched.
struct Rig {
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    done: u64,
    event: HANDLE,
    readback: ID3D12Resource,
    upload: ID3D12Resource,
    root: ID3D12RootSignature,
    pso: ID3D12PipelineState,
}

impl Drop for Rig {
    fn drop(&mut self) {
        // SAFETY: our own event.
        unsafe { let _ = CloseHandle(self.event); }
    }
}

impl Rig {
    fn new(device: &ID3D12Device) -> windows::core::Result<Rig> {
        // SAFETY: plain creation of objects of the probe's own.
        unsafe {
            let queue: ID3D12CommandQueue =
                device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC { Type: D3D12_COMMAND_LIST_TYPE_DIRECT, ..Default::default() })?;
            let allocator: ID3D12CommandAllocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;
            let list: ID3D12GraphicsCommandList = device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)?;
            list.Close()?;
            let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
            let event = CreateEventW(None, false, false, None)?;
            let readback = committed(device, D3D12_HEAP_TYPE_READBACK, SIZE as u64, D3D12_RESOURCE_STATE_COPY_DEST)?;
            let upload = committed(device, D3D12_HEAP_TYPE_UPLOAD, SIZE as u64, D3D12_RESOURCE_STATE_GENERIC_READ)?;
            let (root, pso) = shader(device)?;
            Ok(Rig {
                device: device.clone(),
                queue,
                allocator,
                list,
                fence,
                done: 0,
                event,
                readback,
                upload,
                root,
                pso,
            })
        }
    }

    /// Records with `record`, runs it and waits (ten seconds at most).
    fn run(&mut self, record: impl FnOnce(&ID3D12GraphicsCommandList)) -> windows::core::Result<()> {
        // SAFETY: the probe's own list and queue, one run at a time.
        unsafe {
            self.allocator.Reset()?;
            self.list.Reset(&self.allocator, None)?;
            record(&self.list);
            self.list.Close()?;
            self.queue.ExecuteCommandLists(&[Some(self.list.cast()?)]);
            self.done += 1;
            self.queue.Signal(&self.fence, self.done)?;
            self.fence.SetEventOnCompletion(self.done, self.event)?;
            if WaitForSingleObject(self.event, 10_000).0 != 0 {
                return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
            }
            self.device.GetDeviceRemovedReason()
        }
    }

    /// What the card reads of `size` bytes of `resource` from `offset`.
    fn fetch(&mut self, resource: &ID3D12Resource, offset: u64, size: usize) -> windows::core::Result<Vec<u8>> {
        let back = self.readback.clone();
        // SAFETY: two live buffers, the range inside both.
        self.run(|list| unsafe { list.CopyBufferRegion(&back, 0, resource, offset, size as u64) })?;
        let mut data = std::ptr::null_mut();
        // SAFETY: the mapping holds the readback buffer's bytes until unmapped.
        unsafe {
            self.readback.Map(0, None, Some(&mut data))?;
            let bytes = std::slice::from_raw_parts(data.cast::<u8>(), size).to_vec();
            self.readback.Unmap(0, None);
            Ok(bytes)
        }
    }

    /// The GPU copies `bytes` into `offset` of `resource` (the copy engine's
    /// write into the memory).
    fn store(&mut self, resource: &ID3D12Resource, offset: u64, bytes: &[u8]) -> windows::core::Result<()> {
        let mut data = std::ptr::null_mut();
        // SAFETY: the upload buffer is `SIZE` bytes; the mapping holds it until
        // unmapped, and the copy is recorded after the unmap.
        unsafe {
            self.upload.Map(0, None, Some(&mut data))?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
            self.upload.Unmap(0, None);
            let up = self.upload.clone();
            self.run(|list| list.CopyBufferRegion(resource, offset, &up, 0, bytes.len() as u64))
        }
    }

    /// The compute shader: `dst = src ^ 0xFFFFFFFF`, over `bytes` of each.
    fn shade(&mut self, src: u64, dst: u64, bytes: u64) -> windows::core::Result<()> {
        let groups = (bytes / 4).div_ceil(64) as u32;
        let (root, pso) = (self.root.clone(), self.pso.clone());
        // SAFETY: the probe's own list; both addresses name live buffers.
        self.run(|list| unsafe {
            list.SetComputeRootSignature(&root);
            list.SetPipelineState(&pso);
            list.SetComputeRootUnorderedAccessView(0, src);
            list.SetComputeRootUnorderedAccessView(1, dst);
            list.Dispatch(groups, 1, 1);
        })
    }
}

fn committed(device: &ID3D12Device, kind: D3D12_HEAP_TYPE, size: u64, state: D3D12_RESOURCE_STATES) -> windows::core::Result<ID3D12Resource> {
    let heap = D3D12_HEAP_PROPERTIES { Type: kind, ..Default::default() };
    let mut buffer: Option<ID3D12Resource> = None;
    // SAFETY: plain creation.
    unsafe { device.CreateCommittedResource(&heap, D3D12_HEAP_FLAG_NONE, &buffer_desc(size, D3D12_RESOURCE_FLAG_NONE), state, None, &mut buffer)? };
    buffer.ok_or_else(windows::core::Error::empty)
}

/// A root signature of two root unordered-access views (u0, u1), serialized
/// here rather than taken from the shader's own `[RootSignature]`: under Wine
/// the blob's signature is refused, and this description is the same on both.
fn root_signature(device: &ID3D12Device) -> windows::core::Result<ID3D12RootSignature> {
    let parameters = [
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
            Anonymous: D3D12_ROOT_PARAMETER_0 { Descriptor: D3D12_ROOT_DESCRIPTOR { ShaderRegister: 0, RegisterSpace: 0 } },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
        },
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
            Anonymous: D3D12_ROOT_PARAMETER_0 { Descriptor: D3D12_ROOT_DESCRIPTOR { ShaderRegister: 1, RegisterSpace: 0 } },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
        },
    ];
    let desc = D3D12_VERSIONED_ROOT_SIGNATURE_DESC {
        Version: D3D_ROOT_SIGNATURE_VERSION_1_0,
        Anonymous: D3D12_VERSIONED_ROOT_SIGNATURE_DESC_0 {
            Desc_1_0: D3D12_ROOT_SIGNATURE_DESC {
                NumParameters: parameters.len() as u32,
                pParameters: parameters.as_ptr(),
                NumStaticSamplers: 0,
                pStaticSamplers: std::ptr::null(),
                Flags: D3D12_ROOT_SIGNATURE_FLAG_NONE,
            },
        },
    };
    let mut blob: Option<ID3DBlob> = None;
    let mut error: Option<ID3DBlob> = None;
    // SAFETY: a description of ours; the blob's bytes outlive the call below.
    unsafe {
        D3D12SerializeVersionedRootSignature(&desc, &mut blob, Some(&mut error))
            .map_err(|e| windows::core::Error::new(e.code(), format!("{} {}", e.message().trim(), blob_text(&error))))?;
        let blob = blob.ok_or_else(windows::core::Error::empty)?;
        let bytes = std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize());
        device.CreateRootSignature(0, bytes)
    }
}

fn blob_text(blob: &Option<ID3DBlob>) -> String {
    match blob {
        // SAFETY: the blob's own bytes, as a string of the compiler's.
        Some(b) => unsafe {
            let bytes = std::slice::from_raw_parts(b.GetBufferPointer().cast::<u8>(), b.GetBufferSize());
            String::from_utf8_lossy(bytes).trim().to_owned()
        },
        None => String::new(),
    }
}

/// The compute pipeline: the shader above on `root_signature`'s signature.
fn shader(device: &ID3D12Device) -> windows::core::Result<(ID3D12RootSignature, ID3D12PipelineState)> {
    let mut code: Option<ID3DBlob> = None;
    let mut error: Option<ID3DBlob> = None;
    let root = root_signature(device)?;
    // SAFETY: the source and names live through the call; then the blob's own bytes.
    unsafe {
        D3DCompile(SHADER.as_ptr().cast(), SHADER.len(), s!("seccommit-probe"), None, None, s!("main"), s!("cs_5_1"), 0, 0, &mut code, Some(&mut error))
            .map_err(|e| windows::core::Error::new(e.code(), format!("{} {}", e.message().trim(), blob_text(&error))))?;
        let code = code.ok_or_else(windows::core::Error::empty)?;
        let bytes = std::slice::from_raw_parts(code.GetBufferPointer().cast::<u8>(), code.GetBufferSize());
        let pso = device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(&root),
            CS: D3D12_SHADER_BYTECODE { pShaderBytecode: bytes.as_ptr().cast(), BytecodeLength: bytes.len() },
            ..Default::default()
        })?;
        Ok((root, pso))
    }
}

/// A buffer of `size` bytes at `offset` (a multiple of 64 KiB) of an opened
/// heap, in the common state. The flags are tried widest first, as
/// `victory-gpu`'s `place` sets them for a heap opened from an address.
fn place(device: &ID3D12Device, heap: &ID3D12Heap, offset: u64, size: u64) -> windows::core::Result<ID3D12Resource> {
    let tries = [
        (D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, "cross-adapter, unordered access"),
        (D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, "unordered access"),
        (D3D12_RESOURCE_FLAG_NONE, "no flags"),
    ];
    let mut last = None;
    for (flags, what) in tries {
        let mut placed: Option<ID3D12Resource> = None;
        // SAFETY: plain creation in a live heap.
        let made = unsafe { device.CreatePlacedResource(heap, offset, &buffer_desc(size, flags), D3D12_RESOURCE_STATE_COMMON, None, &mut placed) };
        match (made, placed) {
            (Ok(()), Some(buffer)) => {
                println!("  a buffer placed in it ({what}): ok");
                return Ok(buffer);
            }
            (Err(e), _) => {
                println!("  a buffer placed in it ({what}): REFUSED {:#010x} ({})", e.code().0, e.message().trim());
                last = Some(e);
            }
            (Ok(()), None) => last = Some(windows::core::Error::empty()),
        }
    }
    Err(last.unwrap_or_else(windows::core::Error::empty))
}

pub(crate) fn main() {
    let mut report = Report::default();
    // SAFETY (the whole run): the probe's own sections, views and device.
    unsafe { run(&mut report) };
    let (reads, writes, later, outlives, guards) =
        (report.reads, report.writes, report.later, report.outlives, report.guards);
    report.say(format!(
        "VERDICT: section SEC_COMMIT + OpenExistingHeapFromFileMapping: reads {reads}, writes {writes}, later CPU writes {later}, outlives views {outlives}, guards still work {guards}"
    ));
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn run(report: &mut Report) {
    // The adapter and the device.
    let factory: IDXGIFactory1 = match CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) {
        Ok(f) => f,
        Err(e) => {
            report.say(format!("no DXGI factory: {e} (run under the launcher's Wine?)"));
            return;
        }
    };
    for i in 0..4 {
        match unsafe { factory.EnumAdapters1(i) } {
            Ok(adapter) => {
                let desc = unsafe { adapter.GetDesc1() };
                if let Some(d) = desc.as_ref().ok().copied() {
                    let name: String = d.Description.iter().take_while(|c| **c != 0).map(|c| char::from_u32(*c as u32).unwrap_or('?')).collect();
                    report.say(format!("adapter {i}: {name} (vendor {:#06x}, device {:#06x}, vram {} MB)", d.VendorId, d.DeviceId, d.DedicatedVideoMemory >> 20));
                }
            }
            Err(_) => break,
        }
    }
    let mut device: Option<ID3D12Device> = None;
    match unsafe { D3D12CreateDevice(None, D3D_FEATURE_LEVEL_12_0, &mut device) } {
        Ok(()) => {}
        Err(e) => {
            report.say(format!("no D3D12 device: {e}"));
            return;
        }
    }
    let device = match device {
        Some(d) => d,
        None => {
            report.say("no D3D12 device".to_owned());
            return;
        }
    };
    let device3 = device.cast::<ID3D12Device3>();
    let device13 = device.cast::<ID3D12Device13>();
    report.say(format!(
        "device: ID3D12Device3 (OpenExistingHeapFromAddress/FromFileMapping) {}, ID3D12Device13 (a part of an allocation) {}",
        device3.is_ok(),
        device13.is_ok()
    ));
    let mut rig = match Rig::new(&device) {
        Ok(rig) => rig,
        Err(e) => {
            report.say(format!("no queue or shader of its own: {e}"));
            return;
        }
    };
    report.say("rig: queue, fence, readback, upload and the compute shader ready".to_owned());

    // 1. The F-path: a section made committed, its handle opened as a heap.
    let handle = match unsafe { CreateFileMappingW(HANDLE(-1isize as *mut c_void), None, PAGE_READWRITE | SEC_COMMIT, 0, SIZE as u32, None) } {
        Ok(h) => h,
        Err(e) => {
            report.say(format!("a section made committed: REFUSED {:#010x} ({e})", e.code().0));
            return;
        }
    };
    let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, SIZE) };
    if view.Value.is_null() {
        report.say("a section made committed: no view of it".to_owned());
        let _ = unsafe { CloseHandle(handle) };
        return;
    }
    let memory = view.Value.cast::<u8>();
    report.say(format!("1. a section of {} KiB made SEC_COMMIT, a view of it at {:p}", SIZE >> 10, memory));
    // `PROBE_HOLD_SECS`: wait here, so that another process can look at what
    // the section's pages are backed by (`/proc/<pid>/maps`) while it stands.
    if let Ok(secs) = std::env::var("PROBE_HOLD_SECS").unwrap_or_default().parse::<u64>() {
        report.say(format!("   holding the section for {secs} s (pid {})", std::process::id()));
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }

    let opened = device3.as_ref().ok().map(|d| unsafe { d.OpenExistingHeapFromFileMapping::<ID3D12Heap>(handle) });
    match &opened {
        Some(Ok(heap)) => {
            report.say("   OpenExistingHeapFromFileMapping: ok".to_owned());
            report.say(format!("   {}", describe(heap)));
        }
        Some(Err(e)) => {
            report.say(format!("   OpenExistingHeapFromFileMapping: REFUSED {:#010x} ({})", e.code().0, e.message().trim()));
            report.say("   the F-path stops here: no heap, nothing to place".to_owned());
        }
        None => report.say("   OpenExistingHeapFromFileMapping: the device is not an ID3D12Device3".to_owned()),
    }
    let placed = match &opened {
        Some(Ok(heap)) => {
            let buffer = place(&device, heap, 0, SIZE as u64);
            match buffer {
                Ok(b) => Some(b),
                Err(e) => {
                    report.say(format!("   no buffer in it: {e}"));
                    None
                }
            }
        }
        _ => None,
    };

    if let Some(buffer) = &placed {
        let gpu = unsafe { buffer.GetGPUVirtualAddress() };
        report.say(format!("   the buffer's GPU address: {gpu:#x}"));

        // (a) The card reads what the CPU wrote.
        fill(memory, SIZE, 1);
        let want = pattern(SIZE, 1);
        let got = rig.fetch(buffer, 0, SIZE);
        let reads_copy = match &got {
            Ok(bytes) => {
                let said = judge(&want, bytes);
                report.say(format!("   (a) the card reads the CPU's bytes (copy engine): {said}"));
                said == "OK"
            }
            Err(e) => {
                report.say(format!("   (a) the card reads the CPU's bytes (copy engine): the copy was REFUSED ({e})"));
                false
            }
        };
        // The same through a compute shader: the bytes are read by a shader,
        // as title memory would be.
        let scratch = committed(&device, D3D12_HEAP_TYPE_DEFAULT, SIZE as u64, D3D12_RESOURCE_STATE_COMMON).ok();
        let mut reads_shader = false;
        if let Some(scratch) = &scratch {
            let scratch_gpu = unsafe { scratch.GetGPUVirtualAddress() };
            let flipped: Vec<u8> = want.iter().map(|b| !*b).collect();
            let ran = rig.shade(gpu, scratch_gpu, SIZE as u64);
            let said = match (&ran, rig.fetch(scratch, 0, SIZE)) {
                (Ok(()), Ok(bytes)) => judge(&flipped, &bytes),
                (Err(e), _) => format!("the dispatch was REFUSED ({e})"),
                (_, Err(e)) => format!("the readback was REFUSED ({e})"),
            };
            report.say(format!("   (a2) a compute shader reads the CPU's bytes: {said}"));
            reads_shader = said == "OK";
        }
        report.reads = reads_copy && reads_shader;

        // (b) The card writes; the CPU reads at the view's address.
        let written: Vec<u8> = pattern(SIZE, 90);
        let stored = rig.store(buffer, 0, &written);
        let after_copy = unsafe { std::slice::from_raw_parts(memory, SIZE) } == written.as_slice();
        report.say(format!(
            "   (b) the card's write (copy engine) read at the view's address after the fence: {}",
            match (&stored, after_copy) {
                (Err(e), _) => format!("the copy was REFUSED ({e})"),
                (_, true) => "OK".to_owned(),
                (_, false) => judge(&written, unsafe { std::slice::from_raw_parts(memory, SIZE) }),
            }
        ));
        // The shader's own writes, into the same memory (in place).
        let flipped: Vec<u8> = written.iter().map(|b| !*b).collect();
        let shaded = rig.shade(gpu, gpu, SIZE as u64);
        let shader_ok = shaded.is_ok() && unsafe { std::slice::from_raw_parts(memory, SIZE) } == flipped.as_slice();
        report.say(format!(
            "   (b2) the card's write by a compute shader (in place) read at the view's address after the fence: {}",
            match (&shaded, shader_ok) {
                (Err(e), _) => format!("the dispatch was REFUSED ({e})"),
                (_, true) => "OK".to_owned(),
                (_, false) => "WRONG (the CPU's bytes are not the shader's)".to_owned(),
            }
        ));
        report.writes = after_copy && shader_ok;
        // (c) The CPU writes afterwards, with no call between: does the card see it?
        let later = pattern(SIZE, 7);
        unsafe { std::ptr::copy_nonoverlapping(later.as_ptr(), memory, SIZE) };
        let got = rig.fetch(buffer, 0, SIZE);
        let later_ok = match &got {
            Ok(bytes) => {
                let said = judge(&later, bytes);
                report.say(format!("   (c) the CPU writes the view, then the card reads it (no flush between): {said}"));
                said == "OK"
            }
            Err(e) => {
                report.say(format!("   (c) the CPU writes the view, then the card reads it: the copy was REFUSED ({e})"));
                false
            }
        };
        // And with an explicit flush, in case the host wants one.
        let flushed = pattern(SIZE, 8);
        unsafe { std::ptr::copy_nonoverlapping(flushed.as_ptr(), memory, SIZE) };
        let flush = unsafe { FlushViewOfFile(memory as *const c_void, SIZE) };
        let got = rig.fetch(buffer, 0, SIZE);
        let flush_ok = match &got {
            Ok(bytes) => judge(&flushed, bytes),
            Err(e) => format!("the copy was REFUSED ({e})"),
        };
        report.say(format!("   (c2) the same with FlushViewOfFile before it ({}): {flush_ok}", hr(&flush)));
        report.later = later_ok;

        // (d) The guards' calls on the view, with the heap standing.
        let mut old = PAGE_PROTECTION_FLAGS(0);
        let read_only = unsafe { VirtualProtect(memory as *const c_void, K64, PAGE_READONLY, &mut old) };
        let mut read_while_guarded = false;
        let mut back = Err(windows::core::Error::empty());
        if read_only.is_ok() {
            let want = pattern(K64, 8);
            read_while_guarded = rig.fetch(buffer, 0, K64).is_ok_and(|bytes| bytes == want);
            let prev = old;
            back = unsafe { VirtualProtect(memory as *const c_void, K64, prev, &mut old) };
        }
        let decommit = unsafe { VirtualFree(memory.cast::<c_void>(), K64, MEM_DECOMMIT) };
        let recommitted = if decommit.is_ok() {
            !unsafe { VirtualAlloc(Some(memory as *const c_void), K64, MEM_COMMIT, PAGE_READWRITE) }.is_null()
        } else {
            false
        };
        report.say(format!(
            "   (d) the guards' calls on the view under the heap: VirtualProtect read-only {}, the card still reading the page {read_while_guarded}, and back {}; VirtualFree decommit of one page {}, committed again {recommitted}",
            hr(&read_only),
            hr(&back),
            hr(&decommit)
        ));
        report.guards = read_only.is_ok() && read_while_guarded;

        // (e) The heap outlives the view: unmap, the card writes, map again.
        let kept = pattern(SIZE, 33);
        let unmapped = unsafe { UnmapViewOfFile(view) };
        let stored = rig.store(buffer, 0, &kept);
        let again = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, SIZE) };
        let shows = !again.Value.is_null() && unsafe { std::slice::from_raw_parts(again.Value.cast::<u8>(), SIZE) } == kept.as_slice();
        report.say(format!(
            "   (e) the view unmapped ({}), the card writes, a view mapped afterwards shows it ({}): {shows}",
            hr(&unmapped),
            hr(&stored)
        ));
        report.outlives = shows;
        if !again.Value.is_null() {
            let _ = unsafe { UnmapViewOfFile(again) };
        }

        // (f) What it costs: the card copying the memory out, against a
        // shared buffer of today's kind.
        let ms = |rig: &mut Rig, from: &ID3D12Resource| -> Option<f64> {
            let back = rig.readback.clone();
            let mut best = f64::MAX;
            for _ in 0..50 {
                let began = std::time::Instant::now();
                rig.run(|list| unsafe { list.CopyBufferRegion(&back, 0, from, 0, SIZE as u64) }).ok()?;
                best = best.min(began.elapsed().as_secs_f64() * 1000.0);
            }
            Some(best)
        };
        let in_place = ms(&mut rig, buffer);
        let today = committed(&device, D3D12_HEAP_TYPE_DEFAULT, SIZE as u64, D3D12_RESOURCE_STATE_COPY_SOURCE).ok();
        let shape = today.as_ref().and_then(|b| ms(&mut rig, b));
        let format_ms = |v: Option<f64>| v.map_or("?".to_owned(), |v| format!("{v:.3}"));
        report.say(format!(
            "   (f) the card copies {} KiB out of the section in place: {} ms; out of a buffer in the card's memory of today's kind: {} ms",
            SIZE >> 10,
            format_ms(in_place),
            format_ms(shape)
        ));
    }

    // 2. The contrast: SEC_RESERVE, as `xmem` made the pool before.
    let reserved = unsafe { CreateFileMappingW(HANDLE(-1isize as *mut c_void), None, PAGE_READWRITE | SEC_RESERVE, 0, SIZE as u32, None) };
    if let Ok(handle) = reserved {
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, SIZE) };
        let memory = view.Value.cast::<u8>();
        if !memory.is_null() {
            let committed = unsafe { VirtualAlloc(Some(memory.cast()), SIZE, MEM_COMMIT, PAGE_READWRITE) };
            report.say(format!(
                "2. a section made SEC_RESERVE, a view of it at {memory:p}: committed by VirtualAlloc {}",
                if committed.is_null() { "REFUSED" } else { "ok" }
            ));
            fill(memory, SIZE, 3);
            let opened = device3.as_ref().ok().map(|d| unsafe { d.OpenExistingHeapFromFileMapping::<ID3D12Heap>(handle) });
            report.say(format!(
                "   OpenExistingHeapFromFileMapping on a reserved section: {}",
                match &opened {
                    Some(r) => hr(r),
                    None => "no ID3D12Device3".to_owned(),
                }
            ));
            // And the address path on the same view: the known bad one.
            if let Some(Ok(heap)) = device13.as_ref().ok().map(|d| unsafe { d.OpenExistingHeapFromAddress1::<ID3D12Heap>(memory.cast(), SIZE) }) {
                let want = pattern(SIZE, 3);
                let said = match place(&device, &heap, 0, SIZE as u64).and_then(|b| rig.fetch(&b, 0, SIZE)) {
                    Ok(bytes) => judge(&want, &bytes),
                    Err(e) => format!("no buffer or no copy ({e})"),
                };
                report.say(format!("   a heap on the reserved view by OpenExistingHeapFromAddress1: the card reads {said}"));
            }
            let _ = unsafe { UnmapViewOfFile(view) };
        }
        let _ = unsafe { CloseHandle(handle) };
    } else {
        report.say("2. a section made SEC_RESERVE: REFUSED".to_owned());
    }

    // 3. The known good path: private memory, a part of it by address.
    let private = unsafe { VirtualAlloc(None, SIZE, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE).cast::<u8>() };
    if !private.is_null() {
        fill(private, SIZE, 5);
        let want = pattern(SIZE, 5);
        let opened = device13.as_ref().ok().map(|d| unsafe { d.OpenExistingHeapFromAddress1::<ID3D12Heap>(private.cast(), SIZE) });
        report.say(format!(
            "3. private VirtualAlloc memory, a part of it by address (OpenExistingHeapFromAddress1): {}",
            match &opened {
                Some(r) => hr(r),
                None => "no ID3D12Device13".to_owned(),
            }
        ));
        if let Some(Ok(heap)) = opened {
            let buffer = place(&device, &heap, 0, SIZE as u64);
            if let Ok(buffer) = &buffer {
                let said = match rig.fetch(buffer, 0, SIZE) {
                    Ok(bytes) => judge(&want, &bytes),
                    Err(e) => format!("no copy ({e})"),
                };
                report.say(format!("   the card reads the CPU's bytes: {said}"));
                let written = pattern(SIZE, 200);
                let cpu = rig.store(buffer, 0, &written).is_ok() && unsafe { std::slice::from_raw_parts(private, SIZE) } == written.as_slice();
                report.say(format!("   the CPU reads the card's write (copy engine) at the memory's address after the fence: {cpu}"));
                // A shader's own writes into the memory, in place, read back by the CPU.
                let flipped: Vec<u8> = written.iter().map(|b| !*b).collect();
                let gpu = unsafe { buffer.GetGPUVirtualAddress() };
                let shaded = rig.shade(gpu, gpu, SIZE as u64);
                let by_shader = shaded.is_ok() && unsafe { std::slice::from_raw_parts(private, SIZE) } == flipped.as_slice();
                report.say(format!(
                    "   the CPU reads the shader's own write (in place) after the fence: {}",
                    match (&shaded, by_shader) {
                        (Err(e), _) => format!("the dispatch was REFUSED ({e})"),
                        (_, true) => "OK".to_owned(),
                        (_, false) => "WRONG (the CPU's bytes are not the shader's)".to_owned(),
                    }
                ));
            }
            let _ = unsafe { VirtualFree(private.cast::<c_void>(), 0, MEM_RELEASE) };
        }
    }

    // 4. The known bad path: a committed section's view by address.
    if let Ok(handle) = unsafe { CreateFileMappingW(HANDLE(-1isize as *mut c_void), None, PAGE_READWRITE | SEC_COMMIT, 0, SIZE as u32, None) } {
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, SIZE) };
        let memory = view.Value.cast::<u8>();
        if !memory.is_null() {
            fill(memory, SIZE, 9);
            let want = pattern(SIZE, 9);
            let opened = device13.as_ref().ok().map(|d| unsafe { d.OpenExistingHeapFromAddress1::<ID3D12Heap>(memory.cast(), SIZE) });
            report.say(format!(
                "4. a committed section's view by address (OpenExistingHeapFromAddress1): {}",
                match &opened {
                    Some(r) => hr(r),
                    None => "no ID3D12Device13".to_owned(),
                }
            ));
            if let Some(Ok(heap)) = opened {
                let said = match place(&device, &heap, 0, SIZE as u64).and_then(|b| rig.fetch(&b, 0, SIZE)) {
                    Ok(bytes) => judge(&want, &bytes),
                    Err(e) => format!("no buffer or no copy ({e})"),
                };
                report.say(format!("   the card reads the view's bytes: {said}"));
            }
            let _ = unsafe { UnmapViewOfFile(view) };
        }
        let _ = unsafe { CloseHandle(handle) };
    }
    let _ = unsafe { CloseHandle(handle) };
    let _ = unsafe { UnmapViewOfFile(view) };
}
