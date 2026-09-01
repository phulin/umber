//! Private compact resident node and typed word-annex substrate.

use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::fork_arena::PageMaterialLane;
use crate::node::NodeKind;

const ANNEX_WORDS_PER_BLOCK: usize = 16_384;
const HEADER_PRESENT: u32 = 1 << 31;
const KIND_MASK: u32 = 0x1f;
const SUBTYPE_SHIFT: u32 = 5;
const SUBTYPE_MASK: u32 = 0x1f << SUBTYPE_SHIFT;
const FLAGS_SHIFT: u32 = 10;
const FLAGS_MASK: u32 = !(KIND_MASK | SUBTYPE_MASK | HEADER_PRESENT);

static NEXT_ANNEX_OWNER: AtomicU32 = AtomicU32::new(1);

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NodeRecord<Lane = PageMaterialLane> {
    header: NonZeroU32,
    words: [u32; 7],
    lane: PhantomData<fn(&Lane) -> &Lane>,
}

impl<Lane> NodeRecord<Lane> {
    pub(crate) fn new(kind: NodeKind, subtype: u8, flags: u32, words: [u32; 7]) -> Self {
        assert!(subtype < 32, "compact node subtype fits five bits");
        assert_eq!(
            flags & !0x1f_ffff,
            0,
            "compact node flags fit twenty-one bits"
        );
        let header = HEADER_PRESENT
            | kind as u32
            | (u32::from(subtype) << SUBTYPE_SHIFT)
            | (flags << FLAGS_SHIFT);
        Self {
            header: NonZeroU32::new(header).expect("resident node header is nonzero"),
            words,
            lane: PhantomData,
        }
    }

    pub(crate) fn kind(self) -> Option<NodeKind> {
        NodeKind::ALL
            .get((self.header.get() & KIND_MASK) as usize)
            .copied()
    }

    pub(crate) fn subtype(self) -> u8 {
        ((self.header.get() & SUBTYPE_MASK) >> SUBTYPE_SHIFT) as u8
    }

    pub(crate) fn flags(self) -> u32 {
        (self.header.get() & FLAGS_MASK) >> FLAGS_SHIFT
    }

    pub(crate) const fn words(self) -> [u32; 7] {
        self.words
    }
}

impl<Lane> core::fmt::Debug for NodeRecord<Lane> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NodeRecord")
            .field("kind", &self.kind())
            .field("subtype", &self.subtype())
            .finish_non_exhaustive()
    }
}

impl<Lane> PartialEq for NodeRecord<Lane> {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header && self.words == other.words
    }
}

impl<Lane> Eq for NodeRecord<Lane> {}

const _: () = assert!(core::mem::size_of::<NodeRecord>() == 32);
const _: () = assert!(core::mem::align_of::<NodeRecord>() == 4);
const _: () = assert!(!core::mem::needs_drop::<NodeRecord>());
const _: () = assert!(core::mem::size_of::<Option<NodeRecord>>() == 32);

#[repr(C)]
pub(crate) struct AnnexKey<Kind> {
    owner: u32,
    block_ordinal: u32,
    logical_block_incarnation: u32,
    word_offset: u32,
    word_len: u32,
    publication_serial: u32,
    kind: PhantomData<fn(&Kind) -> &Kind>,
}

impl<Kind> Clone for AnnexKey<Kind> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Kind> Copy for AnnexKey<Kind> {}

impl<Kind> core::fmt::Debug for AnnexKey<Kind> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AnnexKey")
            .field("word_len", &self.word_len)
            .finish_non_exhaustive()
    }
}

impl<Kind> AnnexKey<Kind> {
    pub(crate) const fn words(self) -> [u32; 6] {
        [
            self.owner,
            self.block_ordinal,
            self.logical_block_incarnation,
            self.word_offset,
            self.word_len,
            self.publication_serial,
        ]
    }

    pub(crate) const fn from_words(words: [u32; 6]) -> Self {
        Self {
            owner: words[0],
            block_ordinal: words[1],
            logical_block_incarnation: words[2],
            word_offset: words[3],
            word_len: words[4],
            publication_serial: words[5],
            kind: PhantomData,
        }
    }
}

const _: () = assert!(core::mem::size_of::<AnnexKey<()>>() == 24);
const _: () = assert!(core::mem::align_of::<AnnexKey<()>>() == 4);
const _: () = assert!(!core::mem::needs_drop::<AnnexKey<()>>());

struct AnnexBlock {
    words: Box<[u32]>,
    initialized: usize,
    logical_incarnation: u32,
}

