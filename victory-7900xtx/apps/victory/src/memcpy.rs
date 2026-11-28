//! `memcpy` and `memmove` of our own, in place of the C runtime's, for every
//! copy of bytes in this program.
//!
//! Under Wine the C runtime's copies end in ntdll's `memcpy`, a loop of one
//! byte at a step (`dlls/ntdll/string.c`): 8 % of the submitting thread's
//! processor time at the clothes shop (run 102, `CPU-BOTTLENECK-ANALYSIS.md`
//! section 11). Defined here, in the executable, the linker takes these before
//! the import library's (as for `memcmp` in zocram4cc/victory-linux-perf
//! `202c2f2c4`, DoubleGhosty `d78b9eb5b`); other modules keep their own.
//! Ours is `rep movsb`, which processors with fast short string moves (Zen 3
//! and later, Ice Lake and later) run at full speed from a few bytes up, and
//! which the compiler cannot turn back into a call of this function.
//!
//! On Windows itself the C runtime's own (`vcruntime140.dll`) is called
//! instead, found once: what runs there is what ran before this file.
//! `VICTORY_OWN_MEMCPY=0` (read here) uses the C runtime's under Wine too.

#[cfg(all(windows, target_arch = "x86_64"))]
type Copy = unsafe extern "C" fn(*mut core::ffi::c_void, *const core::ffi::c_void, usize) -> *mut core::ffi::c_void;

/// The functions to call: 0 not decided yet, 1 ours, else the address of the
/// C runtime's (`memcpy` and `memmove` each).
#[cfg(all(windows, target_arch = "x86_64"))]
static CHOSEN: [core::sync::atomic::AtomicUsize; 2] =
    [core::sync::atomic::AtomicUsize::new(0), core::sync::atomic::AtomicUsize::new(0)];

#[cfg(all(windows, target_arch = "x86_64"))]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut core::ffi::c_void;
    fn LoadLibraryA(name: *const u8) -> *mut core::ffi::c_void;
    fn GetProcAddress(module: *mut core::ffi::c_void, name: *const u8) -> usize;
    fn GetEnvironmentVariableA(name: *const u8, buffer: *mut u8, size: u32) -> u32;
}

