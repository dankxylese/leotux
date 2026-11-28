//! What goes up to copies in the card's memory goes up as it was when its
//! submission was prepared (`VICTORY_SEND_OWN`, on by default; `0`: as
//! before).
//!
//! The sends were copies out of the copies' CPU sides, which the card makes
//! when it comes to the submission. By then later submissions are prepared
//! (two frames of them with `VICTORY_RELAY`), and their refreshes have
//! written the same CPU sides: the earlier submission's shaders then read a
//! later one's bytes. For what the title's CPU writes that is as on the
//! console (it does not write what the card still reads). But the title's
//! own buffer copies are commands in its lists, and those are done by us
//! when a submission is prepared: "a value to a buffer, a draw that reads
//! it" in one submission and the same with another value in the next is
//! ordinary, and both draws read the second value.
//!
//! So each submission's bytes are taken out of the CPU sides into memory of
//! the submission's own when they are taken from the dirty lists, and the
//! card copies them from there. Runs of a few words, which a refresh that
//! sends only what changed makes many of, go as pairs of offset and value,
//! stored by one dispatch per copy; the others as a copy each.

use super::*;
use crate::shadow::Shadow;

pub(super) fn own() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SEND_OWN").is_none_or(|v| v != "0"))
}

/// Runs up to this many bytes go as pairs.
const SHORT: usize = 256;

/// Whether a run of `len` bytes at offset `at` of a copy goes as pairs.
fn short(at: usize, len: usize) -> bool {
    len <= SHORT && at.is_multiple_of(4) && len.is_multiple_of(4)
}

/// Whether a run of `len` bytes at offset `at` of a copy in the card's memory
/// is no copy command of its own, but pairs of one dispatch.
pub(crate) fn goes_as_words(at: usize, len: usize) -> bool {
    own() && short(at, len)
}

/// Whether the title's bytes that go in under flights' writes
/// (`Shadow::merge_under_writes`) go in by whole words
/// (`VICTORY_UNDER_WORDS=0`: by the bytes that changed, as before).
///
/// By bytes nearly every run (a float's three low bytes) was a copy command
/// of its own, and a submission with more than 4,096 of them stood until the
/// flights had landed: 4 to 14 ms of a steady frame (session play-1125-0203:
/// "more changed runs than a submission sends up"), for some 20 KB. Whole
/// words are pairs of one dispatch, and are not counted.
///
/// A word is taken to be the title's or the shaders', not shared: where the
/// title changed a byte of a word of which a flight's shader writes another
/// byte, that byte is as it was before the flight. The landing rests on the
/// same ([`crate::shadow::exact_back`]).
pub(crate) fn words_under() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| own() && std::env::var_os("VICTORY_UNDER_WORDS").is_none_or(|v| v != "0"))
}

/// The scatter: `pairs` holds `(offset in the copy, word)` one after the other,
/// and each thread stores one word at its offset in `card`.
///
/// Bounds: the pairs of a dispatch are whole and inside `pairs` (`at` and
/// `count` come from [`take`]'s packing of complete runs, and the dispatch is
/// cut at what a dispatch can hold), and a pair's offset is checked against the
/// copy's own `size` here as well. A wrong offset writes into another buffer,
/// so the shader does not take the packing's word for it. The guards are one
/// statement each: HLSL does not promise that `||` short-circuits, and
/// `size - p.x` underflows into a bound that passes where it is not evaluated
/// alone. A word's store is aligned at a word's offset, and a copy is whole
/// pages (so `size >= 4` and `size - 4` exists).
const SCATTER: &str = r#"
#define RS "RootConstants(num32BitConstants=3, b0), SRV(t0), UAV(u0)"
cbuffer Part : register(b0) { uint at; uint count; uint size; };
ByteAddressBuffer pairs : register(t0);
RWByteAddressBuffer card : register(u0);
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= count) return;
    uint2 p = pairs.Load2(at + id.x * 8u);
    if (p.x >= size) return;
    if (size - p.x < 4u) return;
    if ((p.x & 3u) != 0u) return;
    card.Store(p.x, p.y);
}
"#;