impl AnnexBlock {
    fn new(logical_incarnation: u32) -> Self {
        Self {
            words: vec![0; ANNEX_WORDS_PER_BLOCK].into_boxed_slice(),
            initialized: 0,
            logical_incarnation,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NodeAnnexMetrics {
    pub(crate) superblocks_allocated: u64,
    pub(crate) superblocks_reclaimed: u64,
    pub(crate) words_published: u64,
    pub(crate) words_rolled_back: u64,
    pub(crate) boundary_padding_words: u64,
    pub(crate) direct_lookups: u64,
    pub(crate) stale_rejections: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeAnnexMark {
    owner: u32,
    words: usize,
}

pub(crate) struct NodeAnnexArena {
    owner: u32,
    blocks: Vec<AnnexBlock>,
    words: usize,
    next_publication_serial: u32,
    next_logical_incarnation: u32,
    metrics: NodeAnnexMetrics,
}

impl Default for NodeAnnexArena {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeAnnexArena {
    pub(crate) fn new() -> Self {
        Self {
            owner: NEXT_ANNEX_OWNER
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .expect("node annex owner domain exhausted"),
            blocks: Vec::new(),
            words: 0,
            next_publication_serial: 1,
            next_logical_incarnation: 1,
            metrics: NodeAnnexMetrics::default(),
        }
    }

    pub(crate) const fn mark(&self) -> NodeAnnexMark {
        NodeAnnexMark {
            owner: self.owner,
            words: self.words,
        }
    }

    pub(crate) const fn metrics(&self) -> NodeAnnexMetrics {
        self.metrics
    }

    pub(crate) const fn len(&self) -> usize {
        self.words
    }

    fn ensure_block(&mut self, ordinal: usize) {
        while self.blocks.len() <= ordinal {
            let incarnation = self.next_logical_incarnation;
            self.next_logical_incarnation = incarnation
                .checked_add(1)
                .expect("node annex logical incarnation exhausted");
            self.blocks.push(AnnexBlock::new(incarnation));
            self.metrics.superblocks_allocated += 1;
        }
    }

    fn next_serial(&mut self) -> u32 {
        let serial = self.next_publication_serial;
        self.next_publication_serial = serial
            .checked_add(1)
            .expect("node annex publication serial exhausted");
        serial
    }

    pub(crate) fn append_fixed<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        let word_len = body
            .len()
            .checked_add(1)
            .expect("node annex fixed record length overflow");
        assert!(
            word_len <= 41,
            "fixed node annex record exceeds design maximum"
        );
        let tail = self.words % ANNEX_WORDS_PER_BLOCK;
        if tail != 0 && tail + word_len > ANNEX_WORDS_PER_BLOCK {
            let padding = ANNEX_WORDS_PER_BLOCK - tail;
            self.words += padding;
            self.metrics.boundary_padding_words += padding as u64;
        }
        self.append_contiguous(body)
    }

    pub(crate) fn append_span<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        self.append_contiguous(body)
    }

    fn append_contiguous<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        let serial = self.next_serial();
        let start = self.words;
        let total = body
            .len()
            .checked_add(1)
            .expect("node annex span length overflow");
        for (index, word) in core::iter::once(serial)
            .chain(body.iter().copied())
            .enumerate()
        {
            let position = start + index;
            let ordinal = position / ANNEX_WORDS_PER_BLOCK;
            let offset = position % ANNEX_WORDS_PER_BLOCK;
            self.ensure_block(ordinal);
            let block = &mut self.blocks[ordinal];
            block.words[offset] = word;
            block.initialized = block.initialized.max(offset + 1);
        }
        self.words = start
            .checked_add(total)
            .expect("node annex logical length overflow");
        self.metrics.words_published += total as u64;
        let ordinal = start / ANNEX_WORDS_PER_BLOCK;
        AnnexKey {
            owner: self.owner,
            block_ordinal: u32::try_from(ordinal).expect("node annex block ordinal overflow"),
            logical_block_incarnation: self.blocks[ordinal].logical_incarnation,
            word_offset: u32::try_from(start % ANNEX_WORDS_PER_BLOCK)
                .expect("node annex offset overflow"),
            word_len: u32::try_from(total).expect("node annex record length overflow"),
            publication_serial: serial,
            kind: PhantomData,
        }
    }

    pub(crate) fn rollback(&mut self, mark: NodeAnnexMark) -> bool {
        if mark.owner != self.owner || mark.words > self.words {
            return false;
        }
        let removed = self.words - mark.words;
        self.words = mark.words;
        let retained_blocks = self.words.div_ceil(ANNEX_WORDS_PER_BLOCK);
        while self.blocks.len() > retained_blocks {
            self.blocks.pop();
            self.metrics.superblocks_reclaimed += 1;
        }
        if let Some(block) = self.blocks.last_mut() {
            block.initialized = self.words % ANNEX_WORDS_PER_BLOCK;
            if block.initialized == 0 && self.words != 0 {
                block.initialized = ANNEX_WORDS_PER_BLOCK;
            }
        }
        self.metrics.words_rolled_back += removed as u64;
        true
    }

    pub(crate) fn resolve_fixed<Kind>(&mut self, key: AnnexKey<Kind>) -> Option<&[u32]> {
        let reject = |arena: &mut Self| {
            arena.metrics.stale_rejections += 1;
            None
        };
        if key.owner != self.owner || key.word_len == 0 {
            return reject(self);
        }
        let ordinal = key.block_ordinal as usize;
        let offset = key.word_offset as usize;
        let len = key.word_len as usize;
        let Some(block) = self.blocks.get(ordinal) else {
            return reject(self);
        };
        if block.logical_incarnation != key.logical_block_incarnation
            || offset
                .checked_add(len)
                .is_none_or(|end| end > block.initialized)
            || block.words.get(offset).copied() != Some(key.publication_serial)
        {
            return reject(self);
        }
        self.metrics.direct_lookups += 1;
        block.words.get(offset + 1..offset + len)
    }

    pub(crate) fn resolve_word<Kind>(&self, key: AnnexKey<Kind>, index: usize) -> Option<u32> {
        let len = key.word_len as usize;
        if key.owner != self.owner || index + 1 >= len {
            return None;
        }
        let absolute = (key.block_ordinal as usize)
            .checked_mul(ANNEX_WORDS_PER_BLOCK)?
            .checked_add(key.word_offset as usize)?;
        let serial_ordinal = absolute / ANNEX_WORDS_PER_BLOCK;
        let serial_offset = absolute % ANNEX_WORDS_PER_BLOCK;
        let serial_block = self.blocks.get(serial_ordinal)?;
        if serial_block.logical_incarnation != key.logical_block_incarnation
            || serial_block.words.get(serial_offset).copied() != Some(key.publication_serial)
        {
            return None;
        }
        let position = absolute.checked_add(index + 1)?;
        if position >= self.words {
            return None;
        }
        self.blocks
            .get(position / ANNEX_WORDS_PER_BLOCK)?
            .words
            .get(position % ANNEX_WORDS_PER_BLOCK)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum Fixed {}

    #[test]
    fn exact_record_and_key_layouts_are_copy_only() {
        assert_eq!(core::mem::size_of::<NodeRecord>(), 32);
        assert_eq!(core::mem::align_of::<NodeRecord>(), 4);
        assert!(!core::mem::needs_drop::<NodeRecord>());
        assert_eq!(core::mem::size_of::<Option<NodeRecord>>(), 32);
        assert_eq!(core::mem::size_of::<AnnexKey<Fixed>>(), 24);
        assert_eq!(core::mem::align_of::<AnnexKey<Fixed>>(), 4);
        assert!(!core::mem::needs_drop::<AnnexKey<Fixed>>());
    }

    #[test]
    fn rollback_reuse_rejects_old_publication_serial() {
        let mut arena = NodeAnnexArena::new();
        let mark = arena.mark();
        let stale = arena.append_fixed::<Fixed>(&[7, 8]);
        assert_eq!(arena.resolve_fixed(stale), Some([7, 8].as_slice()));
        assert!(arena.rollback(mark));
        let current = arena.append_fixed::<Fixed>(&[9, 10]);
        assert!(arena.resolve_fixed(stale).is_none());
        assert_eq!(arena.resolve_fixed(current), Some([9, 10].as_slice()));
    }

    #[test]
    fn fixed_records_pad_instead_of_crossing_a_superblock() {
        let mut arena = NodeAnnexArena::new();
        let body = vec![0; ANNEX_WORDS_PER_BLOCK - 2];
        let _ = arena.append_span::<()>(&body);
        let fixed = arena.append_fixed::<Fixed>(&[1, 2, 3]);
        assert_eq!(fixed.word_offset, 0);
        assert_eq!(arena.metrics().boundary_padding_words, 1);
        assert_eq!(arena.resolve_fixed(fixed), Some([1, 2, 3].as_slice()));
    }
}
