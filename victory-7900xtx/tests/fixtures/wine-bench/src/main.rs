//! What the calls behind the write watch and the guards cost on this host, one thread and
//! several at once: `GetWriteWatch`, `VirtualProtect`, `VirtualQuery`, a write fault taken by a
//! vectored handler, a view mapped into a placeholder. See `docs/linux-wine.md`.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const MEM_WRITE_WATCH: u32 = 0x20_0000;
const MEM_RESERVE_PLACEHOLDER: u32 = 0x4_0000;
const MEM_REPLACE_PLACEHOLDER: u32 = 0x4000;
const MEM_PRESERVE_PLACEHOLDER: u32 = 2;
const PAGE_NOACCESS: u32 = 1;
const PAGE_READONLY: u32 = 2;
const PAGE_READWRITE: u32 = 4;
const SEC_COMMIT: u32 = 0x800_0000;
const FILE_MAP_WRITE: u32 = 2;
const WRITE_WATCH_FLAG_RESET: u32 = 1;

#[repr(C)]
struct ExceptionRecord {
    code: u32,
    flags: u32,
    record: *mut ExceptionRecord,
    address: *mut c_void,
    parameters: u32,
    information: [usize; 15],
}

#[repr(C)]
struct ExceptionPointers {
    record: *mut ExceptionRecord,
    context: *mut c_void,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn VirtualAlloc(at: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(at: *mut c_void, size: usize, kind: u32) -> i32;
    fn VirtualProtect(at: *const c_void, size: usize, new: u32, old: *mut u32) -> i32;
    fn VirtualQuery(at: *const c_void, info: *mut [u64; 6], len: usize) -> usize;
    fn GetWriteWatch(flags: u32, base: *const c_void, size: usize, found: *mut usize, count: *mut usize, granularity: *mut u32) -> u32;
    fn CreateFileMappingW(file: isize, attributes: *const c_void, protect: u32, high: u32, low: u32, name: *const u16) -> isize;
    fn MapViewOfFile(mapping: isize, access: u32, high: u32, low: u32, bytes: usize) -> *mut c_void;
    fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*mut ExceptionPointers) -> i32) -> *mut c_void;
    fn GetCurrentProcess() -> isize;
    fn GetModuleHandleA(name: *const u8) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> Option<unsafe extern "C" fn() -> *const u8>;
}

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn VirtualAlloc2(process: isize, at: *mut c_void, size: usize, kind: u32, protect: u32, more: *mut c_void, count: u32) -> *mut c_void;
    fn MapViewOfFile3(mapping: isize, process: isize, at: *mut c_void, offset: u64, size: usize, kind: u32, protect: u32, more: *mut c_void, count: u32) -> *mut c_void;
    fn UnmapViewOfFile2(process: isize, at: *mut c_void, flags: u32) -> i32;
}

const BLOCK: usize = 64 << 10;
const KERNEL_PAGE: usize = 4096;
/// Write-watched memory: a piece of it for each thread.
const WATCHED: usize = 1 << 30;
const PIECE: usize = 64 << 20;
/// A view of a section: a piece of it for each thread.
const VIEW: usize = 256 << 20;
const THREADS: usize = 8;

static VIEW_AT: AtomicUsize = AtomicUsize::new(0);
static FAULTS: AtomicU64 = AtomicU64::new(0);

/// A write to a read-only block of the view: write access given back, as the guards do.
unsafe extern "system" fn on_write(info: *mut ExceptionPointers) -> i32 {
    // SAFETY: the system's own record.
    let record = unsafe { &*(*info).record };
    let view = VIEW_AT.load(Relaxed);
    if record.code == 0xC000_0005 && record.information[0] == 1 && (view..view + VIEW).contains(&record.information[1]) {
        let mut old = 0;
        // SAFETY: our view.
        unsafe { VirtualProtect((record.information[1] / BLOCK * BLOCK) as *const _, BLOCK, PAGE_READWRITE, &mut old) };
        FAULTS.fetch_add(1, Relaxed);
        return -1; // EXCEPTION_CONTINUE_EXECUTION
    }
    0
}

