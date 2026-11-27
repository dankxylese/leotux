//! Written pages of write-watched memory asked of Linux directly
//! (`VICTORY_LINUX_SCAN=1`, under Wine with native calls, [`crate::wine`]).
//!
//! Proton keeps `MEM_WRITE_WATCH` memory write-protected with userfaultfd
//! (asynchronous: a write unprotects its page in the kernel and marks it
//! written) and answers `GetWriteWatch` with the `PAGEMAP_SCAN` ioctl, under
//! Wine's one lock over all virtual memory: ~20 ms a GB in the world where
//! the walk alone costs ~0.2 ms. The same ioctl from here asks and protects
//! again in one step without that lock. Only ranges inside one reservation
//! the runtime made write-watched are asked ([`watched`]): elsewhere the
//! scan would stop part way, after protecting what it had found.

use std::collections::BTreeMap;
use std::sync::RwLock;

/// Write-watched reservations, by address: start -> end.
static WATCHED: RwLock<BTreeMap<usize, usize>> = RwLock::new(BTreeMap::new());

/// `[at, at + len)` was reserved write-watched (`MEM_WRITE_WATCH`).
pub fn memory_watched(at: usize, len: usize) {
    if len != 0 {
        WATCHED.write().unwrap_or_else(|e| e.into_inner()).insert(at, at.saturating_add(len));
    }
}

/// The reservations in `[at, at + len)` (`len` 0: the one at `at`) are gone.
pub fn memory_released(at: usize, len: usize) {
    let mut w = WATCHED.write().unwrap_or_else(|e| e.into_inner());
    let end = if len == 0 { w.get(&at).copied().unwrap_or(at + 1) } else { at.saturating_add(len) };
    let gone: Vec<usize> = w.range(..end).rev().take_while(|&(_, &e)| e > at).map(|(&s, _)| s).collect();
    for s in gone {
        w.remove(&s);
    }
}

/// Whether `[start, end)` lies in one write-watched reservation.
fn watched(start: usize, end: usize) -> bool {
    WATCHED
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .range(..=start)
        .next_back()
        .is_some_and(|(_, &e)| e >= end)
}

/// Whether the ioctl way is on: under Wine, unless `VICTORY_LINUX_SCAN=0`
/// (upstream has it off; this tree has it on, the switch falling back to the
/// host's `GetWriteWatch` as it was), and where native calls are there
/// ([`fd`]: `VICTORY_LINUX_SYSCALLS=0` or no
/// `WINE_HACK_NATIVE_SYSCALLS=1` makes them absent, and the asking is made
/// as before).
fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = crate::wine::switch("VICTORY_LINUX_SCAN", true) && fd().is_some();
        if on {
            tracing::info!("written pages of write-watched memory asked of Linux directly (VICTORY_LINUX_SCAN)");
        }
        on
    })
}

/// `/proc/self/pagemap`, opened with a native call.
fn fd() -> Option<usize> {
    static FD: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *FD.get_or_init(|| {
        const SYS_OPENAT: usize = 257;
        let path = c"/proc/self/pagemap";
        let fd = crate::wine::linux_syscall(SYS_OPENAT, [-100isize as usize, path.as_ptr() as usize, 0o2000000, 0, 0, 0])?;
        (fd >= 0).then_some(fd as usize)
    })
}

#[repr(C)]
struct ScanArg {
    size: u64,
    flags: u64,
    start: u64,
    end: u64,
    walk_end: u64,
    vec: u64,
    vec_len: u64,
    max_pages: u64,
    category_inverted: u64,
    category_mask: u64,
    category_anyof_mask: u64,
    return_mask: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Region {
    start: u64,
    end: u64,
    categories: u64,
}

const PAGEMAP_SCAN: usize = 0xc060_6610;
const PM_SCAN_WP_MATCHING: u64 = 1;
const PM_SCAN_CHECK_WPASYNC: u64 = 2;
const PAGE_IS_WRITTEN: u64 = 1 << 1;
const SYS_IOCTL: usize = 16;

/// The 4 KiB pages of `[start, end)` written since last asked, protected
/// again (start addresses, in order); `None` where this way is not taken
/// (off, not one write-watched reservation, or the kernel refused: the
/// caller asks as before).
pub(crate) fn written(start: usize, end: usize) -> Option<Vec<usize>> {
    if !enabled() || !watched(start, end) {
        return None;
    }
    let fd = fd()?;
    let mut regions = [Region::default(); 512];
    let mut pages = Vec::new();
    let mut at = start;
    while at < end {
        let mut arg = ScanArg {
            size: size_of::<ScanArg>() as u64,
            flags: PM_SCAN_WP_MATCHING | PM_SCAN_CHECK_WPASYNC,
            start: at as u64,
            end: end as u64,
            walk_end: 0,
            vec: regions.as_mut_ptr() as u64,
            vec_len: regions.len() as u64,
            max_pages: 0,
            category_inverted: 0,
            category_mask: PAGE_IS_WRITTEN,
            category_anyof_mask: 0,
            return_mask: PAGE_IS_WRITTEN,
        };
        let n = crate::wine::linux_syscall(SYS_IOCTL, [fd, PAGEMAP_SCAN, (&raw mut arg) as usize, 0, 0, 0])?;
        if n < 0 {
            static TOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !TOLD.swap(true, std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(errno = -n, start = format_args!("{start:#x}"), "PAGEMAP_SCAN refused: asked as before");
            }
            return None;
        }
        for r in &regions[..n as usize] {
            pages.extend((r.start as usize..r.end as usize).step_by(4096));
        }
        if arg.walk_end as usize <= at {
            break;
        }
        at = arg.walk_end as usize;
    }
    Some(pages)
}

#[cfg(test)]
mod tests {
    /// With native calls and `VICTORY_LINUX_SCAN=1` under Wine: a page
    /// written is found once, then not again until written again; what
    /// Wine's own `GetWriteWatch` says afterwards does not matter here.
    #[test]
    fn written_pages_are_found_once() {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};
        const MEM_WRITE_WATCH: u32 = 0x20_0000;
        const LEN: usize = 1 << 20;
        // SAFETY: fresh memory of ours.
        let at = unsafe { VirtualAlloc(std::ptr::null(), LEN, MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH, PAGE_READWRITE) } as usize;
        assert_ne!(at, 0);
        super::memory_watched(at, LEN);
        let Some(_) = super::written(at, at + LEN) else {
            assert!(std::env::var_os("VICTORY_LINUX_SCAN").is_none() || !crate::wine::on_wine(), "asked for, but not taken");
            return;
        };
        // SAFETY: inside our memory.
        unsafe { ((at + 3 * 4096 + 7) as *mut u8).write_volatile(1) };
        assert_eq!(super::written(at, at + LEN), Some(vec![at + 3 * 4096]));
        assert_eq!(super::written(at, at + LEN), Some(vec![]));
        unsafe { ((at + 200 * 4096) as *mut u8).write_volatile(2) };
        unsafe { ((at + 201 * 4096) as *mut u8).write_volatile(2) };
        assert_eq!(super::written(at, at + LEN), Some(vec![at + 200 * 4096, at + 201 * 4096]));
        // Not one reservation: not taken.
        assert_eq!(super::written(at, at + 2 * LEN), None);
    }
}
