//! Names the faulting code when title code crashes.
//!
//! A vectored handler sees every exception first, so it only prints the
//! kinds that end a process (access violations and the like, and
//! breakpoints: titles assert with `int3`, fatal without a debugger) as
//! first-chance, since a handler may still take them (ntdll's loader does
//! with its own probes). What nobody handles, of any kind (an uncaught C++
//! exception too), is reported from the unhandled-exception filter
//! (`report_unhandled`, which the runtime calls ahead of the title's own)
//! and appended to the crash log, with the title's last debug output, which
//! often states the reason. Panics of ours go to the crash log too.

use std::ptr::null_mut;
use std::sync::atomic::{
    AtomicBool, AtomicU32, AtomicU64, AtomicUsize,
    Ordering::{Acquire, Relaxed, Release},
};
use windows_sys::Win32::Foundation::{
    EXCEPTION_ACCESS_VIOLATION, EXCEPTION_BREAKPOINT, EXCEPTION_ILLEGAL_INSTRUCTION,
    EXCEPTION_INT_DIVIDE_BY_ZERO, EXCEPTION_PRIV_INSTRUCTION, EXCEPTION_STACK_OVERFLOW, HMODULE,
};

use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, CONTEXT, EXCEPTION_POINTERS, EXCEPTION_RECORD, M128A,
    RtlLookupFunctionEntry, RtlVirtualUnwind, SetUnhandledExceptionFilter, UNW_FLAG_NHANDLER,
};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadStackLimits;

const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
/// Frames listed per report.
pub(crate) const MAX_FRAMES: usize = 40;

/// Memory saved to files when a crash is reported: (address, length, file).
#[cfg(feature = "diagnostics")]
static SAVE: std::sync::OnceLock<Vec<(usize, usize, std::path::PathBuf)>> =
    std::sync::OnceLock::new();

/// Debugging aid: on a reported crash, also write each `[address, address
/// + len)` (readable title memory, e.g. an engine table) to its file.
#[cfg(feature = "diagnostics")]
pub fn save_on_crash(ranges: Vec<(usize, usize, std::path::PathBuf)>) {
    let _ = SAVE.set(ranges);
}

/// The file crash reports are appended to, besides being printed.
static LOG: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Also appends every unhandled-exception report and panic to `file`, so a
/// crash can be read after the console is gone.
pub fn log_crashes_to(file: std::path::PathBuf) {
    if LOG.set(file).is_ok() {
        let print = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            print(info);
            append_to_log(&format!(
                "VIctory: panic on thread {:?}: {info}\n{}",
                std::thread::current().name().unwrap_or("?"),
                title_output()
            ));
        }));
    }
}

/// Appends a failure that is not an exception (the host GPU's) to the crash
/// log, with the title's last debug output, and prints it.
pub fn log_crash(text: &str) {
    let text = format!("{text}
{}", title_output());
    eprint!("{text}");
    append_to_log(&text);
}

fn append_to_log(text: &str) {
    use std::io::Write;
    let Some(file) = LOG.get() else { return };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let opened = std::fs::OpenOptions::new().create(true).append(true).open(file);
    if let Ok(mut f) = opened {
        let _ = write!(f, "unix time {now}, process {}\n{text}\n", std::process::id());
    }
}

/// The title's last lines of debug output, kept for crash reports.
static SAID: std::sync::Mutex<std::collections::VecDeque<String>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());
const SAID_LINES: usize = 8;

/// Keeps a line of the title's debug output for the next crash report.
pub fn note_title_output(line: &str) {
    if line.is_empty() {
        return;
    }
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    if said.len() == SAID_LINES {
        said.pop_front();
    }
    said.push_back(line.to_owned());
}

/// The kept lines, one per report line; nothing if another thread holds
/// them (it may be the one that faulted).
fn title_output() -> String {
    let Ok(said) = SAID.try_lock() else {
        return String::new();
    };
    said.iter().map(|l| format!("  title said: {l}\n")).collect()
}

/// A `VICTORY_...` switch that is on unless it is set to `0` (or `false`,
/// `off`, `no`).
fn switch(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        Err(_) => true,
    }
}

/// Installs the handlers; call once before running title code.
pub fn report_crashes() {
    // Read here, not in the handler: a faulting thread may hold any lock,
    // the heap's included. Two switches: a title can have one race without
    // the other, and a run that turns one off must still be guarded by the
    // other.
    SKIP_NULL_VTABLE.store(switch("VICTORY_SKIP_NULL_VTABLE"), Relaxed);
    SKIP_NULL_CHAIN.store(switch("VICTORY_SKIP_NULL_CHAIN"), Relaxed);
    SKIP_SMALL_WRITE.store(switch("VICTORY_SKIP_SMALL_WRITE"), Relaxed);
    static AT_EXIT: std::sync::Once = std::sync::Once::new();
    // SAFETY: registers a function with the host CRT's exit table; it reads
    // our counters and logs, nothing else.
    AT_EXIT.call_once(|| unsafe {
        atexit(skips_at_exit);
    });
    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the system passes the exception being dispatched.
        unsafe { report_unhandled(info) };
        EXCEPTION_CONTINUE_SEARCH
    }
    // SAFETY: registers handlers that only read the exception they are
    // given; the title's own filter, once it sets one, takes the place of
    // ours and calls `report_unhandled` first.
    unsafe {
        AddVectoredExceptionHandler(0, Some(handler));
        SetUnhandledExceptionFilter(Some(filter));
    }
}

/// What the vectored handler reports as it happens.
const FATAL: [i32; 6] = [
    EXCEPTION_ACCESS_VIOLATION,
    EXCEPTION_ILLEGAL_INSTRUCTION,
    EXCEPTION_PRIV_INSTRUCTION,
    EXCEPTION_INT_DIVIDE_BY_ZERO,
    EXCEPTION_STACK_OVERFLOW,
    EXCEPTION_BREAKPOINT,
];

/// Console CPUs are Zen 2: titles use AMD-only instructions Intel CPUs
/// lack. Does their job instead: `clzero` (`0f 01 fc`) zeroes the 64-byte
/// line at `rax` (see [`clzero`] for the loops around it);
/// `monitorx`/`mwaitx` (`0f 01 fa`/`fb`) are wait hints, so a spin loop
/// around them works with mwaitx as a yield; `rdpru` (`0f 01 fd`) reads
/// MPERF (`ecx` 0) or APERF (1), cycles at the nominal and the actual
/// frequency, and gets the time-stamp counter for both: a core that never
/// changes frequency.
unsafe fn emulate_amd_only(info: *mut EXCEPTION_POINTERS) -> bool {
    // SAFETY: the system passes valid exception pointers; the faulting
    // instruction's bytes are readable, as the CPU just fetched them.
    unsafe {
        let record = &*(*info).ExceptionRecord;
        let context = &mut *(*info).ContextRecord;
        if record.ExceptionCode != EXCEPTION_ILLEGAL_INSTRUCTION {
            return false;
        }
        let code = context.Rip as *const u8;
        match code.cast::<[u8; 3]>().read_unaligned() {
            [0x0f, 0x01, 0xfc] => return clzero(context),
            [0x0f, 0x01, 0xfa] => {}
            [0x0f, 0x01, 0xfb] => std::thread::yield_now(),
            [0x0f, 0x01, 0xfd] => {
                // Flags are left as they are: a Zen 3 changes none, for
                // known counters or not (measured in user mode).
                let known = context.Rcx as u32 <= 1;
                let cycles = if known { std::arch::x86_64::_rdtsc() } else { 0 };
                context.Rax = cycles & 0xffff_ffff;
                context.Rdx = cycles >> 32;
            }
            _ => {
                // Only bytes inside the faulting instruction's page are read: the next page may
                // not be mapped.
                let room = 0x1000 - (context.Rip as usize & 0xfff);
                let mut bytes = [0u8; 32];
                let n = room.min(bytes.len());
                std::ptr::copy_nonoverlapping(code, bytes.as_mut_ptr(), n);
                if let Some(pass) = clwb_loop(&bytes[..n])
                    && let Some(end) =
                        clwb_loop_end(*gpr(context, pass.base), *gpr(context, pass.limit))
                {
                    // A whole write-back loop in one exception (see `clwb_loop`).
                    let limit = *gpr(context, pass.limit);
                    *gpr(context, pass.base) = end;
                    let flags = cmp_flags(end, limit);
                    context.EFlags = context.EFlags & !CMP_FLAGS | flags;
                    context.Rip += pass.len as u64;
                    return true;
                }
                if let Some(len) = clwb_len(&bytes[..n]) {
                    // A write-back hint: the PC's memory is coherent, so it has nothing to do.
                    context.Rip += len as u64;
                    return true;
                }
                let Some(len) = sse4a(code, &mut context.Anonymous.FltSave.XmmRegisters) else {
                    return false;
                };
                context.Rip += len as u64;
                return true;
            }
        }
        context.Rip += 3;
    }
    true
}

/// `clwb m8` (`66 [rex] 0f ae /6`, a memory operand), which Intel CPUs before
/// Ice Lake lack: its length, or `None` for any other instruction.
fn clwb_len(code: &[u8]) -> Option<usize> {
    let at = if *code.get(1)? & 0xf0 == 0x40 { 2 } else { 1 };
    if *code.first()? != 0x66 || code.get(at..at + 2)? != [0x0f, 0xae] {
        return None;
    }
    let modrm = *code.get(at + 2)?;
    let (mode, rm) = (modrm >> 6, modrm & 7);
    if mode == 3 || (modrm >> 3) & 7 != 6 {
        return None;
    }
    let mut len = at + 3;
    let mut base = rm;
    if rm == 4 {
        base = *code.get(len)? & 7;
        len += 1;
    }
    len += match mode {
        0 if rm == 5 || (rm == 4 && base == 5) => 4,
        1 => 1,
        2 => 4,
        _ => 0,
    };
    (len <= code.len()).then_some(len)
}

/// A `clwb` write-back loop as the title's compilers build it, one cache line a
/// pass (`base` and `limit` any registers):
///
/// ```text
/// top: clwb [base]
///      add  base, 0x40
///      cmp  base, limit
///      jbe  top
/// ```
///
/// An exception costs about a microsecond and the title writes back whole
/// buffers this way while it loads, so the loop is done in one: `clwb` has
/// nothing to do on the PC, `base` ends past `limit` as the last pass leaves it.
struct ClwbLoop {
    base: u8,
    limit: u8,
    /// Bytes from `clwb` to the instruction after `jbe`.
    len: usize,
}

/// The `base` value left after a `clwb_loop` finishes without wrapping past
/// `u64::MAX`, or `None` if `add base, 0x40` would overflow or wrap (in which
/// case we fall through to single-instruction `clwb` emulation).
fn clwb_loop_end(base: u64, limit: u64) -> Option<u64> {
    let step = if base <= limit {
        ((limit - base) / 64 + 1).checked_mul(64)?
    } else {
        64
    };
    let end = base.checked_add(step)?;
    (end > limit).then_some(end)
}

fn clwb_loop(code: &[u8]) -> Option<ClwbLoop> {
    // clwb [base]: 66 [40|41] 0f ae, ModRM mod 00, reg 6, no SIB or displacement.
    let (rex, at) = match code.get(1)? {
        0x40 | 0x41 => (code[1], 2),
        _ => (0, 1),
    };
    if *code.first()? != 0x66 || code.get(at..at + 2)? != [0x0f, 0xae] {
        return None;
    }
    let modrm = *code.get(at + 2)?;
    let rm = modrm & 7;
    if modrm >> 6 != 0 || (modrm >> 3) & 7 != 6 || rm == 4 || rm == 5 {
        return None;
    }
    let base = rm | (rex & 1) << 3;
    // add base, 0x40: REX.W(+B) 83 /0 ib.
    let add = at + 3;
    if *code.get(add..add + 4)? != [0x48 | base >> 3, 0x83, 0xc0 | (base & 7), 0x40] {
        return None;
    }
    // cmp base, limit: REX.W 39 /r with base in r/m.
    let cmp = add + 4;
    let &[cmp_rex @ 0x48..=0x4f, 0x39, cmp_modrm] = code.get(cmp..cmp + 3)? else {
        return None;
    };
    if cmp_modrm >> 6 != 3 || (cmp_modrm & 7 | (cmp_rex & 1) << 3) != base {
        return None;
    }
    let limit = (cmp_modrm >> 3) & 7 | (cmp_rex & 4) << 1;
    // jbe top.
    let jump = cmp + 3;
    let len = jump + 2;
    if *code.get(jump)? != 0x76 || i64::from(*code.get(jump + 1)? as i8) != -(len as i64) {
        return None;
    }
    (limit != base).then_some(ClwbLoop { base, limit, len })
}

/// `clzero` at `context.Rip`. An exception costs about a microsecond and
/// titles clear tens of kilobytes at a time, a line per pass, so the two
/// loops compilers build around the instruction are done whole in one
/// exception, leaving registers and flags as their last pass would (`a`
/// and `b` any registers but `rax`):
///
/// ```text
/// top: clzero                 top: mov rax, a
///      add rax, 0x40               add a, 0x40
///      cmp rax, b                  clzero
///      jb  top                     cmp a, b
///                                  jb  top
/// ```
///
/// Anything else around it is one line and the next instruction.
///
/// # Safety
/// `context` is the state of a thread that faulted at a `clzero`; the
/// lines its registers name are the title's to zero.
unsafe fn clzero(context: &mut CONTEXT) -> bool {
    // SAFETY: (every use) a line the title asked to zero.
    let zero = |rax: u64| unsafe { ((rax & !63) as *mut u8).write_bytes(0, 64) };
    let at = context.Rip as usize;
    // The loops' other instructions are read only where that stays inside
    // the faulting instruction's page (the next one may not be mapped).
    let page = at & 0xfff;
    if page + 12 <= 0x1000 {
        // SAFETY: inside the page the CPU fetched the instruction from.
        let after = unsafe { &*((at + 3) as *const [u8; 9]) };
        if let [0x48, 0x83, 0xc0, 0x40, cmp @ .., 0x72, 0xf4] = after
            && let Some((0, limit)) = compare_registers(cmp)
            && limit != 0
        {
            let limit = *gpr(context, limit);
            loop {
                zero(context.Rax);
                context.Rax = context.Rax.wrapping_add(0x40);
                if context.Rax >= limit {
                    break;
                }
            }
            context.EFlags = context.EFlags & !CMP_FLAGS | cmp_flags(context.Rax, limit);
            context.Rip += 12;
            return true;
        }
    }
    if (7..=0x1000 - 8).contains(&page) {
        // SAFETY: inside the page the CPU fetched the instruction from.
        let (before, after) =
            unsafe { (&*((at - 7) as *const [u8; 7]), &*((at + 3) as *const [u8; 5])) };
        if let [rex @ (0x48 | 0x4c), 0x89, load, add_rex @ (0x48 | 0x49), 0x83, add, 0x40] = *before
            && let [cmp @ .., 0x72, 0xf1] = after
            && let Some((a, b)) = compare_registers(cmp)
            && load & 0xc7 == 0xc0
            && a == (load >> 3) & 7 | (rex & 4) << 1
            && add & 0xf8 == 0xc0
            && a == add & 7 | (add_rex & 1) << 3
            && a != 0
            && b != 0
        {
            let limit = *gpr(context, b);
            loop {
                zero(context.Rax);
                let next = *gpr(context, a);
                if next >= limit {
                    break;
                }
                context.Rax = next;
                *gpr(context, a) = next.wrapping_add(0x40);
            }
            context.EFlags =
                context.EFlags & !CMP_FLAGS | cmp_flags(*gpr(context, a), limit);
            context.Rip += 8;
            return true;
        }
    }
    zero(context.Rax);
    context.Rip += 3;
    true
}

/// `cmp rm, reg` between 64-bit registers (`REX.W 39 /r`): (`rm`, `reg`).
fn compare_registers(code: &[u8]) -> Option<(u8, u8)> {
    let &[rex @ 0x48..=0x4f, 0x39, modrm] = code else {
        return None;
    };
    (modrm >> 6 == 3).then_some((modrm & 7 | (rex & 1) << 3, (modrm >> 3) & 7 | (rex & 4) << 1))
}

/// The general register numbered `n` as instructions encode it.
fn gpr(context: &mut CONTEXT, n: u8) -> &mut u64 {
    match n & 15 {
        0 => &mut context.Rax,
        1 => &mut context.Rcx,
        2 => &mut context.Rdx,
        3 => &mut context.Rbx,
        4 => &mut context.Rsp,
        5 => &mut context.Rbp,
        6 => &mut context.Rsi,
        7 => &mut context.Rdi,
        8 => &mut context.R8,
        9 => &mut context.R9,
        10 => &mut context.R10,
        11 => &mut context.R11,
        12 => &mut context.R12,
        13 => &mut context.R13,
        14 => &mut context.R14,
        _ => &mut context.R15,
    }
}