struct Pass {
    root: ID3D12RootSignature,
    pso: ID3D12PipelineState,
}

fn pass(host: &Host) -> Option<&'static Pass> {
    static PASS: OnceLock<Option<Pass>> = OnceLock::new();
    PASS.get_or_init(|| {
        let code = compile(SCATTER, windows::core::s!("main"), windows::core::s!("cs_5_1")).ok()?;
        // SAFETY: the code lives through the calls. The blob's own root
        // signature first: only where the host refuses it (Wine, E_FAIL) is
        // the description read from the source's `RS` and serialized here
        // used, as the title's own signatures are.
        unsafe {
            let root: ID3D12RootSignature = crate::rootstring::make(&host.device, SCATTER, &code).ok()?;
            let pso: ID3D12PipelineState = host
                .device
                .CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
                    pRootSignature: std::mem::transmute_copy(&root),
                    CS: D3D12_SHADER_BYTECODE {
                        pShaderBytecode: code.as_ptr().cast(),
                        BytecodeLength: code.len(),
                    },
                    ..Default::default()
                })
                .ok()?;
            name(&pso, || "victory: changed words sent to the card".into());
            Some(Pass { root, pso })
        }
    })
    .as_ref()
}

/// Memory a submission's sends come from, mapped.
pub(super) struct Memory {
    buffer: ID3D12Resource,
    data: usize,
    size: u64,
}

// SAFETY: a D3D12 object (free-threaded) and its mapping, with one owner.
unsafe impl Send for Memory {}

/// What submissions that have run gave back.
static SPARE: Mutex<Vec<Memory>> = Mutex::new(Vec::new());

/// At most this much is kept (of the card's memory, with [`on_card`]).
fn kept() -> u64 {
    if on_card() { 256 << 20 } else { 768 << 20 }
}

/// Whether the memory lies in the card's memory, written by the CPU through
/// the bus (`VICTORY_SEND_BAR=1`): the copies on the card's drawing queue, which
/// nothing overlaps, then cross no bus. In a captured frame they took 5.4 ms of
/// the card's 30 (342 copies), and take 0.4 this way; but filling it takes the
/// submitting thread 8-9 ms a frame instead of 3 (73-93 MB, run card-3 against
/// card-1), so it is a loss while the card waits for that thread: off by
/// default.
fn on_card() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SEND_BAR").is_some_and(|v| v == "1"))
}

fn memory(host: &Host, size: u64) -> Option<Memory> {
    {
        let mut spare = SPARE.lock().unwrap_or_else(|e| e.into_inner());
        let fit = spare.iter().enumerate().filter(|(_, m)| m.size >= size).min_by_key(|(_, m)| m.size).map(|(i, _)| i);
        if let Some(i) = fit {
            return Some(spare.swap_remove(i));
        }
    }
    // By doubles, so that the next of about this size finds it: what
    // submissions send differs from frame to frame.
    let size = size.max(8 << 20).next_power_of_two();
    let mapped = if on_card() { host.mapped_card_buffer(size, false) } else { None };
    let (buffer, data) = match mapped {
        Some(made) => made,
        None => host.upload_buffer(size)?,
    };
    name(&buffer, || "bytes a submission sends up".into());
    Some(Memory {
        buffer,
        data: data as usize,
        size,
    })
}

/// The submission has run.
pub(super) fn give_back(memory: Memory) {
    let mut spare = SPARE.lock().unwrap_or_else(|e| e.into_inner());
    spare.push(memory);
    spare.sort_by_key(|m| std::cmp::Reverse(m.size));
    while spare.iter().map(|m| m.size).sum::<u64>() > kept() {
        spare.remove(0);
    }
}

/// Since the last frame-rate line: bytes copied, runs copied, words stored,
/// runs stored as pairs, dispatches that stored them, microseconds of taking
/// them out of the CPU sides.
pub(super) static STATS: [std::sync::atomic::AtomicU64; 6] = [const { std::sync::atomic::AtomicU64::new(0) }; 6];

