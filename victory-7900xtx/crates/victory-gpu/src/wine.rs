//! Running under Wine (Linux): what the host's answers cost there, and the
//! ways around them.

/// Whether we run under Wine (its ntdll exports `wine_get_version`). The
/// same probe as [`crate::host::on_wine`], kept here as upstream #475 has it
/// so this file merges cleanly with the mirror.
pub(crate) fn on_wine() -> bool {
    static WINE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WINE.get_or_init(|| {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
        // SAFETY: plain lookups in ntdll, always loaded.
        unsafe {
            let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
            !ntdll.is_null() && GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()).is_some()
        }
    })
}

/// Whether a switch `name` is on: `1` on, `0` off, unset: `wine_default`
/// under Wine and off elsewhere.
pub(crate) fn switch(name: &str, wine_default: bool) -> bool {
    match std::env::var(name).as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => wine_default && on_wine(),
    }
}

// Linux system calls from our code under Wine. Wine runs a `syscall` made
// from Windows code as an NT service (syscall user dispatch). Proton lets
// code at 0x7001_0000_0000 or above make native ones when
// `WINE_HACK_NATIVE_SYSCALLS=1` (its handler widens the thread's allowed
// range once). A copy of the stub below is put there.
//
// The stub: Windows x64 arguments (nr, a1..a6 in rcx, rdx, r8, r9, then the
// stack) to Linux ones (rax; rdi, rsi, rdx, r10, r8, r9). rdi and rsi are
// the caller's to keep in the Windows convention; rcx and r11 (which
// `syscall` takes) are not.
core::arch::global_asm!(
    ".globl victory_linux_syscall_stub",
    ".globl victory_linux_syscall_stub_end",
    "victory_linux_syscall_stub:",
    "push rdi",
    "push rsi",
    "mov rax, rcx",
    "mov rdi, rdx",
    "mov rsi, r8",
    "mov rdx, r9",
    "mov r10, [rsp + 0x38]",
    "mov r8, [rsp + 0x40]",
    "mov r9, [rsp + 0x48]",
    "syscall",
    "pop rsi",
    "pop rdi",
    "ret",
    "victory_linux_syscall_stub_end:",
);

unsafe extern "C" {
    static victory_linux_syscall_stub: u8;
    static victory_linux_syscall_stub_end: u8;
}

type Syscall = unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize) -> isize;

/// Whether native calls are asked for: unless `VICTORY_LINUX_SYSCALLS=0`
/// (upstream asks with `=1`; this tree has them on, the switch falling back
/// to the Wine ways). Nothing is done where Proton's own
/// `WINE_HACK_NATIVE_SYSCALLS=1` is not there ([`make_native`]).
fn asked() -> bool {
    std::env::var_os("VICTORY_LINUX_SYSCALLS").is_none_or(|v| v != "0")
}

/// The copy of the stub Wine lets make native calls, once checked; `None`
/// where that is not so (not Wine, not asked for ([`asked`]), Proton's
/// switch not set, no room up there, or the self-test failed).
fn native() -> Option<Syscall> {
    static STUB: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let at = (*STUB.get_or_init(|| {
        let made = make_native();
        match made {
            Ok(at) => {
                tracing::info!(at = format_args!("{at:#x}"), "native Linux system calls from the runtime (VICTORY_LINUX_SYSCALLS)");
                Some(at)
            }
            Err(why) => {
                if asked() {
                    tracing::warn!(why, "no native Linux system calls: the Wine ways are used");
                }
                None
            }
        }
    }))?;
    // SAFETY: a copy of the stub, made executable and tested.
    Some(unsafe { std::mem::transmute::<usize, Syscall>(at) })
}

