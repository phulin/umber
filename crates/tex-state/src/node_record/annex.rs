use super::*;

use crate::fork_arena::{ArenaListId, ChunkPool, ForkArena};
use crate::node_region::NodeAnnexLane;

#[repr(C)]
pub(crate) struct AnnexKey<Kind> {
    words: [u32; 7],
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
            .field("word_len", &self.words[5])
            .finish_non_exhaustive()
    }
}

impl<Kind> AnnexKey<Kind> {
    pub(crate) const fn words(self) -> [u32; 7] {
        self.words
    }

    pub(crate) const fn from_words(words: [u32; 7]) -> Self {
        Self {
            words,
            kind: PhantomData,
        }
    }

    fn from_list(list: ArenaListId<NodeAnnexLane>, publication_serial: u32) -> Self {
        let words = list.words();
        Self::from_words([
            words[1],
            words[2],
            words[3],
            words[4],
            words[5],
            words[7],
            publication_serial,
        ])
    }

    fn list(self, space: u32, chunk_capacity: usize) -> Option<ArenaListId<NodeAnnexLane>> {
        let chunk_capacity = u32::try_from(chunk_capacity).ok()?;
        let head_offset = self.words[2];
        let len = self.words[5];
        let end = head_offset.checked_add(len)?;
        let tail_offset = if self.words[0] == self.words[3] && self.words[1] == self.words[4] {
            end
        } else {
            match end % chunk_capacity {
                0 => chunk_capacity,
                offset => offset,
            }
        };
        ArenaListId::from_words([
            space,
            self.words[0],
            self.words[1],
            self.words[2],
            self.words[3],
            self.words[4],
            tail_offset,
            len,
        ])
    }
}

const _: () = assert!(core::mem::size_of::<AnnexKey<()>>() == 28);
const _: () = assert!(core::mem::align_of::<AnnexKey<()>>() == 4);
const _: () = assert!(!core::mem::needs_drop::<AnnexKey<()>>());

pub struct NodeAnnexWriter<'a> {
    pool: &'a mut ChunkPool<u32>,
    arena: &'a mut ForkArena<u32, NodeAnnexLane>,
    dependency_floor: usize,
}

pub(super) enum NodeAnnexCopySource<'a> {
    SameRegion,
    OtherRegion(&'a ForkArena<u32, NodeAnnexLane>),
}

pub(super) struct NodeAnnexCopier<'a> {
    pool: &'a mut ChunkPool<u32>,
    source: NodeAnnexCopySource<'a>,
    destination: &'a mut ForkArena<u32, NodeAnnexLane>,
    dependency_floor: usize,
}

#[derive(Clone, Copy)]
pub struct NodeAnnexView<'a> {
    pool: &'a ChunkPool<u32>,
    arena: &'a ForkArena<u32, NodeAnnexLane>,
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
pub(super) enum OpenOutPayload {}
pub(super) enum SpecialPayload {}
pub(super) enum DeferredSpecialPayload {}
pub(super) enum PdfDestinationPayload {}
pub(super) enum PdfThreadPayload {}
pub(super) enum PdfColorStackPayload {}

pub(super) fn key_words<Kind>(key: AnnexKey<Kind>) -> [u32; 7] {
    key.words()
}

pub(super) fn key_from_record<Kind>(record: NodeRecord) -> AnnexKey<Kind> {
    let words = record.words();
    AnnexKey::from_words(words)
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

impl<'a> NodeAnnexWriter<'a> {
    pub(crate) fn new(
        pool: &'a mut ChunkPool<u32>,
        arena: &'a mut ForkArena<u32, NodeAnnexLane>,
    ) -> Self {
        Self {
            pool,
            arena,
            dependency_floor: usize::MAX,
        }
    }

    pub(crate) fn view(&self) -> NodeAnnexView<'_> {
        NodeAnnexView::new(self.pool, self.arena)
    }

    pub(crate) fn dependency_floor(&self) -> Option<usize> {
        (self.dependency_floor != usize::MAX).then_some(self.dependency_floor)
    }

    pub(crate) fn append_fixed<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        assert!(
            body.len() <= 40,
            "fixed node annex record exceeds design maximum"
        );
        let publication_serial = self.pool.next_publication_serial();
        let list = self
            .arena
            .append_unsealed_fixed_copy_parts(self.pool, publication_serial, body)
            .expect("fixed typed annex publication fits one paired-region chunk");
        let position = self
            .arena
            .owner_relative_head_position(self.pool, list)
            .expect("new fixed annex record belongs to its paired region");
        self.dependency_floor = self.dependency_floor.min(position);
        AnnexKey::from_list(list, publication_serial)
    }

    pub(crate) fn append_span<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        let publication_serial = self.pool.next_publication_serial();
        let list = self
            .arena
            .append_unsealed_copy_parts(self.pool, publication_serial, body)
            .expect("typed annex publication fits its paired region");
        let position = self
            .arena
            .owner_relative_head_position(self.pool, list)
            .expect("new annex record belongs to its paired region");
        self.dependency_floor = self.dependency_floor.min(position);
        AnnexKey::from_list(list, publication_serial)
    }
}