/// One copy's sends: ranges copied (offset in the copy, in the memory,
/// bytes), and its pairs (where in the memory, how many).
struct Copy {
    runs: Vec<(usize, u64, usize)>,
    pairs: (u64, u32),
}

/// Where a submission's bytes are taken to.
pub(super) struct Own {
    memory: Memory,
    copies: Vec<Copy>,
    /// Bytes of memory used.
    used: u64,
}

/// Runs whose pairs are one piece of work.
const RUNS_AT_A_TIME: usize = 4096;

/// Takes what `dirty` sends up out of the CPU sides.
pub(super) fn take(host: &Host, dirty: &[(std::sync::Arc<Shadow>, Vec<(usize, usize)>)]) -> Option<Own> {
    let began = std::time::Instant::now();
    pass(host)?;
    // Runs of whole words of the copy: only those are pairs. (A byte beside
    // them in a word may be the card's.)
    let short = |&(a, b): &(usize, usize)| short(a, b - a);
    let mut at = 0u64;
    let mut copies = Vec::with_capacity(dirty.len());
    let mut pair_runs = 0u64;
    for (_, ranges) in dirty {
        let mut runs = Vec::new();
        for &(a, b) in ranges.iter().filter(|r| !short(r)) {
            runs.push((a, at, b - a));
            at += ((b - a) as u64).next_multiple_of(16);
        }
        let words: usize = ranges.iter().filter(|r| short(r)).map(|r| (r.1 - r.0) / 4).sum();
        pair_runs += ranges.iter().filter(|r| short(r)).count() as u64;
        copies.push(Copy { runs, pairs: (at, words as u32) });
        at += (words as u64 * 8).next_multiple_of(16);
    }
    if at == 0 {
        return None;
    }
    let memory = memory(host, at)?;
    let mut moves: Vec<(usize, usize, usize)> = Vec::new();
    let mut work: Vec<(usize, &[(usize, usize)], usize)> = Vec::new();
    for ((shadow, ranges), copy) in dirty.iter().zip(&copies) {
        let from = shadow.card_data() as usize;
        moves.extend(copy.runs.iter().map(|&(a, to, n)| (from + a, memory.data + to as usize, n)));
        // (80,000 runs of some words a frame: in pieces of work.)
        let mut to = memory.data + copy.pairs.0 as usize;
        for ranges in ranges.chunks(RUNS_AT_A_TIME) {
            let words: usize = ranges.iter().filter(|r| short(r)).map(|r| (r.1 - r.0) / 4).sum();
            if words > 0 {
                work.push((from, ranges, to));
                to += words * 8;
            }
        }
    }
    let one = |i: usize| {
        let (from, ranges, to) = work[i];
        let mut to = to as *mut [u32; 2];
        for &(a, b) in ranges.iter().filter(|r| short(r)) {
            for offset in (a..b).step_by(4) {
                // SAFETY: inside the copy's mapping (its CPU side, which
                // the thread that asked, and waits, alone writes in these
                // ranges) and inside the memory, which has room for every
                // pair, a piece of work's behind the last one's.
                unsafe {
                    let word = ((from + offset) as *const u32).read_unaligned();
                    to.write([offset as u32, word]);
                    to = to.add(1);
                }
            }
        }
    };
    crate::land::each_under(work.len(), &one);
    STATS[0].fetch_add(moves.iter().map(|m| m.2 as u64).sum(), Ordering::Relaxed);
    STATS[1].fetch_add(moves.len() as u64, Ordering::Relaxed);
    STATS[2].fetch_add(copies.iter().map(|c| u64::from(c.pairs.1)).sum(), Ordering::Relaxed);
    STATS[4].fetch_add(pair_runs, Ordering::Relaxed);
    crate::shadow::copy_all(moves);
    STATS[3].fetch_add(began.elapsed().as_micros() as u64, Ordering::Relaxed);
    Some(Own { memory, copies, used: at })
}