fn make_native() -> Result<usize, &'static str> {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_READWRITE, VirtualAlloc, VirtualProtect,
    };
    if !asked() {
        return Err("not asked for");
    }
    if !on_wine() {
        return Err("not under Wine");
    }
    // Checked before any call: without it the call would be an NT service.
    if std::env::var_os("WINE_HACK_NATIVE_SYSCALLS").is_none_or(|v| v != "1") {
        return Err("WINE_HACK_NATIVE_SYSCALLS=1 is not set");
    }
    // SAFETY: the two symbols bound the stub's bytes in our image.
    let (from, to) = (&raw const victory_linux_syscall_stub as usize, &raw const victory_linux_syscall_stub_end as usize);
    let len = to - from;
    let mut at = 0usize;
    for k in 0..64usize {
        let want = 0x7001_0000_0000usize + k * 0x10_0000_0000;
        // SAFETY: a fresh page at an address of our choosing, or nothing.
        let got = unsafe { VirtualAlloc(want as *const _, 4096, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) } as usize;
        if got >= 0x7001_0000_0000 {
            at = got;
            break;
        }
    }
    if at == 0 {
        return Err("no page at 0x7001_0000_0000 or above");
    }
    // SAFETY: `len` bytes of the stub into our fresh page, then made executable.
    unsafe {
        std::ptr::copy_nonoverlapping(from as *const u8, at as *mut u8, len);
        let mut old = 0;
        if VirtualProtect(at as *const _, 4096, PAGE_EXECUTE_READ, &mut old) == 0 {
            return Err("the page could not be made executable");
        }
    }
    // Self-test: getpid as Linux answers it is the process /proc/self/stat,
    // read natively, is of. (Not read through Wine: Wine opens files in its
    // server, whose "self" that is.) Were the call taken for an NT service,
    // neither would come back right.
    // SAFETY: the stub just copied; getpid takes no arguments.
    let f: Syscall = unsafe { std::mem::transmute::<usize, Syscall>(at) };
    let pid = unsafe { f(SYS_GETPID, 0, 0, 0, 0, 0, 0) };
    let stat = read_native(f, c"/proc/self/stat").ok_or("self-test: native openat/read failed")?;
    if pid <= 0 || stat.split(' ').next().and_then(|p| p.parse::<isize>().ok()) != Some(pid) {
        return Err("self-test: getpid and /proc/self/stat differ");
    }
    Ok(at)
}

const SYS_GETPID: usize = 39;

const SYS_READ: usize = 0;
const SYS_CLOSE: usize = 3;
const SYS_OPENAT: usize = 257;
const AT_FDCWD: usize = -100isize as usize;
const O_RDONLY_CLOEXEC: usize = 0o2000000;

/// A small file read with native calls (at most 4 KiB).
fn read_native(f: Syscall, path: &std::ffi::CStr) -> Option<String> {
    // SAFETY: a NUL-terminated path and a buffer of ours.
    unsafe {
        let fd = f(SYS_OPENAT, AT_FDCWD, path.as_ptr() as usize, O_RDONLY_CLOEXEC, 0, 0, 0);
        if fd < 0 {
            return None;
        }
        let mut buf = [0u8; 4096];
        let n = f(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0, 0);
        f(SYS_CLOSE, fd as usize, 0, 0, 0, 0, 0);
        (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
    }
}

/// A Linux system call, raw: its result (`-errno` on failure); `None` where
/// native calls are not available ([`native`]).
pub(crate) fn linux_syscall(nr: usize, args: [usize; 6]) -> Option<isize> {
    let f = native()?;
    // SAFETY: the caller's arguments for system call `nr`.
    Some(unsafe { f(nr, args[0], args[1], args[2], args[3], args[4], args[5]) })
}

#[cfg(test)]
mod tests {
    /// With `WINE_HACK_NATIVE_SYSCALLS=1` under Wine (and not
    /// `VICTORY_LINUX_SYSCALLS=0`): native calls work and answer as Linux
    /// does. Elsewhere: none. (The ways are on by default here, so the
    /// failure this test names is the one where both were asked for by hand
    /// and the stub still did not come up.)
    #[test]
    fn native_calls_answer_as_linux_does() {
        let Some(pid) = super::linux_syscall(super::SYS_GETPID, [0; 6]) else {
            let asked = std::env::var_os("VICTORY_LINUX_SYSCALLS").is_none_or(|v| v != "1");
            let proton = std::env::var_os("WINE_HACK_NATIVE_SYSCALLS").is_none_or(|v| v != "1");
            assert!(
                asked || proton || !super::on_wine(),
                "asked for, but no native calls: {:?}",
                super::make_native()
            );
            return;
        };
        assert!(pid > 0);
        // Again, from another thread (each traps once first).
        let other = std::thread::spawn(|| super::linux_syscall(super::SYS_GETPID, [0; 6])).join().unwrap();
        assert_eq!(other, Some(pid));
    }
}