/// The status flags `cmp a, b` leaves (CF, PF, AF, ZF, SF, OF) as `EFLAGS` bits.
pub(crate) fn cmp_flags(a: u64, b: u64) -> u32 {
    let r = a.wrapping_sub(b);
    u32::from(a < b)
        | u32::from((r as u8).count_ones().is_multiple_of(2)) << 2
        | (((a ^ b ^ r) >> 4) as u32 & 1) << 4
        | u32::from(r == 0) << 6
        | ((r >> 63) as u32) << 7
        | ((((a ^ b) & (a ^ r)) >> 63) as u32) << 11
}
pub(crate) const CMP_FLAGS: u32 = 0x8d5;

/// SSE4a's `extrq`/`insertq` (register forms, the only ones there are) on
/// the saved XMM registers; returns the instruction's length.
unsafe fn sse4a(code: *const u8, xmm: &mut [M128A; 16]) -> Option<usize> {
    // SAFETY: the caller's code bytes are readable up to the instruction's end.
    let b = |i: usize| unsafe { code.add(i).read() };
    let prefix = b(0);
    let (rex, at) = if b(1) & 0xf0 == 0x40 {
        (b(1), 2)
    } else {
        (0, 1)
    };
    if !matches!(prefix, 0x66 | 0xf2) || b(at) != 0x0f || !matches!(b(at + 1), 0x78 | 0x79) {
        return None;
    }
    let modrm = b(at + 2);
    if modrm >> 6 != 3 {
        return None;
    }
    let dst = usize::from((modrm >> 3) & 7 | (rex & 4) << 1);
    let src = usize::from(modrm & 7 | (rex & 1) << 3);
    let immediate = b(at + 1) == 0x78;
    let (fields, len) = if immediate {
        ((b(at + 3), b(at + 4)), at + 5)
    } else {
        // extrq reads its fields from src's low half, insertq from its high.
        let f = if prefix == 0x66 { xmm[src].Low } else { xmm[src].High as u64 };
        ((f as u8, (f >> 8) as u8), at + 3)
    };
    let (n, at_bit) = (u32::from(fields.0 & 63), u32::from(fields.1 & 63));
    let bits = if n == 0 { 64 } else { n };
    let mask = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
    if prefix == 0x66 {
        // extrq: register form takes its fields from src, works on dst.
        if immediate && (modrm >> 3) & 7 != 0 {
            return None;
        }
        let target = if immediate { src } else { dst };
        xmm[target].Low = (xmm[target].Low >> at_bit) & mask;
    } else {
        let value = (xmm[src].Low & mask) << at_bit;
        xmm[dst].Low = xmm[dst].Low & !(mask << at_bit) | value;
    }
    Some(len)
}

/// `VICTORY_SKIP_NULL_VTABLE` (on unless it is `0`): a call through a null
/// vtable pointer is skipped instead of ending the process (see
/// [`skip_null_vtable`]). Read once by [`report_crashes`], so the handler
/// reads no environment on a thread that may hold the heap's lock.
static SKIP_NULL_VTABLE: AtomicBool = AtomicBool::new(true);

/// `VICTORY_SKIP_NULL_CHAIN` (on unless it is `0`): a load through a null
/// pointer in a chain of them is skipped instead of ending the process (see
/// [`skip_null_chain`]). Its own switch, on its own: the two faults are
/// different shapes of the title's lifecycle races, and a run may want one
/// skipped and the other reported. Read once by [`report_crashes`], as
/// above.
static SKIP_NULL_CHAIN: AtomicBool = AtomicBool::new(true);

/// The skips the guard made, printed at exit and put in crash reports: a
/// title that limps must not read like one that never faltered.
static NULL_VTABLE_SKIPS: AtomicU64 = AtomicU64::new(0);

/// The loads [`skip_null_chain`] skipped, counted with the vtable skips in
/// the same accounting (at exit, and in every crash report).
static NULL_CHAIN_SKIPS: AtomicU64 = AtomicU64::new(0);

/// `VICTORY_SKIP_SMALL_WRITE` (on unless it is `0`): a store through a small
/// non-null pointer — a size or a count where a pointer belongs — is skipped
/// instead of ending the process (see [`skip_small_write`]). Its own switch
/// as the other two have theirs. Read once by [`report_crashes`].
static SKIP_SMALL_WRITE: AtomicBool = AtomicBool::new(true);

/// The stores [`skip_small_write`] skipped: `WRITE_SKIPS` at exit and in
/// every crash report, beside the other two.
static WRITE_SKIPS: AtomicU64 = AtomicU64::new(0);

/// Title images the loader mapped, that [`skip_null_chain`] may skip in: a
/// fixed table, so that the handler reads it without a lock (the faulting
/// thread may hold any of the title's, ours or the heap's). More images than
/// a title ships; one that does not fit is not skipped in, which is the safe
/// way to be wrong.
const TITLE_IMAGES: usize = 32;

/// Where one title image lies: `base` is 0 until an image takes the entry.
struct ImageSpan {
    base: AtomicUsize,
    /// Bytes from `base`, the image's `SizeOfImage`.
    len: AtomicUsize,
}

impl ImageSpan {
    const fn new() -> Self {
        Self {
            base: AtomicUsize::new(0),
            len: AtomicUsize::new(0),
        }
    }
}

static SPANS: [ImageSpan; TITLE_IMAGES] = [const { ImageSpan::new() }; TITLE_IMAGES];

/// Notes the title image the loader just mapped at `base`, `len` bytes long.
/// [`skip_null_chain`] skips nothing outside one: the host's code (the
/// runtime's, the C runtime's, the graphics driver's) reads a null pointer
/// for its own reasons, and hiding that would hide our bugs as the title's.
pub(crate) fn title_image(base: usize, len: usize) {
    if base == 0 || len == 0 {
        return;
    }
    for span in &SPANS {
        if span.base.load(Relaxed) == base {
            span.len.store(len, Relaxed);
            return;
        }
    }
    for span in &SPANS {
        if span.base.load(Acquire) != 0 {
            continue;
        }
        // The length first, then the base that publishes the entry.
        span.len.store(len, Relaxed);
        if span.base.compare_exchange(0, base, Release, Relaxed).is_ok() {
            return;
        }
    }
    tracing::warn!(
        base = format_args!("{base:#x}"),
        "no room left in the title image table: the null guards skip nothing in it"
    );
}

/// The image at `base` is unmapped: no code there is the title's any more.
pub(crate) fn title_image_unmapped(base: usize) {
    for span in &SPANS {
        if span.base.load(Acquire) == base {
            span.base.store(0, Release);
            span.len.store(0, Relaxed);
        }
    }
}

/// Whether `at` is inside a title image the loader mapped.
fn in_title_image(at: usize) -> bool {
    SPANS.iter().any(|span| {
        let base = span.base.load(Acquire);
        let len = span.len.load(Acquire);
        base != 0 && at.wrapping_sub(base) < len
    })
}

/// Below any object the title could have: a read here is a null pointer's.
const NULL_LIMIT: usize = 0x1_0000;

/// Fault addresses the loop breaker tracks: a fixed table, so that the
/// handler allocates nothing and waits for no lock (the faulting thread may
/// hold the title's, ours or the heap's).
const SKIP_SITES: usize = 64;
/// A site skipped more often than this within [`SKIP_WINDOW`] stops being
/// skipped: a guard that hides an infinite loop is worse than the crash it
/// hides.
const SKIP_LIMIT: u32 = 8;
/// Seconds of [`SKIP_WINDOW`]: a site's allowance comes back after this long.
const SKIP_WINDOW: u64 = 60;

/// One fault address the guard skipped at, and how often lately.
struct Site {
    rip: AtomicUsize,
    skips: AtomicU32,
    /// Unix time of the first skip in the current window.
    since: AtomicU64,
}

impl Site {
    const fn new() -> Self {
        Self {
            rip: AtomicUsize::new(0),
            skips: AtomicU32::new(0),
            since: AtomicU64::new(0),
        }
    }
}

static SITES_SEEN: [Site; SKIP_SITES] = [const { Site::new() }; SKIP_SITES];

/// The table entry for `rip`, taking a free one if the site is new; `None`
/// with every entry taken by other sites (the guard then gives up, rather
/// than skip a site it cannot count).
fn site_for(rip: usize) -> Option<&'static Site> {
    let first = (rip >> 4) % SKIP_SITES;
    for i in 0..SKIP_SITES {
        let site = &SITES_SEEN[(first + i) % SKIP_SITES];
        if site.rip.load(Relaxed) == rip {
            return Some(site);
        }
        if site.rip.load(Relaxed) == 0
            && site.rip.compare_exchange(0, rip, Relaxed, Relaxed).is_ok()
        {
            return Some(site);
        }
    }
    None
}

/// Counts one skip at `rip` and says whether it may happen: its number at
/// the site (to log), or `None` past [`SKIP_LIMIT`] within [`SKIP_WINDOW`],
/// which leaves the exception to be reported as usual.
fn count_skip(rip: usize) -> Option<u32> {
    let site = site_for(rip)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let n = site.skips.fetch_add(1, Relaxed) + 1;
    if n == 1 {
        site.since.store(now, Relaxed);
    }
    let since = match site.since.load(Relaxed) {
        0 => now,
        since => since,
    };
    if n <= SKIP_LIMIT {
        return Some(n);
    }
    if now.saturating_sub(since) < SKIP_WINDOW {
        return None;
    }
    // The site behaved for a window: its allowance starts again.
    site.since.store(now, Relaxed);
    site.skips.store(1, Relaxed);
    Some(1)
}

/// A vtable call: `mov reg, [reg2]` and then `call|jmp *disp(reg)`.
struct VtableCall {
    /// Bytes from the call to the instruction after it.
    len: usize,
    /// The register holding the vtable pointer, as instructions encode it.
    vtable: u8,
    /// The slot read through it, which is the address faulted on when the
    /// vtable pointer is null.
    slot: u64,
}

/// An indirect call through a slot of an object: `call|jmp *disp(reg)` and
/// the slot read through it.
struct IndirectCall {
    /// Bytes from the call to the instruction after it.
    len: usize,
    /// The register holding the vtable pointer, as instructions encode it.
    vtable: u8,
    /// The slot read through it, which is the address faulted on when the
    /// vtable pointer is null.
    slot: u64,
}

/// The indirect call at `at` in `code`, if it has exactly the shape of a
/// call through a slot of an object: a `call` or a `jmp` (a tail call's)
/// through memory, a SIB with a base and no scaled index, not RIP-relative,
/// and a slot that is one (`0` to `0x78`, a multiple of 8). `reg`, the base
/// and the slot are free: a title's crash moves from site to site, not its
/// idiom.
///
/// Whether a `mov` filled that register is [`vtable_load`]'s question: the
/// two are separate so that a call of the shape whose load does not check
/// out is a fault the guard refuses to skip, and says so.
fn indirect_call(code: &[u8], at: usize) -> Option<IndirectCall> {
    let (rex, mut i) = match *code.get(at)? {
        rex @ 0x40..=0x4f => (rex, at + 1),
        _ => (0, at),
    };
    if *code.get(i)? != 0xff {
        return None;
    }
    let modrm = *code.get(i + 1)?;
    // /2 is `call`, /4 `jmp`, both through memory.
    if !matches!((modrm >> 3) & 7, 2 | 4) {
        return None;
    }
    let (mode, rm) = (modrm >> 6, modrm & 7);
    if mode == 3 {
        return None;
    }
    i += 2;
    let mut has_sib = false;
    let base = if rm == 4 {
        let sib = *code.get(i)?;
        // A base register and no index: a scaled index is no vtable slot.
        if (sib >> 3) & 7 != 4 || rex & 2 != 0 {
            return None;
        }
        has_sib = true;
        i += 1;
        sib & 7
    } else if rm == 5 && mode == 0 {
        // RIP-relative: no object's vtable.
        return None;
    } else {
        rm
    };
    // A SIB with no base register at all (an absolute address).
    if has_sib && mode == 0 && base == 5 && rex & 1 == 0 {
        return None;
    }
    let (disp, size) = match mode {
        0 => (0i64, 0usize),
        1 => (i64::from(*code.get(i)? as i8), 1),
        _ => (
            i64::from(i32::from_le_bytes(code.get(i..i + 4)?.try_into().ok()?)),
            4,
        ),
    };
    let len = i + size - at;
    if !(0..=0x78).contains(&disp) || disp % 8 != 0 {
        return None;
    }
    let vtable = base | (rex & 1) << 3;
    Some(IndirectCall {
        len,
        vtable,
        slot: disp as u64,
    })
}

/// The virtual call at `at` in `code`, if [`indirect_call`]'s shape is there
/// and [`vtable_load`] proves the register it goes through was filled by a
/// `mov reg, [base]`.
fn null_vtable_call(code: &[u8], at: usize) -> Option<VtableCall> {
    let call = indirect_call(code, at)?;
    vtable_load(code, at, call.vtable).then_some(VtableCall {
        len: call.len,
        vtable: call.vtable,
        slot: call.slot,
    })
}

/// How far before the call the load of its register may be: the title's
/// code puts a few `mov reg, reg` between them, and no more.
const VTABLE_REACH: usize = 16;

/// Whether a `mov reg, [reg2]` into `vtable` ends exactly at `end` — the
/// call — or ends at most [`VTABLE_REACH`] bytes before it and only argument
/// setup came between: `mov reg, reg` instructions that write neither
/// `vtable` nor any memory.
///
/// The run48 crash is why: the compiler put three of them between the load
/// and the call (`mov r15,[rdi]; mov rcx,rdi; mov edx,ebx; mov r14,rax;
/// call [r15+8]`), and a load that ends exactly at the call — all this asked
/// for before — is not the shape the title's code has.
///
/// Fail-closed: the nearest load into that register is taken only if every
/// byte between it and the call is such a move. An instruction that writes
/// the register, reads or writes memory, or that is not a register move ends
/// the walk, and the answer is no: the fault is then reported as before,
/// because a skip on a valid call corrupts the title silently.
fn vtable_load(code: &[u8], end: usize, vtable: u8) -> bool {
    let first = end.saturating_sub(VTABLE_REACH);
    (first..=end.saturating_sub(3))
        .rev()
        .any(|at| load_into(code, at) == Some(vtable) && register_setup(code, at + 3, end, vtable))
}

/// Whether `code[from..to]` is argument setup only: instructions that move
/// one register into another, none of them writing `keep`, and none of them
/// reading or writing memory. The bytes must tile exactly: a range that
/// decodes to anything else, or that a move overruns, is no setup.
fn register_setup(code: &[u8], from: usize, to: usize, keep: u8) -> bool {
    let mut at = from;
    while at < to {
        let Some((len, dst)) = move_register(code, at) else {
            return false;
        };
        // (The move must end inside the range, not past it.)
        if dst == keep || at + len > to {
            return false;
        }
        at += len;
    }
    at == to
}

/// `mov reg, reg` at `at`: an optional REX prefix, `88`/`89` (the modrm's
/// `rm` is written) or `8a`/`8b` (`reg` is), `modrm` mod 11 — a register as
/// the source and as the destination. Its length and the register written.
/// Anything with a memory operand is a load or a store and is no argument
/// setup; so is anything else, and both end the walk back.
fn move_register(code: &[u8], at: usize) -> Option<(usize, u8)> {
    let rex = match *code.get(at)? {
        rex @ 0x40..=0x4f => rex,
        _ => 0,
    };
    let i = at + usize::from(rex != 0);
    let op = *code.get(i)?;
    let modrm = *code.get(i + 1)?;
    if !matches!(op, 0x88..=0x8b) || modrm >> 6 != 3 {
        return None;
    }
    // 88/89 write `rm`, 8a/8b write `reg`. An 8- or 16-bit move writes its
    // register too: which part of it does not matter to the walk.
    let dst = if op & 2 == 0 {
        modrm & 7 | (rex & 1) << 3
    } else {
        (modrm >> 3) & 7 | (rex & 4) << 1
    };
    Some((i + 2 - at, dst))
}

/// The register `mov reg, [reg2]` at `at` loads, if that is one instruction
/// of three bytes: `REX.W(+R/B) 8B /r`, mod 00, no SIB, no displacement.
fn load_into(code: &[u8], at: usize) -> Option<u8> {
    let &[rex @ 0x48..=0x4f, 0x8b, modrm] = code.get(at..at + 3)? else {
        return None;
    };
    if modrm >> 6 != 0 || matches!(modrm & 7, 4 | 5) {
        return None;
    }
    Some((modrm >> 3) & 7 | (rex & 4) << 1)
}