fn timed(f: impl FnOnce()) -> u64 {
    let began = Instant::now();
    f();
    began.elapsed().as_nanos() as u64
}

/// `each(thread, round)` returns the nanoseconds of what it measures; the mean of one, for one
/// thread alone and for [`THREADS`] at once (each on memory of its own).
fn bench(name: &str, rounds: usize, per: u64, each: impl Fn(usize, usize) -> u64 + Sync) {
    let run = |threads: usize| -> f64 {
        let total = AtomicU64::new(0);
        std::thread::scope(|s| {
            for t in 0..threads {
                let (each, total) = (&each, &total);
                s.spawn(move || {
                    let mut ns = 0;
                    for round in 0..rounds {
                        ns += each(t, round);
                    }
                    total.fetch_add(ns, Relaxed);
                });
            }
        });
        total.load(Relaxed) as f64 / (threads * rounds) as f64 / per as f64 / 1e3
    };
    run(1); // warm
    let (one, many) = (run(1), run(THREADS));
    println!("{name:<78} {one:>9.2} us  {many:>9.2} us  x{:.1}", many / one);
}

fn main() {
    // SAFETY: the whole of it: plain calls on memory this program makes for itself.
    unsafe {
        let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
        match GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()) {
            Some(version) => println!("Wine {}", std::ffi::CStr::from_ptr(version().cast()).to_string_lossy()),
            None => println!("Windows"),
        }
        let process = GetCurrentProcess();
        let watched = VirtualAlloc(std::ptr::null_mut(), WATCHED, MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH, PAGE_READWRITE) as usize;
        assert_ne!(watched, 0, "no write-watched memory");
        let section = CreateFileMappingW(-1, std::ptr::null(), PAGE_READWRITE | SEC_COMMIT, 0, VIEW as u32, std::ptr::null());
        assert_ne!(section, 0, "no section");
        let view = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, VIEW) as usize;
        assert_ne!(view, 0, "no view");
        VIEW_AT.store(view, Relaxed);
        AddVectoredExceptionHandler(1, on_write);
        let ms = timed(|| {
            for at in (0..WATCHED).step_by(KERNEL_PAGE) {
                *((watched + at) as *mut u8) = 1;
            }
            for at in (0..VIEW).step_by(KERNEL_PAGE) {
                *((view + at) as *mut u8) = 1;
            }
        }) as f64
            / 1e6;
        println!("{} MiB write-watched and {} MiB of a view written for the first time: {ms:.0} ms", WATCHED >> 20, VIEW >> 20);
        let mut found = vec![0usize; WATCHED / KERNEL_PAGE];
        let (mut count, mut granularity) = (found.len(), 0u32);
        let ms = timed(|| {
            GetWriteWatch(WRITE_WATCH_FLAG_RESET, watched as *const _, WATCHED, found.as_mut_ptr(), &mut count, &mut granularity);
        }) as f64
            / 1e6;
        println!("asked and reset, all of it written ({count} pages): {ms:.1} ms");
        let ms = timed(|| {
            count = found.len();
            GetWriteWatch(WRITE_WATCH_FLAG_RESET, watched as *const _, WATCHED, found.as_mut_ptr(), &mut count, &mut granularity);
        }) as f64
            / 1e6;
        println!("asked and reset again, nothing written ({count} pages): {ms:.2} ms");
        drop(found);

        println!("{:<78} {:>12}  {:>12}", "", "1 thread", format!("{THREADS} threads"));
        let ask = |at: usize, len: usize, reset: bool| -> usize {
            thread_local! { static FOUND: std::cell::RefCell<Vec<usize>> = std::cell::RefCell::new(vec![0; PIECE / KERNEL_PAGE]); }
            FOUND.with(|found| {
                let mut found = found.borrow_mut();
                let (mut count, mut granularity) = (found.len(), 0u32);
                let failed = GetWriteWatch(u32::from(reset), at as *const _, len, found.as_mut_ptr(), &mut count, &mut granularity);
                if failed != 0 { usize::MAX } else { count }
            })
        };
        bench("GetWriteWatch with reset, 64 KiB, nothing written", 20_000, 1, |t, _| {
            timed(|| {
                ask(watched + t * PIECE, BLOCK, true);
            })
        });
        bench("GetWriteWatch with reset, 64 MiB, nothing written", 2_000, 1, |t, _| {
            timed(|| {
                ask(watched + t * PIECE, PIECE, true);
            })
        });
        bench("first write to a write-watched page after a reset (256 of them), each", 400, 256, |t, _| {
            timed(|| {
                for k in 0..256 {
                    *((watched + t * PIECE + k * BLOCK) as *mut u8) = 2;
                }
            }) + {
                ask(watched + t * PIECE, PIECE, true);
                0
            }
        });
        bench("GetWriteWatch with reset, 64 MiB, 256 pages written", 400, 1, |t, _| {
            for k in 0..256 {
                *((watched + t * PIECE + k * BLOCK) as *mut u8) = 2;
            }
            timed(|| assert_eq!(ask(watched + t * PIECE, PIECE, true), 256))
        });
        bench("GetWriteWatch without reset, 64 MiB, 256 pages written", 400, 1, |t, round| {
            if round == 0 {
                for k in 0..256 {
                    *((watched + t * PIECE + k * BLOCK) as *mut u8) = 2;
                }
            }
            timed(|| assert_eq!(ask(watched + t * PIECE, PIECE, false), 256))
        });
        bench("GetWriteWatch of a view (refused)", 20_000, 1, |t, _| {
            timed(|| assert_eq!(ask(view + t * (VIEW / THREADS), BLOCK, true), usize::MAX))
        });
        bench("VirtualQuery of a view", 20_000, 1, |t, _| {
            let mut info = [0u64; 6];
            timed(|| {
                VirtualQuery((view + t * (VIEW / THREADS)) as *const _, &mut info, 48);
            })
        });
        bench("VirtualProtect of 64 KiB of a view, read-only and back (the two)", 20_000, 1, |t, round| {
            let at = view + t * (VIEW / THREADS) + round % 256 * BLOCK;
            let mut old = 0;
            timed(|| {
                VirtualProtect(at as *const _, BLOCK, PAGE_READONLY, &mut old);
                VirtualProtect(at as *const _, BLOCK, PAGE_READWRITE, &mut old);
            })
        });
        bench("VirtualProtect of 64 KiB to what it is already (read-write)", 20_000, 1, |t, round| {
            let at = view + t * (VIEW / THREADS) + round % 256 * BLOCK;
            let mut old = 0;
            timed(|| {
                VirtualProtect(at as *const _, BLOCK, PAGE_READWRITE, &mut old);
            })
        });
        bench("a write to a read-only block, opened by the vectored handler", 20_000, 1, |t, round| {
            let at = view + t * (VIEW / THREADS) + round % 256 * BLOCK;
            let mut old = 0;
            VirtualProtect(at as *const _, BLOCK, PAGE_READONLY, &mut old);
            timed(|| *(at as *mut u8) = 3)
        });
        bench("MapViewOfFile3 into a placeholder and UnmapViewOfFile2, 64 KiB (the two)", 5_000, 1, |t, round| {
            thread_local! { static HOLE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
            let hole = HOLE.with(|h| {
                if h.get() == 0 {
                    let p = VirtualAlloc2(0, std::ptr::null_mut(), BLOCK, MEM_RESERVE | MEM_RESERVE_PLACEHOLDER, PAGE_NOACCESS, std::ptr::null_mut(), 0);
                    assert!(!p.is_null(), "no placeholder");
                    h.set(p as usize);
                }
                h.get()
            });
            let offset = (t * (VIEW / THREADS) + round % 256 * BLOCK) as u64;
            timed(|| {
                let p = MapViewOfFile3(section, process, hole as *mut _, offset, BLOCK, MEM_REPLACE_PLACEHOLDER, PAGE_READWRITE, std::ptr::null_mut(), 0);
                assert!(!p.is_null(), "no view in the placeholder");
                UnmapViewOfFile2(process, p, MEM_PRESERVE_PLACEHOLDER);
            })
        });

        // One thread asks about all the write-watched memory in a loop (as a submission's
        // asking does); the others take write faults meanwhile.
        for asking in [false, true] {
            let stop = AtomicBool::new(false);
            let (asks, ask_ns) = (AtomicU64::new(0), AtomicU64::new(0));
            let mut cycle = 0.0;
            std::thread::scope(|s| {
                if asking {
                    s.spawn(|| {
                        let mut found = vec![0usize; WATCHED / KERNEL_PAGE];
                        while !stop.load(Relaxed) {
                            for k in 0..4096 {
                                *((watched + k * 4 * BLOCK) as *mut u8) = 4;
                            }
                            let (mut count, mut granularity) = (found.len(), 0u32);
                            ask_ns.fetch_add(
                                timed(|| {
                                    GetWriteWatch(WRITE_WATCH_FLAG_RESET, watched as *const _, WATCHED, found.as_mut_ptr(), &mut count, &mut granularity);
                                }),
                                Relaxed,
                            );
                            asks.fetch_add(1, Relaxed);
                        }
                    });
                }
                let ns = AtomicU64::new(0);
                std::thread::scope(|s| {
                    for t in 0..4 {
                        let ns = &ns;
                        s.spawn(move || {
                            for round in 0..20_000 {
                                let at = view + t * (VIEW / THREADS) + round % 256 * BLOCK;
                                let mut old = 0;
                                VirtualProtect(at as *const _, BLOCK, PAGE_READONLY, &mut old);
                                ns.fetch_add(timed(|| *(at as *mut u8) = 5), Relaxed);
                            }
                        });
                    }
                });
                cycle = ns.load(Relaxed) as f64 / 80_000.0 / 1e3;
                stop.store(true, Relaxed);
            });
            if asking {
                let n = asks.load(Relaxed).max(1);
                println!(
                    "the same fault on 4 threads while a fifth asks about {} MiB with 4,096 pages written: {cycle:.2} us ({n} askings, {:.2} ms each)",
                    WATCHED >> 20,
                    ask_ns.load(Relaxed) as f64 / n as f64 / 1e6
                );
            } else {
                println!("a write to a read-only block on 4 threads, nothing else running: {cycle:.2} us");
            }
        }
        println!("{} faults taken by the handler", FAULTS.load(Relaxed));

        // The title shows its physical pages one 64 KiB view at a time, tens of thousands of
        // them: what one more view costs with that many there, and what the other threads'
        // faults and protection changes cost while one thread maps and unmaps.
        let hole = VirtualAlloc2(0, std::ptr::null_mut(), BLOCK, MEM_RESERVE | MEM_RESERVE_PLACEHOLDER, PAGE_NOACCESS, std::ptr::null_mut(), 0);
        assert!(!hole.is_null(), "no placeholder");
        let pair = |round: usize| {
            let p = MapViewOfFile3(section, process, hole, (round % 256 * BLOCK) as u64, BLOCK, MEM_REPLACE_PLACEHOLDER, PAGE_READWRITE, std::ptr::null_mut(), 0);
            assert!(!p.is_null(), "no view in the placeholder");
            UnmapViewOfFile2(process, p, MEM_PRESERVE_PLACEHOLDER);
        };
        let mut views = 0usize;
        for wanted in [0usize, 10_000, 30_000, 60_000, 120_000] {
            if wanted > views {
                let n = wanted - views;
                let all = VirtualAlloc2(0, std::ptr::null_mut(), n * BLOCK, MEM_RESERVE | MEM_RESERVE_PLACEHOLDER, PAGE_NOACCESS, std::ptr::null_mut(), 0) as usize;
                assert_ne!(all, 0, "no placeholder for {n} views");
                let ms = timed(|| {
                    for k in 0..n {
                        let at = (all + k * BLOCK) as *mut c_void;
                        if k + 1 < n {
                            VirtualFree(at, BLOCK, MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER);
                        }
                        // (Every other page of the section, so that neighbours are not one mapping to the host.)
                        let p = MapViewOfFile3(section, process, at, (k * 2 % (VIEW / BLOCK) * BLOCK) as u64, BLOCK, MEM_REPLACE_PLACEHOLDER, PAGE_READWRITE, std::ptr::null_mut(), 0);
                        assert!(!p.is_null(), "view {k} of {n} not mapped");
                    }
                }) as f64
                    / 1e6;
                views = wanted;
                println!("{n} more views of 64 KiB mapped in {ms:.0} ms ({:.1} us each)", ms * 1e3 / n as f64);
            }
            let rounds = 2_000;
            let map = timed(|| (0..rounds).for_each(pair)) as f64 / rounds as f64 / 1e3;
            let mut old = 0;
            let protect = timed(|| {
                for round in 0..rounds {
                    let at = view + round % 256 * BLOCK;
                    VirtualProtect(at as *const _, BLOCK, PAGE_READONLY, &mut old);
                    VirtualProtect(at as *const _, BLOCK, PAGE_READWRITE, &mut old);
                }
            }) as f64
                / rounds as f64
                / 1e3;
            // The same while another thread maps and unmaps without a pause.
            let stop = AtomicBool::new(false);
            let maps = AtomicU64::new(0);
            let (mut fault, mut protect_beside, mut ask_beside) = (0.0, 0.0, 0.0);
            std::thread::scope(|s| {
                s.spawn(|| {
                    let other = VirtualAlloc2(0, std::ptr::null_mut(), BLOCK, MEM_RESERVE | MEM_RESERVE_PLACEHOLDER, PAGE_NOACCESS, std::ptr::null_mut(), 0);
                    let mut round = 0;
                    while !stop.load(Relaxed) {
                        let p = MapViewOfFile3(section, process, other, (round % 256 * BLOCK) as u64, BLOCK, MEM_REPLACE_PLACEHOLDER, PAGE_READWRITE, std::ptr::null_mut(), 0);
                        UnmapViewOfFile2(process, p, MEM_PRESERVE_PLACEHOLDER);
                        round += 1;
                    }
                    maps.store(round as u64, Relaxed);
                });
                let rounds = 1_000;
                let began = Instant::now();
                let mut ns = 0;
                for round in 0..rounds {
                    let at = view + round % 256 * BLOCK;
                    VirtualProtect(at as *const _, BLOCK, PAGE_READONLY, &mut old);
                    ns += timed(|| *(at as *mut u8) = 6);
                }
                fault = ns as f64 / rounds as f64 / 1e3;
                protect_beside = (began.elapsed().as_nanos() as u64 - ns) as f64 / rounds as f64 / 1e3;
                ask_beside = timed(|| {
                    for _ in 0..rounds {
                        ask(view, BLOCK, true);
                    }
                }) as f64
                    / rounds as f64
                    / 1e3;
                stop.store(true, Relaxed);
            });
            println!(
                "{views:>6} views there: map and unmap one more {map:>7.1} us; protect and back {protect:.1} us; beside a thread that maps and unmaps: \
                 a fault opened {fault:>7.1} us, one protection change {protect_beside:>7.1} us, a refused asking {ask_beside:>7.1} us"
            );
        }
    }
}
