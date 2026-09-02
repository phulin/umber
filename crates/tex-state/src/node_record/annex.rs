use super::*;

static NEXT_ANNEX_OWNER: AtomicU32 = AtomicU32::new(1);

#[repr(C)]
pub(crate) struct AnnexKey<Kind> {
    pub(super) owner: u32,
    pub(super) block_ordinal: u32,
    pub(super) logical_block_incarnation: u32,
    pub(super) word_offset: u32,
    pub(super) word_len: u32,
    pub(super) publication_serial: u32,
    pub(super) kind: PhantomData<fn(&Kind) -> &Kind>,
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
    words: tex_dense_prefix::Superblock<u32>,
    logical_incarnation: u32,
}

impl AnnexBlock {
    fn new(logical_incarnation: u32) -> Self {
        Self {
            words: tex_dense_prefix::Superblock::try_new()
                .expect("u32 annex superblock layout is valid"),
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

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeAnnexMark {
    owner: u32,
    words: usize,
}

pub struct NodeAnnexArena {
    owner: u32,
    blocks: Vec<AnnexBlock>,
    words: usize,
    next_publication_serial: u32,
    next_logical_incarnation: u32,
    metrics: NodeAnnexMetrics,
}

pub(super) enum LigaturePayload {}
pub(super) enum LigatureSource {}
pub(super) enum BoxPayload {}
pub(super) enum LeaderBoxPayload {}
pub(super) enum UnsetPayload {}
pub(super) enum DiscPayload {}
pub(super) enum InsertionPayload {}
pub(super) enum MathNoadPayload {}
pub(super) enum FractionPayload {}
pub(super) enum MathChoicePayload {}
pub(super) enum ListPayload {}
pub(super) enum Utf8Span {}
pub(super) enum ByteSpan {}
pub(super) enum SpecialPayload {}
pub(super) enum DeferredSpecialPayload {}
pub(super) enum PdfDestinationPayload {}
pub(super) enum PdfThreadPayload {}

pub(super) fn key_words<Kind>(key: AnnexKey<Kind>) -> [u32; 6] {
    key.words()
}

pub(super) fn key_from_record<Kind>(record: NodeRecord) -> AnnexKey<Kind> {
    let words = record.words();
    AnnexKey::from_words([words[0], words[1], words[2], words[3], words[4], words[5]])
}

pub(super) fn encode_page_list(destination: &mut Vec<u32>, list: PageListId) {
    append_words(destination, list.words());
}

pub(super) fn decode_page_list(source: &[u32], cursor: &mut usize) -> Option<PageListId> {
    PageListId::from_words(take_words(source, cursor)?)
}

pub(super) fn encode_font(destination: &mut Vec<u32>, font: FontId) {
    append_words(destination, font.words());
}

pub(super) fn decode_font(source: &[u32], cursor: &mut usize) -> Option<FontId> {
    FontId::from_words(take_words(source, cursor)?)
}

pub(super) fn encode_box_payload(value: BoxNode<PageListId>) -> Vec<u32> {
    let mut words = Vec::with_capacity(28);
    append_words(
        &mut words,
        [
            scaled_word(value.width),
            scaled_word(value.height),
            scaled_word(value.depth),
            scaled_word(value.shift),
            value.glue_set.numerator() as u32,
            value.glue_set.denominator() as u32,
            value.box_lr as u32
                | ((value.glue_sign as u32) << 8)
                | ((value.glue_order as u32) << 16)
                | (bool_word(value.diagnostic_children.is_some()) << 24),
        ],
    );
    encode_page_list(&mut words, value.children);
    encode_page_list(
        &mut words,
        value.diagnostic_children.unwrap_or_else(PageListId::empty),
    );
    words.push(value.allocator_high_cell_overlap);
    debug_assert_eq!(words.len(), 28);
    words
}

pub(super) fn decode_box_payload(words: &[u32]) -> Option<BoxNode<PageListId>> {
    if words.len() != 28 {
        return None;
    }
    let mut cursor = 0;
    let scalar: [u32; 7] = take_words(words, &mut cursor)?;
    if scalar[5] == 0 || scalar[6] & 0xfe00_00f8 != 0 {
        return None;
    }
    let box_lr = match scalar[6] & 0xff {
        0 => BoxLr::Normal,
        1 => BoxLr::Reversed,
        2 => BoxLr::DList,
        _ => return None,
    };
    let glue_sign = match (scalar[6] >> 8) & 0xff {
        0 => Sign::Normal,
        1 => Sign::Stretching,
        2 => Sign::Shrinking,
        _ => return None,
    };
    let glue_order = decode_order((scalar[6] >> 16) & 0xff)?;
    let diagnostic_present = decode_bool((scalar[6] >> 24) & 1)?;
    let children = decode_page_list(words, &mut cursor)?;
    let diagnostic = decode_page_list(words, &mut cursor)?;
    let overlap = *words.get(cursor)?;
    if diagnostic_present == diagnostic.is_empty() {
        return None;
    }
    let mut value = BoxNode::new(BoxNodeFields {
        width: decode_scaled(scalar[0]),
        height: decode_scaled(scalar[1]),
        depth: decode_scaled(scalar[2]),
        shift: decode_scaled(scalar[3]),
        box_lr,
        glue_set: GlueSetRatio::try_from_ratio_parts(scalar[4] as i32, scalar[5] as i32).ok()?,
        glue_sign,
        glue_order,
        children,
    });
    value.diagnostic_children = diagnostic_present.then_some(diagnostic);
    value.allocator_high_cell_overlap = overlap;
    Some(value)
}

pub(super) fn encode_math_field(destination: &mut Vec<u32>, field: MathField<PageListId>) {
    match field {
        MathField::Empty => destination.extend([0; 11]),
        MathField::MathChar(value) | MathField::MathTextChar(value) => {
            let tag = if matches!(field, MathField::MathChar(_)) {
                1
            } else {
                2
            };
            destination.extend([
                tag,
                u32::from(value.family),
                value.character as u32,
                value.origin.raw(),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]);
        }
        MathField::SubBox(list) | MathField::SubMlist(list) => {
            let tag = if matches!(field, MathField::SubBox(_)) {
                3
            } else {
                4
            };
            destination.push(tag);
            encode_page_list(destination, list);
        }
    }
}

pub(super) fn decode_math_field(
    words: &[u32],
    cursor: &mut usize,
) -> Option<MathField<PageListId>> {
    let field: [u32; 11] = take_words(words, cursor)?;
    match field[0] {
        0 if field[1..].iter().all(|word| *word == 0) => Some(MathField::Empty),
        tag @ (1 | 2)
            if field[4..].iter().all(|word| *word == 0)
                && field[1] < crate::math::MATH_FAMILY_COUNT as u32 =>
        {
            let value = MathChar {
                family: field[1] as u8,
                character: char::from_u32(field[2])?,
                origin: OriginId::from_raw(field[3]),
            };
            Some(if tag == 1 {
                MathField::MathChar(value)
            } else {
                MathField::MathTextChar(value)
            })
        }
        tag @ (3 | 4) => {
            let list = PageListId::from_words(field[1..].try_into().ok()?)?;
            Some(if tag == 3 {
                MathField::SubBox(list)
            } else {
                MathField::SubMlist(list)
            })
        }
        _ => None,
    }
}

pub(super) fn encode_noad_kind(destination: &mut Vec<u32>, kind: NoadKind) {
    let words = match kind {
        NoadKind::Normal(class) => [class as u32, 0, 0],
        NoadKind::Operator(limit) => [1 << 8 | limit as u32, 0, 0],
        NoadKind::Radical { delimiter } => [2 << 8, delimiter, 0],
        NoadKind::Accent { accent } => [
            3 << 8 | u32::from(accent.family),
            accent.character as u32,
            accent.origin.raw(),
        ],
        NoadKind::LeftDelimiter { delimiter } => [4 << 8, delimiter, 0],
        NoadKind::RightDelimiter { delimiter } => [5 << 8, delimiter, 0],
        NoadKind::MiddleDelimiter { delimiter } => [6 << 8, delimiter, 0],
        NoadKind::Underline => [7 << 8, 0, 0],
        NoadKind::Overline => [8 << 8, 0, 0],
        NoadKind::VCenter => [9 << 8, 0, 0],
    };
    destination.extend(words);
}

pub(super) fn decode_noad_kind(words: &[u32], cursor: &mut usize) -> Option<NoadKind> {
    let [tagged, value, origin]: [u32; 3] = take_words(words, cursor)?;
    let tag = tagged >> 8;
    let low = tagged & 0xff;
    match (tag, low, value, origin) {
        (0, class, 0, 0) => Some(NoadKind::Normal(match class {
            0 => NoadClass::Ord,
            1 => NoadClass::Op,
            2 => NoadClass::Bin,
            3 => NoadClass::Rel,
            4 => NoadClass::Open,
            5 => NoadClass::Close,
            6 => NoadClass::Punct,
            7 => NoadClass::Inner,
            _ => return None,
        })),
        (1, limit, 0, 0) => Some(NoadKind::Operator(match limit {
            0 => LimitType::DisplayLimits,
            1 => LimitType::Limits,
            2 => LimitType::NoLimits,
            _ => return None,
        })),
        (2, 0, delimiter, 0) => Some(NoadKind::Radical { delimiter }),
        (3, family, character, origin) if family < crate::math::MATH_FAMILY_COUNT as u32 => {
            Some(NoadKind::Accent {
                accent: MathChar {
                    family: family as u8,
                    character: char::from_u32(character)?,
                    origin: OriginId::from_raw(origin),
                },
            })
        }
        (4, 0, delimiter, 0) => Some(NoadKind::LeftDelimiter { delimiter }),
        (5, 0, delimiter, 0) => Some(NoadKind::RightDelimiter { delimiter }),
        (6, 0, delimiter, 0) => Some(NoadKind::MiddleDelimiter { delimiter }),
        (7, 0, 0, 0) => Some(NoadKind::Underline),
        (8, 0, 0, 0) => Some(NoadKind::Overline),
        (9, 0, 0, 0) => Some(NoadKind::VCenter),
        _ => None,
    }
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

    #[cfg(test)]
    pub(crate) const fn mark(&self) -> NodeAnnexMark {
        NodeAnnexMark {
            owner: self.owner,
            words: self.words,
        }
    }

    #[cfg(test)]
    pub(crate) const fn metrics(&self) -> NodeAnnexMetrics {
        self.metrics
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
            if offset == block.words.len() {
                block
                    .words
                    .push_with(|slot| slot.insert(word))
                    .expect("annex superblock capacity");
            } else {
                *block.words.get_mut(offset).expect("initialized annex word") = word;
            }
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

    #[cfg(test)]
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
            let mut initialized = self.words % ANNEX_WORDS_PER_BLOCK;
            if initialized == 0 && self.words != 0 {
                initialized = ANNEX_WORDS_PER_BLOCK;
            }
            block.words.truncate(initialized);
        }
        self.metrics.words_rolled_back += removed as u64;
        true
    }

    #[cfg(test)]
    pub(crate) fn resolve_fixed<Kind>(&mut self, key: AnnexKey<Kind>) -> Option<&[u32]> {
        if key.owner != self.owner || key.word_len == 0 {
            self.metrics.stale_rejections += 1;
            return None;
        }
        let ordinal = key.block_ordinal as usize;
        let offset = key.word_offset as usize;
        let len = key.word_len as usize;
        let valid = self.blocks.get(ordinal).is_some_and(|block| {
            block.logical_incarnation == key.logical_block_incarnation
                && offset
                    .checked_add(len)
                    .is_some_and(|end| end <= block.words.len())
                && block.words.get(offset).copied() == Some(key.publication_serial)
        });
        if !valid {
            self.metrics.stale_rejections += 1;
            return None;
        }
        self.metrics.direct_lookups += 1;
        self.blocks[ordinal]
            .words
            .initialized()
            .get(offset + 1..offset + len)
    }

    pub(crate) fn resolve_fixed_shared<Kind>(&self, key: AnnexKey<Kind>) -> Option<&[u32]> {
        if key.owner != self.owner || key.word_len == 0 {
            return None;
        }
        let block = self.blocks.get(key.block_ordinal as usize)?;
        let offset = key.word_offset as usize;
        let len = key.word_len as usize;
        if block.logical_incarnation != key.logical_block_incarnation
            || offset.checked_add(len)? > block.words.len()
            || block.words.get(offset).copied()? != key.publication_serial
        {
            return None;
        }
        block.words.initialized().get(offset + 1..offset + len)
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

    pub(super) fn detach_span<Kind>(&self, key: AnnexKey<Kind>) -> Option<Vec<u32>> {
        let body_len = (key.word_len as usize).checked_sub(1)?;
        (0..body_len)
            .map(|index| self.resolve_word(key, index))
            .collect()
    }
}