/// Under Wine (its ntdll exports `wine_get_version`) ours, unless
/// `VICTORY_OWN_MEMCPY=0`; on Windows the C runtime's. Nothing here copies
/// bytes with this program's `memcpy` (the lookups are the system's code).
#[cfg(all(windows, target_arch = "x86_64"))]
#[cold]
fn choose(which: usize) -> usize {
    let name: &core::ffi::CStr = if which == 0 { c"memcpy" } else { c"memmove" };
    // SAFETY: plain module, export and environment lookups with NUL-terminated
    // names; the buffer holds 4 bytes.
    let runtime = unsafe {
        let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
        let wine = !ntdll.is_null() && GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()) != 0;
        let mut value = [0u8; 4];
        let n = GetEnvironmentVariableA(c"VICTORY_OWN_MEMCPY".as_ptr().cast(), value.as_mut_ptr(), 4);
        let refused = n == 1 && value[0] == b'0';
        if wine && !refused {
            0
        } else {
            let mut module = GetModuleHandleA(c"vcruntime140.dll".as_ptr().cast());
            if module.is_null() {
                module = LoadLibraryA(c"vcruntime140.dll".as_ptr().cast());
            }
            if module.is_null() { 0 } else { GetProcAddress(module, name.as_ptr().cast()) }
        }
    };
    let chosen = if runtime > 1 { runtime } else { 1 };
    CHOSEN[which].store(chosen, core::sync::atomic::Ordering::Relaxed);
    chosen
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[inline(always)]
fn chosen(which: usize) -> usize {
    match CHOSEN[which].load(core::sync::atomic::Ordering::Relaxed) {
        0 => choose(which),
        c => c,
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dst: *mut core::ffi::c_void, src: *const core::ffi::c_void, n: usize) -> *mut core::ffi::c_void {
    let c = chosen(0);
    if c == 1 {
        // SAFETY: as the caller promised: `n` bytes, not overlapping.
        unsafe { forward(dst, src, n) };
        return dst;
    }
    // SAFETY: the C runtime's `memcpy`, with what the caller passed.
    unsafe { core::mem::transmute::<usize, Copy>(c)(dst, src, n) }
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dst: *mut core::ffi::c_void, src: *const core::ffi::c_void, n: usize) -> *mut core::ffi::c_void {
    let c = chosen(1);
    if c == 1 {
        // SAFETY: as the caller promised: `n` bytes at each, perhaps overlapping.
        unsafe {
            if (dst as usize).wrapping_sub(src as usize) >= n {
                // The destination does not begin inside the source: a forward
                // copy never writes a byte it has still to read.
                forward(dst, src, n);
            } else {
                backward(dst, src, n);
            }
        }
        return dst;
    }
    // SAFETY: the C runtime's `memmove`, with what the caller passed.
    unsafe { core::mem::transmute::<usize, Copy>(c)(dst, src, n) }
}

/// `n` bytes from `src` to `dst`, lowest first.
#[cfg(all(windows, target_arch = "x86_64"))]
#[inline(always)]
unsafe fn forward(dst: *mut core::ffi::c_void, src: *const core::ffi::c_void, n: usize) {
    // SAFETY: the caller's promise; the direction flag is clear, as the ABI
    // guarantees at every call.
    unsafe {
        core::arch::asm!(
            "rep movsb",
            inout("rdi") dst => _,
            inout("rsi") src => _,
            inout("rcx") n => _,
            options(nostack, preserves_flags)
        );
    }
}

/// `n` bytes from `src` to `dst`, highest first (the destination begins
/// inside the source). Rare: plain backward string moves are slow, but right.
#[cfg(all(windows, target_arch = "x86_64"))]
#[inline(never)]
unsafe fn backward(dst: *mut core::ffi::c_void, src: *const core::ffi::c_void, n: usize) {
    if n == 0 {
        return;
    }
    // SAFETY: the caller's promise; the direction flag is set only for this
    // one move and cleared again before returning, as the ABI requires.
    unsafe {
        core::arch::asm!(
            "std",
            "rep movsb",
            "cld",
            inout("rdi") dst.cast::<u8>().add(n - 1) => _,
            inout("rsi") src.cast::<u8>().add(n - 1) => _,
            inout("rcx") n => _,
            options(nostack)
        );
    }
}

#[cfg(all(test, windows, target_arch = "x86_64"))]
mod tests {
    use super::{memcpy, memmove};

    #[test]
    fn copies_match_a_byte_at_a_time() {
        let src: Vec<u8> = (0..4099u32).map(|i| (i * 7 + 3) as u8).collect();
        for n in [0usize, 1, 7, 15, 16, 17, 63, 64, 65, 255, 4096, 4099] {
            for at in [0usize, 1, 3] {
                if at + n > src.len() {
                    continue;
                }
                let mut dst = vec![0xAAu8; n + 8];
                // SAFETY: both buffers hold the bytes copied.
                unsafe { memcpy(dst.as_mut_ptr().add(2).cast(), src.as_ptr().add(at).cast(), n) };
                assert_eq!(&dst[2..2 + n], &src[at..at + n], "memcpy n={n} at={at}");
                assert!(dst[..2].iter().chain(&dst[2 + n..]).all(|&b| b == 0xAA), "memcpy wrote outside, n={n}");
            }
        }
    }

    #[test]
    fn overlapping_moves_match_copy_within() {
        for (from, to, n) in [(0usize, 1usize, 100usize), (1, 0, 100), (0, 37, 500), (37, 0, 500), (5, 5, 10), (0, 1, 1)] {
            let mut a: Vec<u8> = (0..600u32).map(|i| i as u8).collect();
            let mut b = a.clone();
            b.copy_within(from..from + n, to);
            // SAFETY: both ranges lie inside `a`.
            unsafe { memmove(a.as_mut_ptr().add(to).cast(), a.as_ptr().add(from).cast(), n) };
            assert_eq!(a, b, "memmove from={from} to={to} n={n}");
        }
    }
}