/// The guard: for an access violation that is a virtual call through a null
/// vtable pointer -- `*(rdi)` is 0 at `mov (%rdi),%r15; call *0x8(%r15)`,
/// which faults reading `0x8` -- skip the call: `rax = 0` (a call returns in
/// it, and the title's use of the result is a pointer it has to check
/// anyway), `rip` past the call, the flags as they were (a call leaves
/// them). Nothing else is emulated, because nothing else is known.
///
/// Only a fault that is a read below [`NULL_LIMIT`], of exactly the slot the
/// call reads, through a register that is exactly 0, at an instruction of
/// exactly the shape [`indirect_call`] decodes and with a load of that
/// register proven by [`vtable_load`] (the load, or argument setup and then
/// the load), is skipped. Anything else returns `false` and is reported as
/// before: a guard that guesses is worse than a crash that says where it
/// was. A call whose shape is right and whose register is null but whose
/// load the walk back cannot prove is refused, and the refusal is logged: it
/// says the compiler put something between the load and the call that this
/// does not know, which is the one thing this guard should be widened for
/// next.
fn skip_null_vtable(context: &mut CONTEXT, record: &EXCEPTION_RECORD) -> bool {
    let p = record.ExceptionInformation;
    if record.ExceptionCode != EXCEPTION_ACCESS_VIOLATION
        || record.NumberParameters < 2
        || p[0] != 0
        || p[1] >= NULL_LIMIT
    {
        return false;
    }
    let rip = context.Rip as usize;
    // The call's 15 bytes at most, and the walk back's 16 before it; where
    // the page below is not there to read, the load can only be the one that
    // ends at the call, as it was before.
    let mut code = [0u8; VTABLE_REACH + 16];
    let at = match rip.checked_sub(VTABLE_REACH) {
        // SAFETY: reads the title's code below `rip` through the system,
        // which faults nothing of ours; the buffer is ours.
        Some(base) if unsafe { read_into(base, &mut code[..VTABLE_REACH + 15]) } => VTABLE_REACH,
        _ => {
            let Some(base) = rip.checked_sub(15) else {
                return false;
            };
            // SAFETY: as above.
            if !unsafe { read_into(base, &mut code[..VTABLE_REACH + 16]) } {
                return false;
            }
            15
        }
    };
    let Some(call) = null_vtable_call(&code, at) else {
        // The shape, then, without a load it would skip on: if that is what
        // faulted, it is this guard's fault it is not skipped.
        if let Some(shape) = indirect_call(&code, at)
            && *gpr(context, shape.vtable) == 0
            && p[1] == shape.slot as usize
        {
            tracing::warn!(
                site = %locate(rip),
                slot = format_args!("{:#x}", shape.slot),
                "null vtable call not skipped: no load of the call's register between the walk back's reach and the call"
            );
        }
        return false;
    };
    // The state as well as the bytes: the register the call read holds the
    // vtable pointer, it is null, and the access was to that slot itself.
    if *gpr(context, call.vtable) != 0 || p[1] != call.slot as usize {
        return false;
    }
    let Some(n) = count_skip(rip) else {
        tracing::warn!(
            site = %locate(rip),
            "null vtable call not skipped: too many at this site"
        );
        return false;
    };
    context.Rax = 0;
    context.Rip = context.Rip.wrapping_add(call.len as u64);
    let total = NULL_VTABLE_SKIPS.fetch_add(1, Relaxed) + 1;
    if n == 1 || n.is_multiple_of(4) {
        tracing::warn!(
            site = %locate(rip),
            slot = format_args!("{:#x}", call.slot),
            skip = n,
            total,
            "null vtable call skipped"
        );
    }
    true
}

/// A load of a field through a base register: `mov reg, [base + disp]`.
struct ChainLoad {
    /// Bytes from the load to the instruction after it.
    len: usize,
    /// The register loaded, as instructions encode it.
    dst: u8,
    /// The register read through, the one that is null at the title's sites.
    base: u8,
    /// The offset read, which is the address that faults through a null base.
    disp: u64,
    /// The bytes the load reads: 8 for `mov`, 2 for `movzx`/`movsx`.
    width: u8,
}

/// The load at `at` in `code`, if it is exactly the load of a chain, with
/// REX.B and REX.X clear (so the base is one of the first eight registers
/// and no index is in play), `modrm` mod 00, 01 or 10, `rm` neither 100 (a
/// SIB) nor, at mod 00, 101 (RIP-relative: no base register at all), and a
/// displacement from 0 up to [`NULL_LIMIT`]. The chain the title builds
/// offsets forward from the object it should have had; a negative
/// displacement is not that shape.
///
/// Three forms of it, which are the same fault when the base is null:
///
/// * `REX.W 8B /r` — `mov reg, [base + disp]`, eight bytes;
/// * `REX? 0F B7 /r` — `movzx reg, WORD [base + disp]`, two bytes;
/// * `REX? 0F BF /r` — `movsx reg, WORD [base + disp]`, two bytes.
///
/// The run44 crash is why the last two are here: its load was
/// `44 0f b7 40 18` (`movzx r8d, WORD [rax+0x18]`) with `rax` null, and a
/// guard that knew only `8B` reported it as a fault to end the process on.
/// REX.W is not needed for them: `movzx r8d, …` is a 32-bit destination and
/// still a chain, and the skip writes 0 into the whole register, which is
/// the value at any width (a 32-bit destination zero-extends it, and a sign
/// extension of 0 is 0, so the upper half the title would see is 0 too).
fn chain_load(code: &[u8], at: usize) -> Option<ChainLoad> {
    let rex = *code.get(at)?;
    // A REX prefix, with X and B clear: `44` (REX.R, for a register above
    // r7), `48` (REX.W), `4c` (both).
    if !matches!(rex, 0x40..=0x4f) || rex & 0x03 != 0 {
        return None;
    }
    let (width, i) = match (*code.get(at + 1)?, *code.get(at + 2)?) {
        (0x8b, _) if rex & 0x08 != 0 => (8u8, at + 2),
        // `movzx` and `movsx` of a word: the same shape with two bytes read.
        (0x0f, 0xb7 | 0xbf) => (2u8, at + 3),
        _ => return None,
    };
    let modrm = *code.get(i)?;
    let (mode, rm) = (modrm >> 6, modrm & 7);
    if mode == 3 || rm == 4 || (mode == 0 && rm == 5) {
        return None;
    }
    let i = i + 1;
    let (disp, size) = match mode {
        0 => (0i64, 0usize),
        1 => (i64::from(*code.get(i)? as i8), 1),
        _ => (
            i64::from(i32::from_le_bytes(code.get(i..i + 4)?.try_into().ok()?)),
            4,
        ),
    };
    if !(0..NULL_LIMIT as i64).contains(&disp) {
        return None;
    }
    Some(ChainLoad {
        len: i + size - at,
        dst: (modrm >> 3) & 7 | (rex & 4) << 1,
        base: rm | (rex & 1) << 3,
        disp: disp as u64,
        width,
    })
}

/// The guard for the title's other null race: a chain of loads in which a
/// middle pointer is null. In the run30 crash the title's own
/// `movq 0x1a8(%rcx),%rax; movq 0x18(%rax),%rdx; movq 0x20(%rax),%rcx;
/// movq 0x18(%rdx),%r8` faulted reading `0x18` with `rdx` exactly 0: a field
/// consumed before the object that should hold it exists (a null object's
/// field was loaded, and its own field is null too). The load is skipped
/// with its destination register set to 0, so the chain goes on as a null
/// pointer, which is the value the title's null tests after it expect
/// (`testq %r14,%r14; jne`) and the value the race would have left had it
/// gone the other way. Nothing else is emulated, because nothing else is
/// known.
///
/// Fail-closed, all of: a read (not a write, not an execute); an address
/// below [`NULL_LIMIT`]; an instruction of exactly the shape [`chain_load`]
/// decodes; a base register that is exactly 0; a fault address that is
/// exactly the load's displacement; and `rip` inside a title image the
/// loader mapped. Anything else returns `false` and is reported as before:
/// a guard that guesses is worse than a crash that says where it was.
///
/// The site table and its loop breaker are the vtable guard's, keyed by
/// `rip`: a faulting site is a faulting site, whatever shape it has, and one
/// that repeats forever must stop being skipped just the same.
fn skip_null_chain(context: &mut CONTEXT, record: &EXCEPTION_RECORD) -> bool {
    let p = record.ExceptionInformation;
    if record.ExceptionCode != EXCEPTION_ACCESS_VIOLATION
        || record.NumberParameters < 2
        || p[0] != 0
        || p[1] >= NULL_LIMIT
    {
        return false;
    }
    let rip = context.Rip as usize;
    if !in_title_image(rip) {
        return false;
    }
    // The longest such load is 8 bytes (`REX 0F B7 /r` and a disp32), and
    // none of them past the faulting instruction's page: the next page may
    // not be mapped.
    let room = (0x1000 - (rip & 0xfff)).min(8);
    let mut code = [0u8; 8];
    // SAFETY: reads the title's code at `rip` through the system, which
    // faults nothing of ours; the buffer is ours.
    if !unsafe { read_into(rip, &mut code[..room]) } {
        return false;
    }
    let Some(load) = chain_load(&code[..room], 0) else {
        return false;
    };
    // The state as well as the bytes: the base is null, and the access was
    // to the very offset the load reads.
    if *gpr(context, load.base) != 0 || p[1] != load.disp as usize {
        return false;
    }
    let Some(n) = count_skip(rip) else {
        tracing::warn!(
            site = %locate(rip),
            "null chain load not skipped: too many at this site"
        );
        return false;
    };
    *gpr(context, load.dst) = 0;
    context.Rip = context.Rip.wrapping_add(load.len as u64);
    let total = NULL_CHAIN_SKIPS.fetch_add(1, Relaxed) + 1;
    if n == 1 || n.is_multiple_of(4) {
        tracing::warn!(
            site = %locate(rip),
            disp = format_args!("{:#x}", load.disp),
            dst = load.dst,
            bytes = load.width,
            skip = n,
            total,
            "null chain load skipped"
        );
    }
    true
}

/// A store through a base register: `mov [base + disp], reg` (or an
/// immediate).
struct SmallStore {
    /// Bytes from the store to the instruction after it.
    len: usize,
    /// The register the store is through, as instructions encode it.
    base: u8,
    /// The offset the store adds to it, sign-extended as the encoding has it.
    disp: u64,
}

/// The store at `at` in `code`, if it is exactly a store of one register (or
/// one immediate) through one base register: an optional operand-size prefix
/// and REX prefix, then `88`/`89` (a register is stored) or `C6`/`C7` with
/// `modrm`'s `reg` field 0 (an immediate is), `modrm` mod 00, 01 or 10, `rm`
/// neither 100 (a SIB: a scaled index is no base register) nor, at mod 00,
/// 101 (RIP-relative: no base at all). The immediate and the displacement
/// are part of the length, so that the whole store is skipped and not half
/// of it.
fn small_store(code: &[u8], at: usize) -> Option<SmallStore> {
    let mut i = at;
    let sized = match *code.get(i)? {
        0x66 => {
            i += 1;
            true
        }
        _ => false,
    };
    let rex = match *code.get(i)? {
        rex @ 0x40..=0x4f => {
            i += 1;
            rex
        }
        _ => 0,
    };
    let op = *code.get(i)?;
    let modrm = *code.get(i + 1)?;
    let imm = match op {
        0x88 | 0x89 => 0usize,
        // (`reg` field 0: the only immediate forms of the store; `/7` is
        // `xbegin` and the rest are not stores.)
        0xc6 if (modrm >> 3) & 7 == 0 => 1,
        0xc7 if (modrm >> 3) & 7 == 0 => {
            if sized {
                2
            } else {
                4
            }
        }
        _ => return None,
    };
    let (mode, rm) = (modrm >> 6, modrm & 7);
    if mode == 3 || rm == 4 || (mode == 0 && rm == 5) {
        return None;
    }
    i += 2;
    let (disp, size) = match mode {
        0 => (0i64, 0usize),
        1 => (i64::from(*code.get(i)? as i8), 1),
        _ => (
            i64::from(i32::from_le_bytes(code.get(i..i + 4)?.try_into().ok()?)),
            4,
        ),
    };
    i += size;
    // The immediate's bytes are part of the store: an instruction cut off
    // before its end is not one to skip.
    code.get(i..i + imm)?;
    i += imm;
    Some(SmallStore {
        len: i - at,
        base: rm | (rex & 1) << 3,
        disp: disp as u64,
    })
}

/// The guard for the title's third crash shape: a *write* through a small
/// non-null pointer. In run46 the record builder's destination came from a
/// block's header word (`0xf0`, a size, where a pointer belongs) and its
/// first store, `mov [rdx+8],al` with `rdx = 0xf0`, faulted writing `0xf8`.
///
/// Skipped: the store, whole — the record is one the title's own next
/// generation overwrites, and a byte more or less in a record whose pointer
/// slot held a size is a record the title drops anyway (the port-descriptor
/// fallback and `descriptor.rs` already refuse a record whose words are no
/// view).
///
/// Fail-closed, all of: a write (not a read, not an execute); an address
/// below [`NULL_LIMIT`]; `rip` inside a title image the loader mapped; an
/// instruction of exactly the shape [`small_store`] decodes; a base register
/// that is exactly the address the fault is at, less the store's
/// displacement; and that address below [`NULL_LIMIT`] — a store through a
/// null or a header-word pointer, not one through memory of the title's own.
/// Anything else returns `false` and is reported as before.
///
/// The site table and its loop breaker are the other guards', keyed by
/// `rip`: one site skipping forever must stop being skipped just the same.
fn skip_small_write(context: &mut CONTEXT, record: &EXCEPTION_RECORD) -> bool {
    let p = record.ExceptionInformation;
    if record.ExceptionCode != EXCEPTION_ACCESS_VIOLATION
        || record.NumberParameters < 2
        || p[0] != 1
        || p[1] >= NULL_LIMIT
    {
        return false;
    }
    let rip = context.Rip as usize;
    if !in_title_image(rip) {
        return false;
    }
    // The longest such store is 15 bytes (`66 REX C7 /0` and a disp32 and an
    // imm32 is 12), and none of them past the faulting instruction's page.
    let room = (0x1000 - (rip & 0xfff)).min(15);
    let mut code = [0u8; 15];
    // SAFETY: reads the title's code at `rip` through the system, which
    // faults nothing of ours; the buffer is ours.
    if !unsafe { read_into(rip, &mut code[..room]) } {
        return false;
    }
    let Some(store) = small_store(&code[..room], 0) else {
        return false;
    };
    // The state as well as the bytes: the base the store is through is small
    // — 0, a size, a count — and the access was to exactly the address the
    // store makes of it.
    let base = *gpr(context, store.base);
    if base >= NULL_LIMIT as u64 || base.wrapping_add(store.disp) != p[1] as u64 {
        return false;
    }
    let Some(n) = count_skip(rip) else {
        tracing::warn!(
            site = %locate(rip),
            "write through a small pointer not skipped: too many at this site"
        );
        return false;
    };
    context.Rip = context.Rip.wrapping_add(store.len as u64);
    let total = WRITE_SKIPS.fetch_add(1, Relaxed) + 1;
    if n == 1 || n.is_multiple_of(4) {
        tracing::warn!(
            site = %locate(rip),
            base = format_args!("{base:#x}"),
            disp = format_args!("{:#x}", store.disp),
            at = format_args!("{:#x}", p[1]),
            len = store.len,
            skip = n,
            total,
            "write through a small pointer skipped"
        );
    }
    true
}

/// The survival guards, in the order the handler runs them; `vtable`,
/// `chain` and `write` are their switches as [`report_crashes`] read them.
/// Their own switches, not each other's: a run that turns one off keeps the
/// others.
fn skip_null_faults(
    context: &mut CONTEXT,
    record: &EXCEPTION_RECORD,
    vtable: bool,
    chain: bool,
    write: bool,
) -> bool {
    if vtable && skip_null_vtable(context, record) {
        return true;
    }
    if chain && skip_null_chain(context, record) {
        return true;
    }
    if write && skip_small_write(context, record) {
        return true;
    }
    false
}

/// `n` bytes at `at` if all of them are readable, without allocating: the
/// handler may run on a thread that holds the heap's lock.
///
/// # Safety
/// Writes only `out`, through the system's own copy.
unsafe fn read_into(at: usize, out: &mut [u8]) -> bool {
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut read = 0usize;
    // SAFETY: the system copies at most `out.len()` bytes into `out` and
    // says how many; nothing is read through a pointer of ours.
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            at as *const _,
            out.as_mut_ptr().cast(),
            out.len(),
            &mut read,
        )
    };
    ok != 0 && read == out.len()
}