impl Own {
    /// Records the sends into `list`; the copies that `looked` says are in
    /// `COPY_DEST`, the others in the common state. Those with a "before" are
    /// left readable by compute shaders.
    ///
    /// # Safety
    /// An open list of ours, whose compute state is set anew behind this.
    pub(super) unsafe fn record(
        &self,
        host: &Host,
        list: &ID3D12GraphicsCommandList,
        dirty: &[(std::sync::Arc<Shadow>, Vec<(usize, usize)>)],
        looked: impl Fn(&Shadow) -> bool,
        from: Option<&ID3D12Resource>,
    ) {
        // (With `crate::host::upq`: a copy of the memory in the card's.)
        let source = from.unwrap_or(&self.memory.buffer);
        let read = D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE;
        let write = D3D12_RESOURCE_STATE_UNORDERED_ACCESS;
        let dest = D3D12_RESOURCE_STATE_COPY_DEST;
        let (mut first, mut last) = (Vec::new(), Vec::new());
        for ((shadow, _), copy) in dirty.iter().zip(&self.copies) {
            for &(a, from, n) in &copy.runs {
                // SAFETY: the caller's; live buffers, ranges inside them.
                unsafe { list.CopyBufferRegion(&shadow.buffer, a as u64, source, from, n as u64) };
            }
            let mut state = (looked(shadow) || !copy.runs.is_empty()).then_some(dest);
            if copy.pairs.1 > 0 {
                if let Some(was) = state {
                    first.push(transition(&shadow.buffer, was, write));
                }
                state = Some(write);
            }
            let hashed = shadow.card_sides().is_some_and(|(_, before, _)| before.is_some());
            if let Some(was) = state
                && hashed
                && (!copy.runs.is_empty() || copy.pairs.1 > 0)
            {
                last.push(transition(&shadow.buffer, was, read));
            }
        }
        // SAFETY: the caller's; live objects.
        let mut dispatches = 0u64;
        unsafe {
            if !first.is_empty() {
                list.ResourceBarrier(&first);
            }
            if let Some(pass) = pass(host)
                && self.copies.iter().any(|c| c.pairs.1 > 0)
            {
                list.SetComputeRootSignature(&pass.root);
                list.SetPipelineState(&pass.pso);
                list.SetComputeRootShaderResourceView(1, source.GetGPUVirtualAddress());
                for ((shadow, _), copy) in dirty.iter().zip(&self.copies).filter(|(_, c)| c.pairs.1 > 0) {
                    list.SetComputeRootUnorderedAccessView(2, shadow.buffer.GetGPUVirtualAddress());
                    // What a pair's offset is checked against in the shader:
                    // the copy's own bytes, past which a wrong offset would
                    // write into another buffer's.
                    let size = u32::try_from(shadow.width()).unwrap_or(u32::MAX);
                    const MOST: u32 = 64 * D3D12_CS_DISPATCH_MAX_THREAD_GROUPS_PER_DIMENSION;
                    let (at, words) = copy.pairs;
                    let mut done = 0;
                    while done < words {
                        let n = (words - done).min(MOST);
                        let constants = [at as u32 + done * 8, n, size];
                        list.SetComputeRoot32BitConstants(0, 3, constants.as_ptr().cast(), 0);
                        list.Dispatch(n.div_ceil(64), 1, 1);
                        dispatches += 1;
                        done += n;
                    }
                }
            }
            if !last.is_empty() {
                list.ResourceBarrier(&last);
            }
        }
        STATS[5].fetch_add(dispatches, Ordering::Relaxed);
    }

    /// The memory and the bytes of it used.
    pub(super) fn source(&self) -> (&ID3D12Resource, u64) {
        (&self.memory.buffer, self.used)
    }

    /// The memory, to be kept until the submission has run.
    pub(super) fn memory(self) -> Memory {
        self.memory
    }
}

#[cfg(test)]
mod tests {
    use super::{SHORT, short};

    /// What the shader stores for a pair at `offset` of a copy of `size` bytes:
    /// its three guards, one statement each, as the HLSL has them.
    fn stores(offset: u32, size: u32) -> bool {
        if offset >= size {
            return false;
        }
        if size - offset < 4 {
            return false;
        }
        offset % 4 == 0
    }

