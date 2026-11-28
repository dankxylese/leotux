//! The F-probe (`docs/f-probe.md`): can the card read title memory in place
//! through a section, under Wine? Windows only — the body is in `probe.rs`,
//! so that the workspace still builds on Linux.

#[cfg(windows)]
mod probe;

#[cfg(windows)]
fn main() {
    probe::main();
}

#[cfg(not(windows))]
fn main() {
    println!("the F-probe is a Windows program: build it for x86_64-pc-windows-msvc and run it under the launcher's Wine (docs/f-probe.md)");
}