unsafe extern "C" {
    fn atexit(f: extern "C" fn()) -> i32;
}

/// Process exit: how many calls and loads the guards skipped, so that a run
/// which limped through them is not read as one that did not. One line with
/// the three counters by name — the measure of how often the title's race
/// fires — and one line for each kind that did skip.
extern "C" fn skips_at_exit() {
    let sites = SITES_SEEN
        .iter()
        .filter(|site| site.rip.load(Relaxed) != 0)
        .count();
    let vtable = NULL_VTABLE_SKIPS.load(Relaxed);
    let chain = NULL_CHAIN_SKIPS.load(Relaxed);
    let writes = WRITE_SKIPS.load(Relaxed);
    tracing::info!(
        null_vtable = vtable,
        null_chain = chain,
        small_write = writes,
        sites,
        "NULL_VTABLE_SKIPS {vtable} NULL_CHAIN_SKIPS {chain} WRITE_SKIPS {writes} SITES {sites} (at exit)"
    );
    if vtable != 0 {
        tracing::info!(skips = vtable, sites, "null vtable calls skipped (at exit)");
    }
    if chain != 0 {
        tracing::info!(skips = chain, sites, "null chain loads skipped (at exit)");
    }
    if writes != 0 {
        tracing::info!(skips = writes, sites, "writes through a small pointer skipped (at exit)");
    }
}

unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    // an instruction of the console's CPU this one lacks: emulated, the title goes on
    // SAFETY: the system passes valid exception pointers.
    if unsafe { (*(*info).ExceptionRecord).ExceptionCode } == EXCEPTION_ILLEGAL_INSTRUCTION {
        // SAFETY: the faulting thread's context, for the exception being dispatched.
        let r = unsafe { crate::console_insn::emulate(&mut *(*info).ContextRecord) };
        if r != 0 {
            return r;
        }
    }
    // SAFETY: the system passes valid exception pointers.
    if unsafe { emulate_amd_only(info) } {
        return -1; // EXCEPTION_CONTINUE_EXECUTION
    }
    // `VICTORY_SKIP_INT3` / `LEONIDA_SKIP_INT3` (diagnostic aid): the
    // title's soft asserts are stepped over instead of reaching its handler.
    // SAFETY: as above.
    if unsafe { (*(*info).ExceptionRecord).ExceptionCode } == EXCEPTION_BREAKPOINT {
        static SKIP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let skip = *SKIP.get_or_init(|| {
            std::env::var_os("VICTORY_SKIP_INT3").is_some() || std::env::var_os("LEONIDA_SKIP_INT3").is_some()
        });
        if skip {
            // SAFETY: as above.
            let context = unsafe { &mut *(*info).ContextRecord };
            let rip = context.Rip as usize;
            // SAFETY: the breakpoint just executed at `rip`, so its bytes are
            // mapped (`int3` is one byte, `int 3` two).
            let len = unsafe {
                if std::ptr::read_volatile(rip as *const u8) == 0xCD
                    && std::ptr::read_volatile((rip + 1) as *const u8) == 0x03
                {
                    2
                } else {
                    1
                }
            };
            tracing::warn!(rip = format_args!("{rip:#x}"), len, "breakpoint skipped");
            context.Rip = context.Rip.wrapping_add(len);
            return -1; // EXCEPTION_CONTINUE_EXECUTION
        }
    }
    // `VICTORY_SKIP_NULL_VTABLE` / `VICTORY_SKIP_NULL_CHAIN` /
    // `VICTORY_SKIP_SMALL_WRITE` (each on unless `0`): a virtual call through
    // a null vtable pointer, a load through a null pointer in a chain of
    // them, or a store through a small non-null pointer (a size where a
    // pointer belongs) is the title's own lifecycle race, and is skipped
    // rather than ending the process (see [`skip_null_faults`]).
    let skipped = {
        // SAFETY: the system passes the exception being dispatched, and
        // `skip_null_faults` reports the exception it could not skip.
        unsafe {
            skip_null_faults(
                &mut *(*info).ContextRecord,
                &*(*info).ExceptionRecord,
                SKIP_NULL_VTABLE.load(Relaxed),
                SKIP_NULL_CHAIN.load(Relaxed),
                SKIP_SMALL_WRITE.load(Relaxed),
            )
        }
    };
    if skipped {
        return -1; // EXCEPTION_CONTINUE_EXECUTION
    }
    // SAFETY: as above.
    if FATAL.contains(&unsafe { (*(*info).ExceptionRecord).ExceptionCode }) {
        // SAFETY: as above.
        unsafe { report("first-chance exception", info, false) };
    }
    EXCEPTION_CONTINUE_SEARCH
}

/// For an unhandled-exception filter: reports the exception that is about
/// to end the process, and appends it to the crash log.
///
/// # Safety
/// `info` comes from the system, for an exception being dispatched.
pub unsafe fn report_unhandled(info: *const EXCEPTION_POINTERS) {
    // SAFETY: forwarded from the caller.
    unsafe { report("unhandled exception", info, true) };
}

/// The bytes at `at` as hex, as many as the longest x86 instruction (15),
/// but none past the end of `at`'s page: the next page may not be mapped.
/// For an instruction the CPU refused, they name it (`0f 01 fd` is
/// `rdpru`), which the address alone does not.
///
/// # Safety
/// `at`'s page is readable.
unsafe fn instruction_bytes(at: usize) -> String {
    let n = (0x1000 - (at & 0xfff)).min(15);
    // SAFETY: `n` bytes from `at` stay inside its page, which the caller
    // says is readable.
    let bytes = unsafe { std::slice::from_raw_parts(at as *const u8, n) };
    let hex: Vec<_> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    hex.join(" ")
}

/// MSVC's code for a C++ `throw`, and its magic first parameter.
const CPP_EXCEPTION: i32 = 0xE06D_7363_u32 as i32;
const CPP_MAGIC: usize = 0x1993_0520;

/// What a C++ throw threw: the decorated name of its type, then its value
/// where that names the cause (an integer, e.g. an HRESULT; a standard
/// exception's message).
///
/// # Safety
/// `p` are the parameters of a live MSVC throw (`p[0]` the magic).
unsafe fn thrown(p: &[usize]) -> String {
    // SAFETY: the throw's ThrowInfo, its RVAs relative to `p[3]`: catchable
    // type array at +12 (count, then type RVAs), each type's descriptor at
    // +4, whose decorated name starts at +16; the object is at `p[1]`.
    unsafe {
        let rva = |at: usize| (at as *const u32).read_unaligned() as usize;
        let types = p[3] + rva(p[2] + 12);
        let name = |i: usize| {
            let descriptor = p[3] + rva(p[3] + rva(types + 4 + 4 * i) + 4);
            std::ffi::CStr::from_ptr((descriptor + 16) as *const std::ffi::c_char)
                .to_string_lossy()
        };
        let first = name(0);
        let value = match &*first {
            // int, unsigned, long (HRESULT), unsigned long
            ".H" | ".I" | ".J" | ".K" if p[1] != 0 => {
                format!(" = {:#010x}", (p[1] as *const u32).read_unaligned())
            }
            // std::exception keeps its message after the table pointer.
            _ if (0..rva(types)).any(|i| name(i) == ".?AVexception@std@@") => {
                let message = (p[1] as *const *const std::ffi::c_char).add(1).read();
                if message.is_null() {
                    String::new()
                } else {
                    format!(": {}", std::ffi::CStr::from_ptr(message).to_string_lossy())
                }
            }
            _ => String::new(),
        };
        format!("{first}{value}")
    }
}

/// What another part of the layer has to say of an access violation, given
/// the sixteen registers and the address accessed ([`describe_on_crash`]).
static DESCRIBE: std::sync::OnceLock<fn(&[u64]) -> String> = std::sync::OnceLock::new();

/// `f` is asked at every access violation reported, with the registers
/// (rax, rbx, rcx, rdx, rsi, rdi, rsp, rbp, r8 to r15) and the address
/// accessed, for lines to add to the report (each ending in a newline). It
/// runs on the faulting thread, which may hold any lock of the title's: it
/// must not wait for one.
pub fn describe_on_crash(f: fn(&[u64]) -> String) {
    let _ = DESCRIBE.set(f);
}

/// `len` bytes at `at` as far as they can be read, without a fault if they
/// cannot.
fn read_quietly(at: usize, len: usize) -> Vec<u8> {
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut bytes = vec![0u8; len];
    let mut read = 0usize;
    // SAFETY: the system copies what is readable into our buffer and says
    // how much; nothing is read through a pointer of ours.
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), at as *const _, bytes.as_mut_ptr().cast(), len, &mut read) };
    bytes.truncate(if ok != 0 { len } else { read.min(len) });
    bytes
}

/// For an access violation: the code around the instruction, and what each
/// register points at, if it points at memory that is no image: its kind,
/// its first bytes, and how many bytes of its 4 KiB page are not zero.
///
/// The title's crashes under Proton were reads through a null pointer in its
/// own code, at another place each time (six by 2026-11-25): a pointer the
/// title took from an object of its own was null. Whether that object lay in
/// a page that was all zeros, and in a view of its physical pages or in
/// memory of its heap, is what the registers and the stack alone do not say.
fn memory_lines(at: usize, registers: &[(&str, u64)], accessed: u64) -> Vec<String> {
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_IMAGE, MEM_MAPPED, MEMORY_BASIC_INFORMATION, VirtualQuery};
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
    let mut lines = vec![format!("  code before: {} | at: {}", hex(&read_quietly(at.saturating_sub(24), 24)), hex(&read_quietly(at, 16)))];
    for &(name, value) in registers {
        if value < 0x1_0000 {
            continue;
        }
        // SAFETY: a zeroed MEMORY_BASIC_INFORMATION is valid; the query reads
        // the address space's bookkeeping, not the address.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: as above.
        if unsafe { VirtualQuery(value as *const _, &mut info, size_of_val(&info)) } == 0
            || info.State != MEM_COMMIT
            || info.Type == MEM_IMAGE
        {
            continue;
        }
        let page = read_quietly(value as usize & !0xfff, 0x1000);
        let first = read_quietly(value as usize, 32);
        lines.push(format!(
            "  {name} -> {} memory, allocation {:#x}, region {:#x}+{:#x}; its 4 KiB page: {} of {} bytes not zero; from it: {}",
            if info.Type == MEM_MAPPED { "mapped (a view)" } else { "private" },
            info.AllocationBase as usize,
            info.BaseAddress as usize,
            info.RegionSize,
            page.iter().filter(|&&b| b != 0).count(),
            page.len(),
            hex(&first),
        ));
    }
    if let Some(describe) = DESCRIBE.get() {
        let mut values: Vec<u64> = registers.iter().map(|r| r.1).collect();
        values.push(accessed);
        lines.extend(describe(&values).lines().map(str::to_owned));
    }
    lines
}

/// # Safety
/// See [`report_unhandled`].
unsafe fn report(what: &str, info: *const EXCEPTION_POINTERS, to_file: bool) {
    // SAFETY: the system passes valid exception pointers.
    let (record, context) = unsafe { (&*(*info).ExceptionRecord, &*(*info).ContextRecord) };
    let code = record.ExceptionCode;
    let at = record.ExceptionAddress as usize;
    let p = record.ExceptionInformation;
    let detail = if code == EXCEPTION_ACCESS_VIOLATION {
        let kind = match p[0] {
            0 => "read",
            1 => "write",
            _ => "execute",
        };
        format!(" ({kind} {:#x}: {})", p[1], memory_at(p[1]))
    } else if code == CPP_EXCEPTION && record.NumberParameters >= 4 && p[0] == CPP_MAGIC {
        // SAFETY: a live MSVC throw.
        format!(" (C++ {} thrown)", unsafe { thrown(&p) })
    } else if code == EXCEPTION_ILLEGAL_INSTRUCTION || code == EXCEPTION_PRIV_INSTRUCTION {
        // SAFETY: the CPU refused the instruction at `at`, so it fetched it.
        format!(" (code {})", unsafe { instruction_bytes(at) })
    } else {
        String::new()
    };
    // Printed line by line (a fault while unwinding must not lose the
    // lines before it), and kept for the file.
    let mut text = String::new();
    let mut line = |l: String| {
        eprintln!("{l}");
        text.push_str(&l);
        text.push('\n');
    };
    line(format!(
        "VIctory: {what} {:#010x} at {}{detail}",
        code as u32,
        locate(at)
    ));
    line(format!(
        "  rax {:#x} rbx {:#x} rcx {:#x} rdx {:#x} rsi {:#x} rdi {:#x} rsp {:#x} rbp {:#x}",
        context.Rax,
        context.Rbx,
        context.Rcx,
        context.Rdx,
        context.Rsi,
        context.Rdi,
        context.Rsp,
        context.Rbp
    ));
    line(format!(
        "  r8 {:#x} r9 {:#x} r10 {:#x} r11 {:#x} r12 {:#x} r13 {:#x} r14 {:#x} r15 {:#x}",
        context.R8,
        context.R9,
        context.R10,
        context.R11,
        context.R12,
        context.R13,
        context.R14,
        context.R15
    ));
    for (i, pc) in unwind(*context).into_iter().enumerate() {
        line(format!("  #{i:<2} {}", locate(pc as usize)));
    }
    for (skips, what) in [
        (
            NULL_VTABLE_SKIPS.load(Relaxed),
            "null vtable calls skipped before this",
        ),
        (
            NULL_CHAIN_SKIPS.load(Relaxed),
            "null chain loads skipped before this",
        ),
        (
            WRITE_SKIPS.load(Relaxed),
            "writes through a small pointer skipped before this",
        ),
    ] {
        if skips != 0 {
            line(format!("  {what}: {skips}"));
        }
    }
    // Once for a crash (the first-chance report; the unhandled one repeats
    // the registers only).
    if code == EXCEPTION_ACCESS_VIOLATION && !to_file {
        let registers = [
            ("rax", context.Rax),
            ("rbx", context.Rbx),
            ("rcx", context.Rcx),
            ("rdx", context.Rdx),
            ("rsi", context.Rsi),
            ("rdi", context.Rdi),
            ("rsp", context.Rsp),
            ("rbp", context.Rbp),
            ("r8", context.R8),
            ("r9", context.R9),
            ("r10", context.R10),
            ("r11", context.R11),
            ("r12", context.R12),
            ("r13", context.R13),
            ("r14", context.R14),
            ("r15", context.R15),
        ];
        for l in memory_lines(at, &registers, p[1] as u64) {
            line(l);
        }
    }
    if to_file {
        let said = title_output();
        eprint!("{said}");
        text.push_str(&said);
        append_to_log(&text);
    }
    #[cfg(feature = "diagnostics")]
    for (address, len, file) in SAVE.get().into_iter().flatten() {
        // SAFETY: the caller of `save_on_crash` named readable memory.
        let bytes = unsafe { std::slice::from_raw_parts(*address as *const u8, *len) };
        let saved = std::fs::write(file, bytes);
        eprintln!(
            "  saved {address:#x}+{len:#x} to {} ({saved:?})",
            file.display()
        );
    }
}

/// Return addresses of the faulting thread's frames, from the faulting
/// instruction up, by the unwind data of each image (the Windows loader
/// registered it for title images too). Code without unwind data (our
/// thunks, leaf functions) is taken to have its return address at `rsp`.
fn unwind(context: CONTEXT) -> Vec<u64> {
    let (mut low, mut high) = (0, 0);
    // SAFETY: writes the current thread's stack bounds.
    unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
    let mut frames = Vec::with_capacity(MAX_FRAMES);
    unwind_into(context, low, high, &mut frames);
    frames
}

/// A `CONTEXT` at the 16-byte alignment the unwind and capture functions need.
#[repr(C, align(16))]
pub(crate) struct Aligned16(pub(crate) CONTEXT);