    #[test]
    fn short_runs_are_whole_words_of_at_most_short() {
        assert!(short(0, 4), "a word");
        assert!(short(2 * 65536, SHORT), "a full short run");
        assert!(!short(0, SHORT + 4), "longer than short");
        assert!(!short(2, 4), "an offset off the words");
        assert!(!short(0, 6), "part of a word");
        // Nothing to send: no words, so no dispatch and no copy.
        assert!(short(0, 0));
        // Every run `short` sends is inside the copy at whole words.
        for (at, len) in [(0usize, 4usize), (65536 + 4, 12), (131072, SHORT)] {
            assert!(short(at, len));
            assert!(at % 4 == 0 && len % 4 == 0 && len >= 4);
        }
    }

    /// The packing and the scatter, end to end on the CPU: `take` writes one
    /// `(offset, word)` pair a word of the run into the submission's memory,
    /// and the shader reads them the way `Load2` does and stores each at its
    /// offset. What lands is the source's word, nothing beside the runs moves,
    /// and the last pair a dispatch reads is the last one packed.
    #[test]
    fn pairs_store_each_word_of_a_run_and_nothing_beside_it() {
        const PAGE: usize = 65536;
        let size = 4 * PAGE;
        let source: Vec<u8> = (0..size)
            .map(|i| (i.wrapping_mul(13) & 0xFF) as u8)
            .collect();
        let runs = [
            (0usize, 4usize),
            (PAGE + 4, PAGE + 16),
            (2 * PAGE, 2 * PAGE + SHORT),
        ];
        let mut words = 0usize;
        for &(a, b) in &runs {
            assert!(short(a, b - a), "run {a}..{b} ({} bytes)", b - a);
            words += (b - a) / 4;
        }

        // As `take` packs a copy's runs: offset and word, back to back, from
        // the copy's own bytes.
        let mut pairs: Vec<u8> = vec![0; words * 8];
        let mut at = 0usize;
        for &(a, b) in &runs {
            for offset in (a..b).step_by(4) {
                let word = u32::from_le_bytes(source[offset..offset + 4].try_into().unwrap());
                pairs[at..at + 4].copy_from_slice(&(offset as u32).to_le_bytes());
                pairs[at + 4..at + 8].copy_from_slice(&word.to_le_bytes());
                at += 8;
            }
        }
        assert_eq!(at, words * 8, "the packing ends where the words do");

        // The shader, over those bytes: thread `i` takes `pairs.Load2(at + i*8)`
        // for `count` threads, `at` the copy's byte offset into the memory.
        let mut card = vec![0xEEu8; size];
        let (pairs_at, count, card_size) = (0usize, words as u32, size as u32);
        for i in 0..count as usize {
            let lane = pairs_at + i * 8;
            assert!(
                lane + 8 <= pairs.len(),
                "a dispatch reads only what was packed"
            );
            let offset = u32::from_le_bytes(pairs[lane..lane + 4].try_into().unwrap());
            let word = u32::from_le_bytes(pairs[lane + 4..lane + 8].try_into().unwrap());
            if !stores(offset, card_size) {
                continue;
            }
            let offset = offset as usize;
            card[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }

        for &(a, b) in &runs {
            assert_eq!(&card[a..b], &source[a..b], "the run's words, word for word");
        }
        let mut beside = card.clone();
        for &(a, b) in &runs {
            beside[a..b].fill(0xEE);
        }
        assert!(
            beside.iter().all(|&b| b == 0xEE),
            "nothing beside the runs moved"
        );

        // A pair off the copy (a wrong offset) is refused, not stored past it:
        // the guards a shader has where the packing was wrong.
        assert!(!stores(card_size, card_size));
        assert!(!stores(card_size - 3, card_size), "a word over the end");
        assert!(!stores(2, card_size), "an offset off the words");
        assert!(stores(card_size - 4, card_size), "the copy's last word");
    }
}
