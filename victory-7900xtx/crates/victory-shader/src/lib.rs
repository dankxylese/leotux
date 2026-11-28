//! Xbox Series shaders to host shaders.
//!
//! An Xbox Series shader is a standard DXBC container ([`dxbc`]). Its `DXIL`
//! part ([`dxil`]) may carry Xbox additions around the LLVM bitcode: an
//! `XDAT` table before it indexing blobs after it, among them `xbop`
//! (compiler options) and the zlib-compressed GFX10 code. The bitcode may use
//! console-only intrinsics, so the GFX10 code is translated in general.

pub mod bvh;
pub mod dxbc;
pub mod dxil;
pub mod emit;
pub mod gfx10;
pub mod io;
pub mod ir;
pub mod native;
pub mod resource;
pub mod spill;
pub mod structure;

/// The version of what the translator makes, part of the key of every
/// cached translation (`victory-gpu`'s `shader_cache`): **bump it with any
/// change that changes the translation of a shader** (its DXIL or what the
/// host binds for it). Changes that do not (speed, diagnostics, tests,
/// shaders that did not translate before) leave it, and every shader
/// translated before is kept across the rebuild. `victory shaders`
/// translates again what an earlier build of the translator cached and says
/// how many came out different: if any did, the bump was due.
/// (0x4c00_0001 and up: the Linux tree's, with #927's half views and the
/// raw typed loads; upstream's own numbers stay below. 0x4c00_0003: the
/// Vice-Tay AMD scalar rules — an `s_min`/`s_max` flag that is the
/// comparison's, and a legacy multiply-add whose zero product plus a `-0`
/// accumulator is `+0`. 0x4c00_0004: 64-bit compares and `v_mad_u64_u32`
/// carries worked out dword by dword, fxceless 1506 and 1507. 0x4c00_0005,
/// one bump for the merge of the two lines: the scalar rules and fxceless
/// 1506/1507 together, and always-true/always-false float compares lowered
/// as constants — no cached translation is trusted across that. 0x4c00_0006:
/// the mirror's translator v2 (`avatarmanz` #469 and #405) — a helper lane's
/// `v_readfirstlane` reads its own value, `image_sample_*_g16` gradients
/// packed as 16-bit pairs, `annotateHandle` kept block-local, export target 16
/// as `pos4`, and a scalar written in a lane-divergent compute branch reaching
/// every lane after the join (`VICTORY_SCALAR_SPLITS`, on) — composed with
/// 0x4c00_0005's rules above, all of which stay. The operandless VOP1 decode
/// and the EXEC destination of VOP3 CMPX (#410) change translations too, and
/// ride this same bump.)
pub const TRANSLATOR_VERSION: u32 = 0x4c00_0006;

pub use dxbc::{Container, FourCC, Part};
pub use dxil::{DxilProgram, ShaderKind, XboxExtras};
pub use native::{
    HwStage, NativeProgram, ResourceType, RootParam, RootType, UsedResource, native_programs,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ShaderError {
    #[error("not a DXBC container")]
    NotDxbc,
    #[error("malformed shader: {0}")]
    Malformed(&'static str),
    #[error("GFX10 code at {offset:#x}: {reason}")]
    Isa { offset: usize, reason: &'static str },
    /// Valid code the translator cannot handle yet: an opcode name or a construct.
    #[error("GFX10 code at {offset:#x}: {what} is not supported")]
    Unsupported { offset: usize, what: &'static str },
}