/// [`unwind`] for any thread of the process, its stack being `low..high`;
/// pushes no more than `MAX_FRAMES`, so it does not allocate when `frames`
/// has that capacity (another thread may be suspended holding the heap).
pub(crate) fn unwind_into(context: CONTEXT, low: usize, high: usize, frames: &mut Vec<u64>) {
    // `RtlVirtualUnwind` writes the context with 16-byte aligned stores, and
    // windows-sys declares `CONTEXT` without that alignment: a by-value copy on
    // the stack can land 8 bytes off and fault (seen in a debug build).
    let mut aligned = Aligned16(context);
    let context = &mut aligned.0;
    frames.push(context.Rip);
    while frames.len() < MAX_FRAMES {
        // Stop where reading the stack would fault inside this handler.
        let rsp = context.Rsp as usize;
        if rsp < low || rsp + 8 > high {
            break;
        }
        let mut image = 0;
        // SAFETY: looks up the function table entry covering `Rip`, if any.
        let entry = unsafe { RtlLookupFunctionEntry(context.Rip, &mut image, null_mut()) };
        if entry.is_null() {
            // SAFETY: `rsp` is inside this thread's stack.
            context.Rip = unsafe { (rsp as *const u64).read() };
            context.Rsp += 8;
        } else {
            let (mut data, mut frame) = (null_mut(), 0);
            // SAFETY: `entry` belongs to `image`; the context is our copy.
            unsafe {
                RtlVirtualUnwind(
                    UNW_FLAG_NHANDLER,
                    image,
                    context.Rip,
                    entry,
                    context,
                    &mut data,
                    &mut frame,
                    null_mut(),
                )
            };
        }
        if context.Rip == 0 {
            break;
        }
        frames.push(context.Rip);
    }
}

/// `module+rva`, or the bare address.
pub fn locate(address: usize) -> String {
    module_of(address).unwrap_or_else(|| format!("{address:#x}"))
}

fn module_of(address: usize) -> Option<String> {
    let mut module: HMODULE = std::ptr::null_mut();
    // SAFETY: looks up the module containing `address`; no reference taken.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        )
    };
    if found == 0 {
        return None;
    }
    let mut path = [0u16; 260];
    // SAFETY: `module` is a loaded module; the buffer length is passed.
    let len = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) };
    let path = String::from_utf16_lossy(&path[..len as usize]);
    let name = path.rsplit('\\').next().unwrap_or(&path);
    Some(format!("{name}+{:#x}", address - module as usize))
}