impl<'a> NodeAnnexView<'a> {
    pub(crate) const fn new(
        pool: &'a ChunkPool<u32>,
        arena: &'a ForkArena<u32, NodeAnnexLane>,
    ) -> Self {
        Self { pool, arena }
    }

    fn list<Kind>(
        self,
        key: AnnexKey<Kind>,
    ) -> Option<crate::fork_arena::ArenaListView<'a, u32, NodeAnnexLane>> {
        let list = key.list(self.pool.logical_space(), self.pool.chunk_capacity())?;
        let view = self.arena.list(self.pool, list).ok()?;
        let actual = *view.get(0)?;
        if actual != key.words[6] {
            return None;
        }
        Some(view)
    }

    pub(crate) fn resolve_fixed_shared<Kind>(self, key: AnnexKey<Kind>) -> Option<Vec<u32>> {
        self.detach_span(key)
    }

    pub(super) fn inspect_fixed<Kind, Result>(
        self,
        key: AnnexKey<Kind>,
        body_words: usize,
        inspect: impl FnOnce(crate::fork_arena::ArenaListView<'a, u32, NodeAnnexLane>) -> Option<Result>,
    ) -> Option<Result> {
        let view = self.list(key)?;
        (view.len() == body_words + 1).then_some(())?;
        inspect(view)
    }

    pub(super) fn resolve_fixed_array<Kind, const N: usize>(
        self,
        key: AnnexKey<Kind>,
    ) -> Option<[u32; N]> {
        let view = self.list(key)?;
        if view.len() != N + 1 {
            return None;
        }
        let mut words = [0; N];
        let mut written = 0_usize;
        view.for_each_range(1..view.len(), |_, source| {
            words[written] = *source;
            written += 1;
        });
        debug_assert_eq!(written, N);
        Some(words)
    }

    pub(super) fn visit_span<Kind>(
        self,
        key: AnnexKey<Kind>,
        mut visit: impl FnMut(u32),
    ) -> Option<()> {
        let view = self.list(key)?;
        view.for_each_range(1..view.len(), |_, word| visit(*word));
        Some(())
    }

    pub(super) fn detach_span<Kind>(self, key: AnnexKey<Kind>) -> Option<Vec<u32>> {
        let view = self.list(key)?;
        let mut words = Vec::with_capacity(view.len().saturating_sub(1));
        view.for_each_range(1..view.len(), |_, word| words.push(*word));
        Some(words)
    }
}

impl<'a> NodeAnnexCopier<'a> {
    pub(super) fn same_region(
        pool: &'a mut ChunkPool<u32>,
        arena: &'a mut ForkArena<u32, NodeAnnexLane>,
    ) -> Self {
        Self {
            pool,
            source: NodeAnnexCopySource::SameRegion,
            destination: arena,
            dependency_floor: usize::MAX,
        }
    }

    pub(super) fn between_regions(
        pool: &'a mut ChunkPool<u32>,
        source: &'a ForkArena<u32, NodeAnnexLane>,
        destination: &'a mut ForkArena<u32, NodeAnnexLane>,
    ) -> Self {
        Self {
            pool,
            source: NodeAnnexCopySource::OtherRegion(source),
            destination,
            dependency_floor: usize::MAX,
        }
    }

    fn source_view(&self) -> NodeAnnexView<'_> {
        let arena = match self.source {
            NodeAnnexCopySource::SameRegion => &*self.destination,
            NodeAnnexCopySource::OtherRegion(arena) => arena,
        };
        NodeAnnexView::new(&*self.pool, arena)
    }

    pub(super) fn resolve_fixed_array<Kind, const N: usize>(
        &self,
        key: AnnexKey<Kind>,
    ) -> Option<[u32; N]> {
        self.source_view().resolve_fixed_array(key)
    }

    pub(super) fn detach_span<Kind>(&self, key: AnnexKey<Kind>) -> Option<Vec<u32>> {
        self.source_view().detach_span(key)
    }

    pub(super) fn append_fixed<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        let mut writer = NodeAnnexWriter::new(self.pool, self.destination);
        let key = writer.append_fixed(body);
        if let Some(floor) = writer.dependency_floor() {
            self.dependency_floor = self.dependency_floor.min(floor);
        }
        key
    }

    pub(super) fn append_span<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        let mut writer = NodeAnnexWriter::new(self.pool, self.destination);
        let key = writer.append_span(body);
        if let Some(floor) = writer.dependency_floor() {
            self.dependency_floor = self.dependency_floor.min(floor);
        }
        key
    }

    pub(super) fn dependency_floor(&self) -> Option<usize> {
        (self.dependency_floor != usize::MAX).then_some(self.dependency_floor)
    }
}