/// What is at `address`, for an access violation there: nothing, memory
/// reserved but never committed, a guard page, or the protection the access
/// broke (with the module, for an image's pages). Each points at a different
/// cause: a wild pointer, an allocation whose commit failed, a stack
/// overrun, a write to code or constants.
fn memory_at(address: usize) -> String {
    use windows_sys::Win32::System::Memory::{
        MEM_FREE, MEM_IMAGE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE,
        PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD,
        PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY, VirtualQuery,
    };
    // SAFETY: a zeroed MEMORY_BASIC_INFORMATION is valid.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: reads the address space's bookkeeping, not `address` itself.
    let n = unsafe { VirtualQuery(address as *const _, &mut info, size_of_val(&info)) };
    if n == 0 {
        // Above the process's address space: Windows reports a
        // non-canonical address as all ones.
        return "outside the address space".into();
    }
    let protect = match info.State {
        MEM_FREE => return "nothing mapped there".into(),
        MEM_RESERVE => return "reserved, never committed".into(),
        _ if info.Protect & PAGE_GUARD != 0 => return "a guard page".into(),
        _ => match info.Protect & 0xff {
            PAGE_NOACCESS => "no access".into(),
            PAGE_READONLY => "read-only".into(),
            PAGE_READWRITE => "read-write".into(),
            PAGE_WRITECOPY => "copy-on-write".into(),
            PAGE_EXECUTE => "execute-only".into(),
            PAGE_EXECUTE_READ => "read-execute".into(),
            PAGE_EXECUTE_READWRITE => "read-write-execute".into(),
            PAGE_EXECUTE_WRITECOPY => "copy-on-write, executable".into(),
            _ => format!("protection {:#x}", info.Protect),
        },
    };
    match module_of(address) {
        Some(module) if info.Type == MEM_IMAGE => format!("{protect}, in {module}"),
        _ => protect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        EXCEPTION_RECORD, RtlCaptureContext, RtlCaptureStackBackTrace,
    };

    /// What [`capture_frames`] saw: our [`unwind`], then the system's walk.
    type Captured = (Vec<u64>, Vec<usize>);

    /// A frame we expect [`unwind`] to report: the helper it is in, and the
    /// helper's name for the message.
    type Expected = (&'static str, fn() -> Captured);

    /// The runtime function entry start covering `at`: the image base the
    /// lookup writes, plus the entry's `BeginAddress`. Asking the system for
    /// both sides of the comparison means a thunk or an incremental-link jump
    /// is not mistaken for a different function.
    fn entry_start_at(at: u64) -> Option<u64> {
        let mut image = 0;
        // SAFETY: looks up the function table entry covering `at`; nothing
        // keeps a reference to the entry past this call.
        let entry = unsafe { RtlLookupFunctionEntry(at, &mut image, null_mut()) };
        if entry.is_null() {
            return None;
        }
        // SAFETY: a non-null entry belongs to `image`, which the lookup wrote,
        // and `BeginAddress` is an offset into that image.
        Some(image + u64::from(unsafe { (*entry).BeginAddress }))
    }

    #[test]
    fn unwinds_like_the_system_walk() {
        // Asking the system's own walk whether the stack looks right is not
        // reliable: on Windows 10 x64 debug builds `RtlCaptureStackBackTrace`
        // returns no frames about half the time. The frames above us are
        // known instead (see `level_a`), so check those first.
        let (frames, walked) = level_a();
        // Frame 0 is in `capture_frames`, which took the context; then its
        // callers in call order, each named by its runtime function entry.
        let helpers: [Expected; 4] = [
            ("capture_frames", capture_frames),
            ("level_c", level_c),
            ("level_b", level_b),
            ("level_a", level_a),
        ];
        assert!(frames.len() >= helpers.len(), "{frames:?}");
        for (i, (name, helper)) in helpers.iter().enumerate() {
            let want = entry_start_at(*helper as usize as u64)
                .unwrap_or_else(|| panic!("no unwind data for {name}"));
            let got = entry_start_at(frames[i])
                .unwrap_or_else(|| panic!("no unwind data for frame {i} at {:#x}", frames[i]));
            assert_eq!(got, want, "frame {i} should be {name}: {frames:?}");
        }
        // A walk that repeats an address is not a walk.
        let mut distinct = frames[..helpers.len()].to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), helpers.len(), "repeated: {frames:?}");
        // Where the system's walk does return frames, it must still agree with
        // ours: frame 0 differs (both are in `capture_frames`, at different
        // call sites), and the callers above are the same.
        if walked.len() >= helpers.len() {
            assert!(frames.len() >= walked.len(), "{frames:?} vs {walked:?}");
            let ours: Vec<usize> = frames[1..walked.len()]
                .iter()
                .map(|&f| f as usize)
                .collect();
            assert_eq!(ours.as_slice(), &walked[1..]);
        }
    }

    /// The frames of this call, both ways: our [`unwind`], and the system's
    /// walk, which Windows 10 can come up empty on.
    #[inline(never)]
    fn capture_frames() -> Captured {
        // SAFETY: a zeroed CONTEXT is valid and both calls write buffers of
        // ours; `RtlCaptureContext` wants the context 16-byte aligned too.
        unsafe {
            let mut context = Aligned16(std::mem::zeroed());
            RtlCaptureContext(&mut context.0);
            let mut walked = [null_mut(); 8];
            let n = RtlCaptureStackBackTrace(0, 8, walked.as_mut_ptr(), null_mut());
            let frames = unwind(context.0);
            let walked = walked[..usize::from(n)]
                .iter()
                .map(|&f| f as usize)
                .collect();
            (frames, walked)
        }
    }

    /// `level_a` → `level_b` → `level_c` → [`capture_frames`]: three known
    /// callers above the capture. `black_box` after each call keeps it from
    /// becoming a tail call, which would erase the caller's frame.
    #[inline(never)]
    fn level_a() -> Captured {
        let captured = level_b();
        std::hint::black_box(&captured);
        captured
    }

    /// See [`level_a`].
    #[inline(never)]
    fn level_b() -> Captured {
        let captured = level_c();
        std::hint::black_box(&captured);
        captured
    }

    /// See [`level_a`].
    #[inline(never)]
    fn level_c() -> Captured {
        let captured = capture_frames();
        std::hint::black_box(&captured);
        captured
    }

    #[test]
    fn memory_at_names_what_an_access_hit() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_GUARD, PAGE_NOACCESS, PAGE_READONLY,
            PAGE_READWRITE, VirtualAlloc, VirtualFree,
        };
        // SAFETY: reserves four pages of ours and commits three of them;
        // nothing reads or writes them.
        let base = unsafe {
            let base = VirtualAlloc(std::ptr::null(), 0x4000, MEM_RESERVE, PAGE_NOACCESS);
            assert!(!base.is_null());
            let page = |i: usize, protect| {
                let at = base.cast::<u8>().add(i * 0x1000).cast();
                assert!(!VirtualAlloc(at, 0x1000, MEM_COMMIT, protect).is_null());
            };
            page(1, PAGE_READONLY);
            page(2, PAGE_READWRITE | PAGE_GUARD);
            page(3, PAGE_NOACCESS);
            base as usize
        };
        assert_eq!(memory_at(base + 8), "reserved, never committed");
        assert_eq!(memory_at(base + 0x1008), "read-only");
        assert_eq!(memory_at(base + 0x2008), "a guard page");
        assert_eq!(memory_at(base + 0x3008), "no access");
        // SAFETY: releases the reservation made above.
        unsafe { VirtualFree(base as *mut _, 0, MEM_RELEASE) };
        assert_eq!(memory_at(base + 8), "nothing mapped there");
        assert_eq!(memory_at(0), "nothing mapped there");
        assert_eq!(memory_at(usize::MAX), "outside the address space");
        // This test's own code: an image page, named by its module.
        let code = memory_at(memory_at_names_what_an_access_hit as *const () as usize);
        let named = code.starts_with("read-execute, in victory_loader-");
        assert!(named, "{code}");
    }

    /// On an Intel CPU, the real instructions go through the vectored
    /// handler. On an AMD CPU they run natively, and the tests below test
    /// the emulation.
    #[test]
    fn clwb_loops() {
        // clwb [rcx]; add rcx, 0x40; cmp rcx, rax; jbe top
        let a = clwb_loop(&[
            0x66, 0x0f, 0xae, 0x31, 0x48, 0x83, 0xc1, 0x40, 0x48, 0x39, 0xc1, 0x76, 0xf3,
        ])
        .expect("rcx loop");
        assert_eq!((a.base, a.limit, a.len), (1, 0, 13));
        // clwb [r9]; add r9, 0x40; cmp r9, rax; jbe top
        let b = clwb_loop(&[
            0x66, 0x41, 0x0f, 0xae, 0x31, 0x49, 0x83, 0xc1, 0x40, 0x49, 0x39, 0xc1, 0x76, 0xf2,
        ])
        .expect("r9 loop");
        assert_eq!((b.base, b.limit, b.len), (9, 0, 14));
        // A different stride, a jump elsewhere, or a cut-off loop is not one.
        assert!(
            clwb_loop(&[
                0x66, 0x0f, 0xae, 0x31, 0x48, 0x83, 0xc1, 0x20, 0x48, 0x39, 0xc1, 0x76, 0xf3
            ])
            .is_none()
        );
        assert!(
            clwb_loop(&[
                0x66, 0x0f, 0xae, 0x31, 0x48, 0x83, 0xc1, 0x40, 0x48, 0x39, 0xc1, 0x76, 0xf0
            ])
            .is_none()
        );
        assert!(clwb_loop(&[0x66, 0x0f, 0xae, 0x31, 0x48, 0x83, 0xc1, 0x40]).is_none());
        // The flags the last cmp leaves: base just past limit.
        assert_eq!(cmp_flags(0x1040, 0x1000) & 0x41, 0); // CF and ZF clear: the loop ends
        assert_eq!(cmp_flags(0x1000, 0x1000) & 0x40, 0x40);
    }

    #[test]
    fn clwb_loop_wrapping_ranges_do_not_overflow_and_fall_back_to_single_clwb() {
        assert_eq!(clwb_loop_end(0x1000, 0x1080), Some(0x10c0));
        assert_eq!(clwb_loop_end(0x10c0, 0x1080), Some(0x1100));
        assert_eq!(clwb_loop_end(0, u64::MAX - 10), None);
        assert_eq!(clwb_loop_end(u64::MAX - 16, 0x100), None);
        assert_eq!(clwb_loop_end(u64::MAX - 63, u64::MAX), None);

        // clwb [rcx]; add rcx, 0x40; cmp rcx, rax; jbe top (13 bytes, clwb is 4 bytes).
        let mut bytes = [0xcc_u8; 16];
        bytes[..13].copy_from_slice(&[
            0x66, 0x0f, 0xae, 0x31, 0x48, 0x83, 0xc1, 0x40, 0x48, 0x39, 0xc1, 0x76, 0xf3,
        ]);
        let run = |base: u64, limit: u64| -> (u64, u64) {
            let start = bytes.as_ptr() as u64;
            // SAFETY: zeroed record/context are valid; `emulate_amd_only` only
            // reads `bytes` and updates `context`.
            unsafe {
                let mut record: EXCEPTION_RECORD = std::mem::zeroed();
                record.ExceptionCode = EXCEPTION_ILLEGAL_INSTRUCTION;
                let mut context: CONTEXT = std::mem::zeroed();
                context.Rip = start;
                context.Rcx = base;
                context.Rax = limit;
                let mut pointers = EXCEPTION_POINTERS {
                    ExceptionRecord: &mut record,
                    ContextRecord: &mut context,
                };
                assert!(emulate_amd_only(&mut pointers));
                (context.Rip - start, context.Rcx)
            }
        };
        // Normal range completes the whole 13-byte loop in one exception.
        assert_eq!(run(0x1000, 0x1080), (13, 0x10c0));
        // Overflowing/wrapping ranges must not panic or skip the loop: emulate
        // just the 4-byte `clwb` instruction and let the CPU run add/cmp/jbe.
        assert_eq!(run(0, u64::MAX - 10), (4, 0));
        assert_eq!(run(u64::MAX - 16, 0x100), (4, u64::MAX - 16));
    }

    #[test]
    fn clwb_lengths() {
        assert_eq!(clwb_len(&[0x66, 0x41, 0x0f, 0xae, 0x31]), Some(5)); // clwb [r9]
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0x30]), Some(4)); // clwb [rax]
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0x70, 0x40]), Some(5)); // clwb [rax+0x40]
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0xb0, 0, 1, 0, 0]), Some(8)); // [rax+disp32]
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0x34, 0x24]), Some(5)); // clwb [rsp]
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0x35, 0, 0, 0, 0]), Some(8)); // [rip+disp32]
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0x38]), None); // clflushopt, /7
        assert_eq!(clwb_len(&[0x0f, 0xae, 0x30]), None); // xsaveopt, no 66
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0xf0]), None); // register form
    }

    #[test]
    fn a_clwb_at_the_end_of_a_page_is_decoded() {
        // Exact-length slices, as at the end of a page.
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0x30]), Some(4));
        assert_eq!(clwb_len(&[0x66, 0x41, 0x0f, 0xae, 0x31]), Some(5));
        // Cut short by the slice: returns None without panicking.
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0xb0, 0, 1]), None); // disp32, 6 of 8 bytes
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0x70]), None); // disp8, 4 of 5 bytes
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0x34]), None); // SIB missing
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae, 0x34, 0x25, 0, 0]), None); // SIB disp32 cut short
        assert_eq!(clwb_len(&[0x66, 0x0f, 0xae]), None); // ModRM missing
        assert_eq!(clwb_len(&[0x66]), None);
        assert_eq!(clwb_len(&[]), None);

        // Emulation at the last 4 and 5 bytes of a page with an unreadable page after it.
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_READWRITE, VirtualAlloc,
            VirtualFree, VirtualProtect,
        };
        // SAFETY: two pages of ours, the second unreadable so reading past the first faults.
        unsafe {
            let page = VirtualAlloc(
                std::ptr::null(),
                0x2000,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
            .cast::<u8>();
            assert!(!page.is_null());
            let mut old = 0;
            let guard = page.add(0x1000).cast();
            assert_ne!(VirtualProtect(guard, 0x1000, PAGE_NOACCESS, &mut old), 0);
            for insn in [
                &[0x66, 0x0f, 0xae, 0x30][..],
                &[0x66, 0x41, 0x0f, 0xae, 0x31][..],
            ] {
                let end = page.add(0x1000 - insn.len());
                end.copy_from_nonoverlapping(insn.as_ptr(), insn.len());
                let mut record: EXCEPTION_RECORD = std::mem::zeroed();
                record.ExceptionCode = EXCEPTION_ILLEGAL_INSTRUCTION;
                let mut context: CONTEXT = std::mem::zeroed();
                context.Rip = end as u64;
                let mut pointers = EXCEPTION_POINTERS {
                    ExceptionRecord: &mut record,
                    ContextRecord: &mut context,
                };
                assert!(emulate_amd_only(&mut pointers));
                assert_eq!(context.Rip, end as u64 + insn.len() as u64);
            }
            VirtualFree(page.cast(), 0, MEM_RELEASE);
        }
    }

    #[test]
    fn clzero_zeroes_its_line_on_any_cpu() {
        report_crashes();
        #[repr(align(64))]
        struct Line([u8; 128]);
        let mut line = Line([0xff; 128]);
        let at = line.0.as_mut_ptr().wrapping_add(5); // inside the first line
        // SAFETY: clzero writes only the 64-byte line holding `at`, ours.
        unsafe { std::arch::asm!(".byte 0x0f, 0x01, 0xfc", in("rax") at) };
        assert!(line.0[..64].iter().all(|&b| b == 0));
        assert!(line.0[64..].iter().all(|&b| b == 0xff));
        // insertq xmm0, xmm1, len 16, at 16; then extrq xmm0, len 8, at 20.
        let out: u64;
        // SAFETY: touches only the named registers.
        unsafe {
            std::arch::asm!(
                "movq xmm0, {a}", "movq xmm1, {b}",
                ".byte 0xf2, 0x0f, 0x78, 0xc1, 0x10, 0x10",
                "movq {a}, xmm0",
                ".byte 0x66, 0x0f, 0x78, 0xc0, 0x08, 0x14",
                "movq {b}, xmm0",
                a = inout(reg) 0x1111_2222_3333_4444u64 => _, b = inout(reg) 0xabcdu64 => out,
                out("xmm0") _, out("xmm1") _)
        };
        assert_eq!(out, 0xbc); // 0x1111_2222_abcd_4444 >> 20, 8 bits
        // SAFETY: monitorx/mwaitx are wait hints that write no memory.
        unsafe {
            std::arch::asm!(".byte 0x0f, 0x01, 0xfa", ".byte 0x0f, 0x01, 0xfb",
                in("rax") at, in("ecx") 0, in("edx") 0)
        };
        // rdpru of MPERF: a count in edx:eax, their high halves clear.
        let (low, high): (u64, u64);
        // SAFETY: touches only the named registers.
        unsafe {
            std::arch::asm!(".byte 0x0f, 0x01, 0xfd",
                out("rax") low, inout("rcx") 0u64 => _, out("rdx") high)
        };
        assert_ne!(high << 32 | low, 0);
        assert!(high >> 32 == 0 && low >> 32 == 0);
    }

    /// The flags a native `cmp a, b` leaves.
    fn native_compare(a: u64, b: u64) -> u32 {
        let flags: u64;
        // SAFETY: compares two registers and reads the flags.
        unsafe {
            std::arch::asm!("cmp {a}, {b}", "pushfq", "pop {f}",
                a = in(reg) a, b = in(reg) b, f = out(reg) flags)
        };
        flags as u32 & CMP_FLAGS
    }

    #[test]
    fn compare_sets_flags_like_the_cpu() {
        let values = [0, 1, 0x40, 0x7f, 0x80, 0xff, 1 << 63, (1 << 63) - 1, !0, !0 - 0x40];
        for a in values {
            for b in values {
                assert_eq!(cmp_flags(a, b), native_compare(a, b), "cmp {a:#x}, {b:#x}");
            }
        }
    }

    #[test]
    fn clzero_loops_end_like_their_last_pass() {
        report_crashes();
        #[repr(align(64))]
        struct Lines([u8; 192]);
        let zeroed = |l: &Lines| [l.0[0] == 0, l.0[64] == 0, l.0[128] == 0];
        // clzero; add rax, 0x40; cmp rax, r10; jb
        let mut mem = Lines([0xff; 192]);
        let start = mem.0.as_mut_ptr() as u64;
        let (end, flags): (u64, u64);
        // SAFETY: zeroes the first two lines of `mem`.
        unsafe {
            std::arch::asm!(
                ".byte 0x0f, 0x01, 0xfc, 0x48, 0x83, 0xc0, 0x40, 0x4c, 0x39, 0xd0, 0x72, 0xf4",
                "pushfq", "pop {f}",
                inout("rax") start => end, in("r10") start + 100, f = out(reg) flags)
        };
        assert_eq!(end, start + 128);
        assert_eq!(flags as u32 & CMP_FLAGS, native_compare(end, start + 100));
        assert_eq!(zeroed(&mem), [true, true, false]);
        // mov rax, rdx; add rdx, 0x40; clzero; cmp rdx, r8; jb
        let mut mem = Lines([0xff; 192]);
        let start = mem.0.as_mut_ptr() as u64;
        let (last, next, flags): (u64, u64, u64);
        // SAFETY: zeroes the last two lines of `mem`.
        unsafe {
            std::arch::asm!(
                ".byte 0x48, 0x89, 0xd0, 0x48, 0x83, 0xc2, 0x40, 0x0f, 0x01, 0xfc",
                ".byte 0x4c, 0x39, 0xc2, 0x72, 0xf1",
                "pushfq", "pop {f}",
                out("rax") last, inout("rdx") start + 64 => next, in("r8") start + 192,
                f = out(reg) flags)
        };
        assert_eq!((last, next), (start + 128, start + 192));
        assert_eq!(flags as u32 & CMP_FLAGS, native_compare(next, start + 192));
        assert_eq!(zeroed(&mem), [false, true, true]);
    }

    fn reg(low: u64, high: u64) -> M128A {
        M128A {
            Low: low,
            High: high as i64,
        }
    }

    /// Gives `code` to the emulation as if it faulted with `exception` at
    /// its first byte, with `rax` and `xmm` as the saved registers. The
    /// emulation does not execute `code`, so this works on any CPU. Returns
    /// the number of bytes that the emulation skipped, or `None` if it did
    /// not emulate the instruction. `xmm` gets the saved registers after.
    fn emulate_as(exception: i32, code: &[u8], rax: u64, xmm: &mut [M128A; 16]) -> Option<u64> {
        // `int3` after the instruction, so that a decoder that reads too
        // far sees no valid bytes.
        let mut bytes = [0xcc_u8; 16];
        bytes[..code.len()].copy_from_slice(code);
        let start = bytes.as_ptr() as u64;
        // SAFETY: a zeroed record and context are valid; the emulation
        // reads only `bytes` and writes only `context` (and memory at
        // `rax` for clzero, which the caller owns).
        unsafe {
            let mut record: EXCEPTION_RECORD = std::mem::zeroed();
            record.ExceptionCode = exception;
            let mut context: CONTEXT = std::mem::zeroed();
            context.Rip = start;
            context.Rax = rax;
            context.Anonymous.FltSave.XmmRegisters = *xmm;
            let mut pointers = EXCEPTION_POINTERS {
                ExceptionRecord: &mut record,
                ContextRecord: &mut context,
            };
            let emulated = emulate_amd_only(&mut pointers);
            *xmm = context.Anonymous.FltSave.XmmRegisters;
            emulated.then(|| context.Rip - start)
        }
    }

    fn emulate(code: &[u8], rax: u64, xmm: &mut [M128A; 16]) -> Option<u64> {
        emulate_as(EXCEPTION_ILLEGAL_INSTRUCTION, code, rax, xmm)
    }

    fn low(xmm: &[M128A; 16], i: usize) -> u64 {
        xmm[i].Low
    }

    #[test]
    fn emulated_clzero_zeroes_its_line() {
        #[repr(align(64))]
        struct Line([u8; 128]);
        let mut line = Line([0xff; 128]);
        let at = line.0.as_mut_ptr().wrapping_add(5) as u64; // inside the first line
        let mut xmm = [reg(0, 0); 16];
        assert_eq!(emulate(&[0x0f, 0x01, 0xfc], at, &mut xmm), Some(3));
        assert!(line.0[..64].iter().all(|&b| b == 0));
        assert!(line.0[64..].iter().all(|&b| b == 0xff));
    }

    /// Gives `code` to the emulation as if it faulted at `code[at]` with
    /// `context` as the saved state, on any CPU; returns whether it
    /// emulated and leaves the state after in `context`.
    fn emulate_in(code: &[u8], at: usize, context: &mut CONTEXT) -> bool {
        // At a fixed place in a page, so the loops' neighbours are always
        // read (they are not near a page's edges).
        #[repr(align(4096))]
        struct Page([u8; 4096]);
        let mut page = Box::new(Page([0xcc; 4096]));
        let bytes = &mut page.0[0x800..];
        bytes[..code.len()].copy_from_slice(code);
        context.Rip = bytes.as_ptr() as u64 + at as u64;
        // SAFETY: a zeroed record is valid; the emulation reads only
        // `bytes` and writes `context` and the lines the caller owns.
        unsafe {
            let mut record: EXCEPTION_RECORD = std::mem::zeroed();
            record.ExceptionCode = EXCEPTION_ILLEGAL_INSTRUCTION;
            let mut pointers = EXCEPTION_POINTERS {
                ExceptionRecord: &mut record,
                ContextRecord: context,
            };
            let emulated = emulate_amd_only(&mut pointers);
            context.Rip -= bytes.as_ptr() as u64;
            emulated
        }
    }

    #[test]
    fn emulated_clzero_loops_end_like_their_last_pass() {
        #[repr(align(64))]
        struct Lines([u8; 192]);
        let zeroed = |l: &Lines| [l.0[0] == 0, l.0[64] == 0, l.0[128] == 0];
        // clzero; add rax, 0x40; cmp rax, r10; jb
        let mut mem = Lines([0xff; 192]);
        let start = mem.0.as_mut_ptr() as u64;
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rax = start;
        context.R10 = start + 100;
        context.EFlags = 0x202 | CMP_FLAGS;
        let code = [0x0f, 0x01, 0xfc, 0x48, 0x83, 0xc0, 0x40, 0x4c, 0x39, 0xd0, 0x72, 0xf4];
        assert!(emulate_in(&code, 0, &mut context));
        assert_eq!((context.Rip, context.Rax), (12, start + 128));
        assert_eq!(context.EFlags, 0x202 | native_compare(start + 128, start + 100));
        assert_eq!(zeroed(&mem), [true, true, false]);
        // mov rax, rdx; add rdx, 0x40; clzero; cmp rdx, r8; jb (faults at clzero,
        // after the first pass's mov and add)
        let mut mem = Lines([0xff; 192]);
        let start = mem.0.as_mut_ptr() as u64;
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rax = start + 64;
        context.Rdx = start + 128;
        context.R8 = start + 192;
        let code = [
            0x48, 0x89, 0xd0, 0x48, 0x83, 0xc2, 0x40, 0x0f, 0x01, 0xfc, 0x4c, 0x39, 0xc2, 0x72,
            0xf1,
        ];
        assert!(emulate_in(&code, 7, &mut context));
        assert_eq!((context.Rip, context.Rax, context.Rdx), (15, start + 128, start + 192));
        assert_eq!(context.EFlags, native_compare(start + 192, start + 192));
        assert_eq!(zeroed(&mem), [false, true, true]);
        // cmp rax, rax is not the loop (the limit would move with rax): one
        // line, flags untouched.
        let mut mem = Lines([0xff; 192]);
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rax = mem.0.as_mut_ptr() as u64;
        context.EFlags = 0x202;
        let code = [0x0f, 0x01, 0xfc, 0x48, 0x83, 0xc0, 0x40, 0x48, 0x39, 0xc0, 0x72, 0xf4];
        assert!(emulate_in(&code, 0, &mut context));
        assert_eq!((context.Rip, context.EFlags), (3, 0x202));
        assert_eq!(zeroed(&mem), [true, false, false]);
        // Any other neighbours: one line, then the next instruction.
        let mut mem = Lines([0xff; 192]);
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rax = mem.0.as_mut_ptr() as u64 + 70;
        context.R10 = !0;
        let code = [0x0f, 0x01, 0xfc, 0x48, 0x83, 0xc0, 0x20, 0x4c, 0x39, 0xd0, 0x72, 0xf4];
        assert!(emulate_in(&code, 0, &mut context));
        assert_eq!(context.Rip, 3);
        assert_eq!(zeroed(&mem), [false, true, false]);
    }

    #[test]
    fn emulated_rdpru_reads_known_counters_and_keeps_flags() {
        let cases = [(0, true), (1, true), (2, false), (0x1_0000_0000, true)];
        let starts = [0x202, 0x202 | CMP_FLAGS];
        for ((ecx, known), flags) in cases.into_iter().flat_map(|c| starts.map(|f| (c, f))) {
            // SAFETY: an all-zero context is valid.
            let mut context: CONTEXT = unsafe { std::mem::zeroed() };
            context.Rcx = ecx;
            context.Rax = !0;
            context.Rdx = !0;
            context.EFlags = flags;
            assert!(emulate_in(&[0x0f, 0x01, 0xfd], 0, &mut context));
            assert_eq!(context.Rip, 3);
            assert_eq!(context.EFlags, flags, "ecx {ecx:#x}");
            let count = context.Rdx << 32 | context.Rax;
            assert!(context.Rax >> 32 == 0 && context.Rdx >> 32 == 0);
            assert_eq!(count != 0, known, "ecx {ecx:#x}");
        }
    }

    #[test]
    fn emulated_monitorx_and_mwaitx_are_skipped() {
        let mut xmm = [reg(1, 2); 16];
        assert_eq!(emulate(&[0x0f, 0x01, 0xfa], 0, &mut xmm), Some(3));
        assert_eq!(emulate(&[0x0f, 0x01, 0xfb], 0, &mut xmm), Some(3));
        assert!(xmm.iter().all(|r| r.Low == 1 && r.High == 2));
    }

    #[test]
    fn emulated_insertq_and_extrq_with_immediates() {
        let mut xmm = [reg(0, 0); 16];
        xmm[0] = reg(0x1111_2222_3333_4444, 0x5555);
        xmm[1] = reg(0xabcd, 0x6666);
        // insertq xmm0, xmm1, length 16, index 16
        let insertq = [0xf2, 0x0f, 0x78, 0xc1, 0x10, 0x10];
        assert_eq!(emulate(&insertq, 0, &mut xmm), Some(6));
        assert_eq!(low(&xmm, 0), 0x1111_2222_abcd_4444);
        assert_eq!((low(&xmm, 1), xmm[1].High), (0xabcd, 0x6666));
        // extrq xmm0, length 8, index 20
        let extrq = [0x66, 0x0f, 0x78, 0xc0, 0x08, 0x14];
        assert_eq!(emulate(&extrq, 0, &mut xmm), Some(6));
        assert_eq!(low(&xmm, 0), 0xbc);
    }

    #[test]
    fn emulated_insertq_and_extrq_with_fields_in_a_register() {
        let mut xmm = [reg(0, 0); 16];
        // insertq xmm0, xmm1: length and index in xmm1's high half.
        xmm[0] = reg(0x1111_2222_3333_4444, 0);
        xmm[1] = reg(0xabcd, 0x10_10);
        assert_eq!(emulate(&[0xf2, 0x0f, 0x79, 0xc1], 0, &mut xmm), Some(4));
        assert_eq!(low(&xmm, 0), 0x1111_2222_abcd_4444);
        // extrq xmm0, xmm1: length and index in xmm1's low half.
        xmm[1] = reg(0x14_08, 0);
        assert_eq!(emulate(&[0x66, 0x0f, 0x79, 0xc1], 0, &mut xmm), Some(4));
        assert_eq!(low(&xmm, 0), 0xbc);
    }

    #[test]
    fn emulated_sse4a_reaches_xmm8_to_xmm15() {
        let mut xmm = [reg(0, 0); 16];
        xmm[8] = reg(0x1111_2222_3333_4444, 0);
        xmm[9] = reg(0xabcd, 0);
        // insertq xmm8, xmm9, length 16, index 16 (REX.R and REX.B)
        let insertq = [0xf2, 0x45, 0x0f, 0x78, 0xc1, 0x10, 0x10];
        assert_eq!(emulate(&insertq, 0, &mut xmm), Some(7));
        assert_eq!(low(&xmm, 8), 0x1111_2222_abcd_4444);
        assert_eq!((low(&xmm, 0), low(&xmm, 1)), (0, 0));
        // extrq xmm9, length 8, index 4 (REX.B)
        let extrq = [0x66, 0x41, 0x0f, 0x78, 0xc1, 0x08, 0x04];
        assert_eq!(emulate(&extrq, 0, &mut xmm), Some(7));
        assert_eq!(low(&xmm, 9), 0xbc);
        assert_eq!(low(&xmm, 1), 0);
    }

    #[test]
    fn emulated_sse4a_length_0_is_64_bits() {
        let mut xmm = [reg(0, 0); 16];
        xmm[0] = reg(0x1122_3344_5566_7788, 0);
        // extrq xmm0, length 0 (64), index 0. A non-zero index with
        // length 64 goes past bit 63, and AMD leaves that result undefined.
        assert_eq!(
            emulate(&[0x66, 0x0f, 0x78, 0xc0, 0x00, 0x00], 0, &mut xmm),
            Some(6)
        );
        assert_eq!(low(&xmm, 0), 0x1122_3344_5566_7788);
        // insertq xmm0, xmm1, length 0 (64), index 0
        xmm[1] = reg(0xdead_beef_cafe_f00d, 0);
        assert_eq!(
            emulate(&[0xf2, 0x0f, 0x78, 0xc1, 0x00, 0x00], 0, &mut xmm),
            Some(6)
        );
        assert_eq!(low(&xmm, 0), 0xdead_beef_cafe_f00d);
    }

    #[test]
    fn other_instructions_and_exceptions_are_not_emulated() {
        let mut xmm = [reg(0, 0); 16];
        assert_eq!(emulate(&[0x0f, 0x0b], 0, &mut xmm), None); // ud2
        // extrq with a memory operand, which does not exist
        assert_eq!(emulate(&[0x66, 0x0f, 0x79, 0x00], 0, &mut xmm), None);
        // extrq with immediates needs ModRM.reg 0
        let bad_extrq = [0x66, 0x0f, 0x78, 0xc8, 0x08, 0x14];
        assert_eq!(emulate(&bad_extrq, 0, &mut xmm), None);
        // a clzero that did not cause an illegal instruction
        let clzero = [0x0f, 0x01, 0xfc];
        assert_eq!(
            emulate_as(EXCEPTION_ACCESS_VIOLATION, &clzero, 0, &mut xmm),
            None
        );
    }

    #[test]
    fn instruction_bytes_stop_at_the_end_of_their_page() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_READWRITE, VirtualAlloc,
            VirtualFree, VirtualProtect,
        };
        // rdpru, then the next instruction (mov rcx, rax) and padding.
        let mut code = [0x90_u8; 16];
        code[..6].copy_from_slice(&[0x0f, 0x01, 0xfd, 0x48, 0x89, 0xc1]);
        // SAFETY: two fresh pages of ours, the second made unreadable, so a
        // read past the end of the first faults.
        unsafe {
            let page = VirtualAlloc(
                std::ptr::null(),
                0x2000,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
            .cast::<u8>();
            assert!(!page.is_null());
            let mut old = 0;
            let guard = page.add(0x1000).cast();
            assert_ne!(VirtualProtect(guard, 0x1000, PAGE_NOACCESS, &mut old), 0);
            page.copy_from_nonoverlapping(code.as_ptr(), code.len());
            let whole = "0f 01 fd 48 89 c1 90 90 90 90 90 90 90 90 90";
            assert_eq!(instruction_bytes(page as usize), whole);
            // The page's last three bytes: rdpru, and nothing read after it.
            let end = page.add(0x1000 - 3);
            end.copy_from_nonoverlapping(code.as_ptr(), 3);
            assert_eq!(instruction_bytes(end as usize), "0f 01 fd");
            VirtualFree(page.cast(), 0, MEM_RELEASE);
        }
    }

    /// `bytes` at a fixed place in a page (so the guard's reads land where
    /// they expect), as the two instructions of a vtable call: what the
    /// decode makes of them, with the instruction before at `pre`.
    fn vtable_call(pre: &[u8], call: &[u8]) -> Option<(usize, u8, u64)> {
        let at = 32;
        let mut code = [0x90u8; 64];
        code[at - pre.len()..at].copy_from_slice(pre);
        code[at..at + call.len()].copy_from_slice(call);
        null_vtable_call(&code, at).map(|c| (c.len, c.vtable, c.slot))
    }

    #[test]
    fn only_a_loaded_vtable_pointer_called_is_a_null_vtable_call() {
        // The title's own site: mov (%rdi),%r15; call *0x8(%r15).
        let load_r15 = [0x4c, 0x8b, 0x3f];
        let call_r15_8 = [0x41, 0xff, 0x57, 0x08];
        assert_eq!(
            vtable_call(&load_r15, &call_r15_8),
            Some((4, 15, 0x8)),
            "the site from the crash"
        );
        // Any registers, the first slot, and a `jmp` for a tail call.
        assert_eq!(
            vtable_call(&[0x48, 0x8b, 0x18], &[0xff, 0x13]),
            Some((2, 3, 0))
        );
        assert_eq!(
            vtable_call(&[0x48, 0x8b, 0x18], &[0xff, 0x23]),
            Some((2, 3, 0))
        );
        // A base that needs a SIB, and a disp32 slot.
        assert_eq!(
            vtable_call(&[0x4c, 0x8b, 0x21], &[0x41, 0xff, 0x54, 0x24, 0x10]),
            Some((5, 12, 0x10))
        );
        assert_eq!(
            vtable_call(&load_r15, &[0x41, 0xff, 0x97, 0x78, 0x00, 0x00, 0x00]),
            Some((7, 15, 0x78))
        );
        // Not the shape: a register call, a call whose register was never
        // loaded from memory, slots that are not slots, a load with a
        // displacement or into another register, and RIP-relative.
        assert_eq!(vtable_call(&load_r15, &[0xff, 0xd7]), None);
        assert_eq!(vtable_call(&[0x90, 0x90, 0x90], &call_r15_8), None);
        assert_eq!(
            vtable_call(
                &load_r15,
                &[0x41, 0xff, 0x97, 0x80, 0x00, 0x00, 0x00]
            ),
            None
        );
        assert_eq!(vtable_call(&load_r15, &[0x41, 0xff, 0x57, 0x0c]), None);
        assert_eq!(
            vtable_call(&[0x4c, 0x8b, 0x7f, 0x08], &call_r15_8),
            None
        );
        assert_eq!(vtable_call(&load_r15, &[0xff, 0x10]), None);
        assert_eq!(vtable_call(&load_r15, &[0xff, 0x15, 0, 0, 0, 0]), None);
    }

    /// As if the exception at `code[at]` were an access violation of `kind`
    /// reading `fault`, with `context` the state there: how far the guard
    /// advanced `rip`, or `None` if it did not skip.
    fn skip_at(
        code: &[u8],
        at: usize,
        context: &mut CONTEXT,
        fault: usize,
        kind: usize,
    ) -> Option<usize> {
        #[repr(align(4096))]
        struct Code([u8; 4096]);
        let mut page = Box::new(Code([0xcc; 4096]));
        page.0[0x800..0x800 + code.len()].copy_from_slice(code);
        let start = page.0.as_ptr() as usize + 0x800 + at;
        context.Rip = start as u64;
        // SAFETY: a zeroed record is valid; the guard reads only the code
        // page, the record and the context.
        let mut record: EXCEPTION_RECORD = unsafe { std::mem::zeroed() };
        record.ExceptionCode = EXCEPTION_ACCESS_VIOLATION;
        record.NumberParameters = 2;
        record.ExceptionInformation[0] = kind;
        record.ExceptionInformation[1] = fault;
        skip_null_vtable(context, &record).then(|| context.Rip as usize - start)
    }

    #[test]
    fn a_null_vtable_call_is_skipped_as_a_call_that_returned_zero() {
        // mov (%rdi),%r15; call *0x8(%r15), as the title's code has it.
        let code = [0x4c, 0x8b, 0x3f, 0x41, 0xff, 0x57, 0x08];
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.R15 = 0;
        context.Rax = 0xdead_beef;
        context.EFlags = 0x202 | CMP_FLAGS;
        assert_eq!(skip_at(&code, 3, &mut context, 0x8, 0), Some(4));
        // The call's result: nothing, a call returns in rax. The flags a
        // call leaves: as they were.
        assert_eq!(context.Rax, 0);
        assert_eq!(context.EFlags, 0x202 | CMP_FLAGS);
        // A `jmp` (a tail call) is skipped the same way.
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        let jump = [0x4c, 0x8b, 0x3f, 0x41, 0xff, 0x67, 0x08];
        assert_eq!(skip_at(&jump, 3, &mut context, 0x8, 0), Some(4));

        // Anything but the exact fault is reported as before: a vtable
        // pointer that is not null, a fault at another address, a write, one
        // above the null page, and an instruction of another shape.
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.R15 = 0x1000;
        assert_eq!(skip_at(&code, 3, &mut context, 0x1008, 0), None);
        // SAFETY: as above.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        assert_eq!(skip_at(&code, 3, &mut context, 0x10, 0), None);
        assert_eq!(skip_at(&code, 3, &mut context, 0x8, 1), None);
        assert_eq!(skip_at(&code, 3, &mut context, 0x1_0000, 0), None);
        let call_only = [0x90, 0x90, 0x90, 0x41, 0xff, 0x57, 0x08];
        assert_eq!(skip_at(&call_only, 3, &mut context, 0x8, 0), None);
    }

    #[test]
    fn a_site_skipped_too_often_is_reported_instead() {
        // A site of its own, so the other tests' skips do not count.
        #[repr(align(4096))]
        struct Code([u8; 4096]);
        let page = Box::new(Code([0xcc; 4096]));
        let rip = page.0.as_ptr() as usize + 0x800;
        for n in 1..=SKIP_LIMIT {
            assert_eq!(count_skip(rip), Some(n), "skip {n}");
        }
        assert_eq!(count_skip(rip), None, "past the limit, inside the window");
        // Another site still has its own allowance.
        assert_eq!(count_skip(rip + 0x100), Some(1));
    }

    /// The chain guard skips nothing outside a title image, so its code
    /// lives in a page registered as one. The page is leaked: its address
    /// stays out of every other test's allocations, so no other test's code
    /// can land inside the span, and the span stays good for the process.
    fn title_code(slot: usize, code: &[u8]) -> usize {
        assert!(slot < 60, "slot {slot} is past the page's 60");
        assert!(code.len() <= 0x20, "{} bytes is past a slot", code.len());
        static BASE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        let base = *BASE.get_or_init(|| {
            #[repr(align(4096))]
            struct Page([u8; 4096]);
            let page = Box::leak(Box::new(Page([0xcc; 4096])));
            let base = page.0.as_ptr() as usize;
            title_image(base, page.0.len());
            base
        });
        // SAFETY: `base` is a leaked page of ours, and each slot is written
        // by one test only (the slot numbers are the tests').
        unsafe {
            std::ptr::copy_nonoverlapping(
                code.as_ptr(),
                (base + 0x40 * slot) as *mut u8,
                code.len(),
            );
        }
        base + 0x40 * slot
    }

    /// The record of an access violation of `kind` (0 read, 1 write) at
    /// `fault`.
    fn access_violation(fault: usize, kind: usize) -> EXCEPTION_RECORD {
        // SAFETY: an all-zero record is valid.
        let mut record: EXCEPTION_RECORD = unsafe { std::mem::zeroed() };
        record.ExceptionCode = EXCEPTION_ACCESS_VIOLATION;
        record.NumberParameters = 2;
        record.ExceptionInformation[0] = kind;
        record.ExceptionInformation[1] = fault;
        record
    }

    /// As if a thread whose `base` register holds `value` faulted reading
    /// `fault` at `code`, at `slot` of the title page: how far the chain
    /// guard advanced `rip`, and the state after.
    fn chain_skip(
        slot: usize,
        (base, value): (u8, u64),
        code: &[u8],
        fault: usize,
        kind: usize,
    ) -> (Option<usize>, CONTEXT) {
        let start = title_code(slot, code);
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rip = start as u64;
        context.EFlags = 0x202 | CMP_FLAGS;
        *gpr(&mut context, base) = value;
        let record = access_violation(fault, kind);
        let skipped = skip_null_chain(&mut context, &record);
        (skipped.then(|| context.Rip as usize - start), context)
    }

    #[test]
    fn a_chain_load_is_a_rex_w_load_through_one_base_register() {
        let load = |code: &[u8]| chain_load(code, 0).map(|l| (l.len, l.dst, l.base, l.disp));
        // The title's own loads: movq 0x18(%rdx),%r8, movq 0x1a8(%rcx),%rax,
        // and the same shape with no displacement, through rbp, and with a
        // disp32.
        assert_eq!(load(&[0x4c, 0x8b, 0x42, 0x18]), Some((4, 8, 2, 0x18)));
        assert_eq!(
            load(&[0x48, 0x8b, 0x81, 0xa8, 0x01, 0x00, 0x00]),
            Some((7, 0, 1, 0x1a8))
        );
        assert_eq!(load(&[0x48, 0x8b, 0x07]), Some((3, 0, 7, 0)));
        assert_eq!(load(&[0x48, 0x8b, 0x4d, 0x20]), Some((4, 1, 5, 0x20)));
        // Not one: a register as the source, a SIB's index, RIP-relative (no
        // base at all), a store, a 32-bit load, a base above the first eight
        // registers, displacements out of range, and a cut-off instruction.
        assert_eq!(load(&[0x48, 0x8b, 0xc2]), None);
        assert_eq!(load(&[0x48, 0x8b, 0x04, 0x10]), None);
        assert_eq!(load(&[0x48, 0x8b, 0x05, 0x18, 0x00, 0x00, 0x00]), None);
        assert_eq!(load(&[0x48, 0x89, 0x42, 0x18]), None);
        assert_eq!(load(&[0x44, 0x8b, 0x42, 0x18]), None);
        assert_eq!(load(&[0x49, 0x8b, 0x42, 0x18]), None);
        assert_eq!(load(&[0x48, 0x8b, 0x87, 0x00, 0x00, 0x01, 0x00]), None);
        assert_eq!(load(&[0x48, 0x8b, 0x47, 0x80]), None);
        assert_eq!(load(&[0x48, 0x8b, 0x87, 0xf0, 0xff, 0xff, 0xff]), None);
        assert_eq!(load(&[0x48, 0x8b, 0x81, 0xa8, 0x01]), None);
        assert_eq!(load(&[0x4c, 0x8b]), None);
        assert_eq!(load(&[]), None);
    }

    #[test]
    fn a_null_chain_load_is_skipped_as_a_load_of_zero() {
        let before = NULL_CHAIN_SKIPS.load(Relaxed);
        // The title's own site: movq 0x18(%rdx),%r8 with rdx = 0, which
        // faults reading 0x18 (run30's 0x1400381f0: 4c 8b 42 18). The load
        // is skipped with r8 = 0, the value a null object's field has and
        // the one the title's testq after it expects; a load leaves the
        // flags as they were.
        let (skipped, context) = chain_skip(0, (2, 0), &[0x4c, 0x8b, 0x42, 0x18], 0x18, 0);
        assert_eq!(skipped, Some(4));
        assert_eq!(context.R8, 0);
        assert_eq!(context.EFlags, 0x202 | CMP_FLAGS);
        assert!(NULL_CHAIN_SKIPS.load(Relaxed) > before, "skips are counted");
        // The same shape at the chain's other loads: no displacement, and a
        // disp32.
        assert_eq!(chain_skip(1, (7, 0), &[0x48, 0x8b, 0x07], 0, 0).0, Some(3));
        assert_eq!(
            chain_skip(
                2,
                (1, 0),
                &[0x48, 0x8b, 0x81, 0xa8, 0x01, 0x00, 0x00],
                0x1a8,
                0
            )
            .0,
            Some(7)
        );
    }

    #[test]
    fn anything_but_a_null_base_at_its_own_displacement_is_not_skipped() {
        let observed = [0x4c, 0x8b, 0x42, 0x18];
        let skip =
            |slot, base, code: &[u8], fault, kind| chain_skip(slot, base, code, fault, kind).0;
        // A base that is not null: the object the chain wanted is there, so
        // a fault is something else.
        assert_eq!(skip(4, (2, 0x1000), &observed, 0x1018, 0), None);
        // Another fault address than the displacement, a write, and one
        // above the null page.
        assert_eq!(skip(5, (2, 0), &observed, 0x20, 0), None);
        assert_eq!(skip(6, (2, 0), &observed, 0x18, 1), None);
        assert_eq!(skip(7, (2, 0), &observed, 0x1_0000, 0), None);
        // Displacements the shape does not have: 0x10000 and up, and
        // negative ones.
        assert_eq!(
            skip(8, (7, 0), &[0x48, 0x8b, 0x87, 0x00, 0x00, 0x01, 0x00], 0x18, 0),
            None
        );
        assert_eq!(skip(9, (7, 0), &[0x48, 0x8b, 0x47, 0x80], 0x18, 0), None);
        assert_eq!(
            skip(
                10,
                (7, 0),
                &[0x48, 0x8b, 0x87, 0xf0, 0xff, 0xff, 0xff],
                0x18,
                0
            ),
            None
        );
        // A SIB (an index is no base), RIP-relative (no base register at
        // all), and a register form (no memory read at all).
        assert_eq!(skip(11, (0, 0), &[0x48, 0x8b, 0x04, 0x10], 0, 0), None);
        assert_eq!(
            skip(12, (5, 0), &[0x48, 0x8b, 0x05, 0x18, 0x00, 0x00, 0x00], 0x18, 0),
            None
        );
        assert_eq!(skip(13, (2, 0), &[0x48, 0x8b, 0xc2], 0x18, 0), None);
        // Not a 64-bit load, and not a load.
        assert_eq!(skip(14, (2, 0), &[0x44, 0x8b, 0x42, 0x18], 0x18, 0), None);
        assert_eq!(skip(15, (2, 0), &[0x48, 0x89, 0x42, 0x18], 0x18, 0), None);
        // Not an access violation.
        let at = title_code(16, &observed);
        // SAFETY: an all-zero record and context are valid.
        let mut record: EXCEPTION_RECORD = unsafe { std::mem::zeroed() };
        record.ExceptionCode = EXCEPTION_ILLEGAL_INSTRUCTION;
        // SAFETY: as above.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rip = at as u64;
        assert!(!skip_null_chain(&mut context, &record));
        // SAFETY: as above.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        // Code outside every title image, the host's: the guard leaves it to
        // be reported.
        #[repr(align(4096))]
        struct Outside([u8; 4096]);
        let page = Box::leak(Box::new(Outside([0xcc; 4096])));
        page.0[0x800..0x804].copy_from_slice(&observed);
        context.Rip = page.0.as_ptr() as u64 + 0x800;
        assert!(!skip_null_chain(&mut context, &access_violation(0x18, 0)));
    }

    #[test]
    fn a_chain_site_skipped_too_often_is_reported_instead() {
        // A site of its own, so the other tests' skips do not count: the
        // vtable guard's table and its breaker are the chain guard's too.
        let code = [0x4c, 0x8b, 0x42, 0x18];
        for n in 1..=SKIP_LIMIT {
            assert_eq!(chain_skip(20, (2, 0), &code, 0x18, 0).0, Some(4), "skip {n}");
        }
        assert_eq!(
            chain_skip(20, (2, 0), &code, 0x18, 0).0,
            None,
            "past the limit, inside the window"
        );
    }

    #[test]
    fn a_vtable_load_is_found_behind_the_calls_argument_setup() {
        // run48's site: mov (%rdi),%r15; mov rcx,rdi; mov edx,ebx;
        // mov r14,rax; call *0x8(%r15). The load does not end at the call,
        // and that is what the guard used to require.
        let load = [0x4c, 0x8b, 0x3f];
        let setup = [0x48, 0x89, 0xf9, 0x89, 0xda, 0x49, 0x89, 0xc6];
        let call = [0x41, 0xff, 0x57, 0x08];
        let mut pre = Vec::new();
        pre.extend_from_slice(&load);
        pre.extend_from_slice(&setup);
        assert_eq!(vtable_call(&pre, &call), Some((4, 15, 0x8)), "run48's site");

        // The load alone, as before.
        assert_eq!(vtable_call(&load, &call), Some((4, 15, 0x8)));
        // One `mov reg, reg` between them, and one into the call's own base
        // register (the one that is not allowed).
        assert_eq!(
            vtable_call(&[0x4c, 0x8b, 0x3f, 0x48, 0x89, 0xf9], &call),
            Some((4, 15, 0x8))
        );
        assert_eq!(
            vtable_call(&[0x4c, 0x8b, 0x3f, 0x49, 0x89, 0xc7], &call),
            None,
            "the call's register is written again"
        );
        // A load through memory between them, of any register: argument setup
        // touches no memory, and a fault it could have caused is not this
        // call's.
        assert_eq!(
            vtable_call(&[0x4c, 0x8b, 0x3f, 0x48, 0x8b, 0x47, 0x08], &call),
            None
        );
        // Anything that is not a register move: an address computation, an
        // immediate, a `push`.
        assert_eq!(
            vtable_call(&[0x4c, 0x8b, 0x3f, 0x48, 0x8d, 0x4f, 0x10], &call),
            None
        );
        assert_eq!(
            vtable_call(&[0x4c, 0x8b, 0x3f, 0xb8, 0x10, 0x00, 0x00, 0x00], &call),
            None
        );
        assert_eq!(
            vtable_call(&[0x4c, 0x8b, 0x3f, 0x53], &call),
            None
        );
        // A load of another register, with the setup after it.
        assert_eq!(
            vtable_call(&[0x48, 0x8b, 0x07, 0x48, 0x89, 0xf9], &call),
            None
        );
        // Past the walk back's 16 bytes: the load, five argument moves, and
        // the call — the load starts 18 bytes before it, and the walk does not
        // go that far.
        let far = [
            0x4c, 0x8b, 0x3f, 0x48, 0x89, 0xf9, 0x48, 0x89, 0xf9, 0x48, 0x89, 0xf9, 0x48, 0x89,
            0xf9, 0x48, 0x89, 0xf9,
        ];
        assert_eq!(vtable_call(&far, &call), None, "18 bytes back is past the reach");
    }

    #[test]
    fn a_null_vtable_call_whose_load_is_behind_argument_setup_is_skipped() {
        // The whole of run48's site, through the guard: the load, the three
        // argument moves, and the call through the register it filled.
        let code = [
            0x4c, 0x8b, 0x3f, 0x48, 0x89, 0xf9, 0x89, 0xda, 0x49, 0x89, 0xc6, 0x41, 0xff, 0x57,
            0x08,
        ];
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.R15 = 0;
        context.Rax = 0xdead_beef;
        assert_eq!(skip_at(&code, 11, &mut context, 0x8, 0), Some(4));
        assert_eq!(context.Rax, 0, "a call's result: nothing");
    }

    #[test]
    fn a_chain_load_may_be_a_movzx_or_a_movsx_of_a_word() {
        let load = |code: &[u8]| chain_load(code, 0).map(|l| (l.len, l.dst, l.base, l.disp, l.width));
        // run44's site: movzx r8d, WORD [rax+0x18] with rax = 0.
        assert_eq!(load(&[0x44, 0x0f, 0xb7, 0x40, 0x18]), Some((5, 8, 0, 0x18, 2)));
        // The same with a sign extension, with a 64-bit destination, with no
        // displacement, and with a disp32.
        assert_eq!(load(&[0x44, 0x0f, 0xbf, 0x40, 0x18]), Some((5, 8, 0, 0x18, 2)));
        assert_eq!(load(&[0x4c, 0x0f, 0xb7, 0x40, 0x18]), Some((5, 8, 0, 0x18, 2)));
        assert_eq!(load(&[0x44, 0x0f, 0xb7, 0x02]), Some((4, 8, 2, 0, 2)));
        assert_eq!(
            load(&[0x44, 0x0f, 0xb7, 0x82, 0x18, 0x00, 0x00, 0x00]),
            Some((8, 8, 2, 0x18, 2))
        );
        // Not one: a byte form, a register as the source, a SIB, RIP-relative
        // (no base at all), an index register, a base above the first eight,
        // a negative displacement, and an instruction cut off.
        assert_eq!(load(&[0x44, 0x0f, 0xb6, 0x40, 0x18]), None);
        assert_eq!(load(&[0x44, 0x0f, 0xb7, 0xc0]), None);
        assert_eq!(load(&[0x44, 0x0f, 0xb7, 0x04, 0x10]), None);
        assert_eq!(load(&[0x44, 0x0f, 0xb7, 0x05, 0x18, 0, 0, 0]), None);
        assert_eq!(load(&[0x46, 0x0f, 0xb7, 0x40, 0x18]), None);
        assert_eq!(load(&[0x45, 0x0f, 0xb7, 0x40, 0x18]), None);
        assert_eq!(load(&[0x44, 0x0f, 0xb7, 0x40, 0xf0]), None);
        assert_eq!(load(&[0x44, 0x0f, 0xb7, 0x80, 0x18]), None);
    }

    #[test]
    fn a_null_movzx_load_is_skipped_as_a_load_of_zero() {
        let before = NULL_CHAIN_SKIPS.load(Relaxed);
        // run44's site, through the guard: movzx r8d, WORD [rax+0x18] with
        // rax = 0, which faults reading 0x18. r8 is the value a null object's
        // word has, and nothing else is emulated.
        let (skipped, context) = chain_skip(40, (0, 0), &[0x44, 0x0f, 0xb7, 0x40, 0x18], 0x18, 0);
        assert_eq!(skipped, Some(5));
        assert_eq!(context.R8, 0);
        assert_eq!(context.EFlags, 0x202 | CMP_FLAGS);
        assert!(NULL_CHAIN_SKIPS.load(Relaxed) > before, "skips are counted");
        // The same load with a sign extension, and one whose base is not
        // null: not this fault.
        assert_eq!(
            chain_skip(40, (0, 0), &[0x44, 0x0f, 0xbf, 0x40, 0x18], 0x18, 0).0,
            Some(5)
        );
        assert_eq!(
            chain_skip(41, (0, 0x1000), &[0x44, 0x0f, 0xb7, 0x40, 0x18], 0x1018, 0).0,
            None
        );
        // A write through the same register is the write guard's, not this
        // one's.
        assert_eq!(
            chain_skip(41, (0, 0), &[0x44, 0x0f, 0xb7, 0x40, 0x18], 0x18, 1).0,
            None
        );
    }

    /// As if the store in `code`, at `slot` of the title page, faulted
    /// writing `fault` with `base`'s register holding `value`: how far the
    /// write guard advanced `rip`, or `None` if it did not skip.
    fn write_skip(
        slot: usize,
        (base, value): (u8, u64),
        code: &[u8],
        fault: usize,
        kind: usize,
    ) -> Option<usize> {
        let start = title_code(slot, code);
        // SAFETY: an all-zero context is valid.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rip = start as u64;
        *gpr(&mut context, base) = value;
        let record = access_violation(fault, kind);
        skip_small_write(&mut context, &record).then(|| context.Rip as usize - start)
    }

    #[test]
    fn a_store_through_a_small_pointer_is_skipped() {
        let before = WRITE_SKIPS.load(Relaxed);
        // run46's site: mov [rdx+8],al with rdx = 0xf0 — a block's header
        // word, a size, where a pointer belongs — which faults writing 0xf8.
        assert_eq!(write_skip(42, (2, 0xf0), &[0x88, 0x42, 0x08], 0xf8, 1), Some(3));
        assert!(WRITE_SKIPS.load(Relaxed) > before, "skips are counted");
        // The rest of the record builder: a byte store of a REX'd register, a
        // dword store, a word of an immediate, and a store with no
        // displacement.
        assert_eq!(
            write_skip(42, (2, 0xf0), &[0x44, 0x88, 0x42, 0x09], 0xf9, 1),
            Some(4)
        );
        assert_eq!(write_skip(43, (2, 0xf0), &[0x89, 0x42, 0x0c], 0xfc, 1), Some(3));
        assert_eq!(
            write_skip(43, (2, 0xf0), &[0x66, 0xc7, 0x42, 0x0a, 0x00, 0x00], 0xfa, 1),
            Some(6)
        );
        assert_eq!(write_skip(44, (2, 0xf0), &[0x88, 0x02], 0xf0, 1), Some(2));
        // A base that is null (a record built on a null object), and one with
        // a disp32.
        assert_eq!(write_skip(44, (2, 0), &[0x88, 0x42, 0x08], 0x8, 1), Some(3));
        assert_eq!(
            write_skip(45, (2, 0xf0), &[0x88, 0x82, 0x08, 0x00, 0x00, 0x00], 0xf8, 1),
            Some(6)
        );
    }

    #[test]
    fn anything_but_a_small_base_at_its_own_store_is_not_skipped() {
        let store = [0x88, 0x42, 0x08];
        // A read (the load guards'), a base that is a real pointer (the store
        // of a record that is there), and a fault at an address the store
        // does not make.
        assert_eq!(write_skip(46, (2, 0xf0), &store, 0xf8, 0), None);
        assert_eq!(
            write_skip(46, (2, 0x6261_03000), &store, 0x6261_03008, 1),
            None
        );
        assert_eq!(write_skip(46, (2, 0xf0), &store, 0xf9, 1), None);
        // An address at the top of what counts as small: 0xfff8 is, and the
        // store's own address reaching 0x10000 is not.
        assert_eq!(write_skip(49, (2, 0xfff8), &[0x88, 0x02], 0xfff8, 1), Some(2));
        assert_eq!(
            write_skip(49, (2, 0xfff8), &[0x88, 0x82, 0x08, 0x00, 0x00, 0x00], 0x1_0000, 1),
            None
        );
        // A base that is no small address at all: a real pointer's store is
        // a store the title made.
        assert_eq!(
            write_skip(49, (2, 0x1_0000), &[0x88, 0x82, 0x08, 0x00, 0x00, 0x00], 0x1_0008, 1),
            None
        );
        // A store with a SIB (an index register is no base), a register as
        // the source, a `C7` that is not a store (`/7`), and an instruction
        // that is no store at all.
        assert_eq!(
            write_skip(47, (2, 0xf0), &[0x4a, 0x89, 0x54, 0xca, 0x10], 0x10, 1),
            None
        );
        assert_eq!(write_skip(47, (2, 0xf0), &[0x88, 0xc2], 0xf0, 1), None);
        assert_eq!(
            write_skip(47, (2, 0xf0), &[0xc7, 0xfa, 0x00, 0x00, 0x00, 0x00], 0x4, 1),
            None
        );
        assert_eq!(write_skip(3, (2, 0xf0), &[0x8b, 0x42, 0x08], 0xf8, 1), None);
        // Not an access violation, and code outside every title image: the
        // guard leaves both to be reported.
        let at = title_code(48, &store);
        // SAFETY: an all-zero record and context are valid.
        let mut record: EXCEPTION_RECORD = unsafe { std::mem::zeroed() };
        record.ExceptionCode = EXCEPTION_ILLEGAL_INSTRUCTION;
        // SAFETY: as above.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rip = at as u64;
        context.Rdx = 0xf0;
        assert!(!skip_small_write(&mut context, &record));
        #[repr(align(4096))]
        struct Outside([u8; 4096]);
        let page = Box::leak(Box::new(Outside([0xcc; 4096])));
        page.0[0x800..0x803].copy_from_slice(&store);
        // SAFETY: as above.
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rip = page.0.as_ptr() as u64 + 0x800;
        context.Rdx = 0xf0;
        assert!(!skip_small_write(&mut context, &access_violation(0xf8, 1)));
    }

    #[test]
    fn a_write_site_skipped_too_often_is_reported_instead() {
        // A site of its own: the write guard shares the other two's table and
        // breaker.
        let code = [0x88, 0x42, 0x08];
        for n in 1..=SKIP_LIMIT {
            assert_eq!(write_skip(51, (2, 0xf0), &code, 0xf8, 1), Some(3), "skip {n}");
        }
        assert_eq!(
            write_skip(51, (2, 0xf0), &code, 0xf8, 1),
            None,
            "past the limit, inside the window"
        );
    }

    #[test]
    fn the_three_survival_guards_keep_their_own_switch() {
        // mov (%rdi),%r15; call *0x8(%r15) at one site, the chain's load at
        // another, a store through a small pointer at a third, with the
        // registers null (or small) as at the title's own sites.
        let call = title_code(21, &[0x4c, 0x8b, 0x3f, 0x41, 0xff, 0x57, 0x08]) + 3;
        let load = title_code(22, &[0x4c, 0x8b, 0x42, 0x18]);
        let store = title_code(50, &[0x88, 0x42, 0x08]);
        let attempt = |at: usize, fault: usize, kind: usize, base: u64, on: [bool; 3]| {
            // SAFETY: an all-zero context is valid.
            let mut context: CONTEXT = unsafe { std::mem::zeroed() };
            context.Rip = at as u64;
            context.Rdx = base;
            skip_null_faults(&mut context, &access_violation(fault, kind), on[0], on[1], on[2])
        };
        // Each guard skips its own shape with its own switch on, whatever the
        // others say, and none with its switch off.
        assert!(attempt(call, 0x8, 0, 0, [true, true, true]));
        assert!(attempt(call, 0x8, 0, 0, [true, false, false]));
        assert!(!attempt(call, 0x8, 0, 0, [false, true, true]));
        assert!(attempt(load, 0x18, 0, 0, [true, true, true]));
        assert!(attempt(load, 0x18, 0, 0, [false, true, false]));
        assert!(!attempt(load, 0x18, 0, 0, [true, false, true]));
        assert!(attempt(store, 0xf8, 1, 0xf0, [true, true, true]));
        assert!(attempt(store, 0xf8, 1, 0xf0, [false, false, true]));
        assert!(!attempt(store, 0xf8, 1, 0xf0, [true, true, false]));
    }
}
