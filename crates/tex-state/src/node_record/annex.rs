use super::*;

use crate::fork_arena::{ArenaListId, ChunkPool, ForkArena, ForkArenaError};
use crate::node_region::NodeAnnexLane;
use smallvec::SmallVec;

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
pub(super) enum BoxMigrationSegments {}
const BOX_MIGRATION_TAG: u32 = 0x424d_5347;
/// TeX fields, the original construction interval, and a typed sidecar key.
pub(super) const BOX_PAYLOAD_WORDS: usize = 43;
const COPIED_BOX_BODY_TAG: u32 = 0x4342_4f58;

/// Non-owning provenance captured during the required recursive copy of a
/// box's children. The actual consumed wrapper remains the move authority.
#[derive(Clone, Debug)]
pub(crate) struct CopiedBoxBodyStamp {
    region: crate::node_region::NodeRegionId,
    node_body: std::ops::Range<usize>,
    annex_body: std::ops::Range<usize>,
    wrapper_node_position: usize,
    wrapper_annex_position: usize,
}

impl CopiedBoxBodyStamp {
    pub(crate) fn new(
        region: crate::node_region::NodeRegionId,
        node_body: std::ops::Range<usize>,
        annex_body: std::ops::Range<usize>,
        wrapper_node_position: usize,
        wrapper_annex_position: usize,
    ) -> Option<Self> {
        if node_body.start > node_body.end
            || annex_body.start > annex_body.end
            || (node_body.is_empty() && annex_body.is_empty())
            || node_body.end > wrapper_node_position
            || annex_body.end > wrapper_annex_position
            || [
                node_body.start,
                node_body.end,
                annex_body.start,
                annex_body.end,
                wrapper_node_position,
                wrapper_annex_position,
            ]
            .into_iter()
            .any(|position| u32::try_from(position).is_err())
        {
            return None;
        }
        Some(Self {
            region,
            node_body,
            annex_body,
            wrapper_node_position,
            wrapper_annex_position,
        })
    }

    fn words(&self) -> [u32; 15] {
        let region = self.region.words();
        [
            0,
            0,
            0,
            0,
            self.node_body.start as u32,
            self.node_body.end as u32,
            self.annex_body.start as u32,
            self.annex_body.end as u32,
            region[0],
            region[1],
            region[2],
            region[3],
            self.wrapper_node_position as u32,
            self.wrapper_annex_position as u32,
            COPIED_BOX_BODY_TAG,
        ]
    }

    /// Writes the descriptor while the required explicit-copy body is still
    /// private scratch. Publication may seal an annex group immediately.
    pub(crate) fn write_flat_body(&self, body: &mut [u32]) -> Option<()> {
        if body.len() != BOX_PAYLOAD_WORDS || body[28..].iter().any(|word| *word != 0) {
            return None;
        }
        body[28..].copy_from_slice(&self.words());
        Some(())
    }

    fn from_words(words: [u32; 15]) -> Option<Self> {
        if words[..4] != [0; 4] || words[14] != COPIED_BOX_BODY_TAG {
            return None;
        }
        Self::new(
            crate::node_region::NodeRegionId::from_words(words[8..12].try_into().ok()?)?,
            words[4] as usize..words[5] as usize,
            words[6] as usize..words[7] as usize,
            words[12] as usize,
            words[13] as usize,
        )
    }

    pub(crate) fn metadata_at_wrapper(
        &self,
        region: crate::node_region::NodeRegionId,
        wrapper_node: usize,
        wrapper_annex: usize,
    ) -> Option<crate::page_node_arena::PageBoxMigrationMetadata> {
        // A whole-region move may change both owner-relative offsets, but
        // keeps the spacing between this wrapper and its copied child block.
        if self.region.words()[..2] != region.words()[..2] {
            return None;
        }
        let node_shift =
            i64::try_from(wrapper_node).ok()? - i64::try_from(self.wrapper_node_position).ok()?;
        let annex_shift =
            i64::try_from(wrapper_annex).ok()? - i64::try_from(self.wrapper_annex_position).ok()?;
        let rebase = |position: usize, shift: i64| -> Option<u32> {
            u32::try_from(i64::try_from(position).ok()?.checked_add(shift)?).ok()
        };
        let node_start = rebase(self.node_body.start, node_shift)?;
        let node_end = rebase(self.node_body.end, node_shift)?;
        let annex_start = rebase(self.annex_body.start, annex_shift)?;
        let annex_end = rebase(self.annex_body.end, annex_shift)?;
        let wrapper_node = u32::try_from(wrapper_node).ok()?;
        let wrapper_annex = u32::try_from(wrapper_annex).ok()?;
        let segment = crate::node_region::PageBoxSegment::from_exclusion_bounds(
            region,
            [
                node_start,
                wrapper_node.checked_add(1)?,
                annex_start,
                wrapper_annex.checked_add(1)?,
            ],
        )?;
        let exclusion = crate::node_region::PageBoxSegment::from_exclusion_bounds(
            region,
            [node_end, wrapper_node, annex_end, wrapper_annex],
        );
        if node_end > wrapper_node || annex_end > wrapper_annex {
            return None;
        }
        Some(crate::page_node_arena::PageBoxMigrationMetadata {
            segment,
            exclusions: exclusion.into_iter().collect(),
            sidecar_annex_range: None,
            wrapper_rebuild: true,
        })
    }
}

pub(super) enum BoxConstructionDescriptor {
    None,
    Original {
        segment: crate::node_region::PageBoxSegment,
        migrations: Option<crate::page_node_arena::PageBoxMigrationKey>,
    },
    Copied(CopiedBoxBodyStamp),
}

pub(super) fn decode_box_construction_descriptor(
    words: &[u32],
) -> Option<BoxConstructionDescriptor> {
    if words.len() == 28 {
        return Some(BoxConstructionDescriptor::None);
    }
    let words: [u32; 15] = words.get(28..43)?.try_into().ok()?;
    if words.iter().all(|word| *word == 0) {
        return Some(BoxConstructionDescriptor::None);
    }
    if words[..4] == [0; 4] {
        return Some(BoxConstructionDescriptor::Copied(
            CopiedBoxBodyStamp::from_words(words)?,
        ));
    }
    let segment = crate::node_region::PageBoxSegment::from_words(words[..8].try_into().ok()?)?;
    let key: [u32; 7] = words[8..].try_into().ok()?;
    let migrations = (!key.iter().all(|word| *word == 0))
        .then(|| crate::page_node_arena::PageBoxMigrationKey::from_words(key));
    Some(BoxConstructionDescriptor::Original {
        segment,
        migrations,
    })
}
/// The largest fixed body accepted by both the typed writer and prepared copy.
/// Math choices use 40 words; a larger box construction sidecar raises this
/// bound without changing the relocation storage in a separate place.
pub(super) const MAX_FIXED_COPY_BODY_WORDS: usize = if BOX_PAYLOAD_WORDS > 40 {
    BOX_PAYLOAD_WORDS
} else {
    40
};
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

pub(crate) fn valid_box_exclusions(
    region: [u32; 4],
    segments: &[crate::node_region::PageBoxSegment],
) -> bool {
    let mut previous = None;
    for &segment in segments {
        let words = segment.words();
        if words[..4] != region
            || words[4] > words[5]
            || words[6] > words[7]
            || (words[4] == words[5] && words[6] == words[7])
            || previous.is_some_and(|end: [u32; 2]| end[0] > words[4] || end[1] > words[6])
        {
            return false;
        }
        previous = Some([words[5], words[7]]);
    }
    true
}

pub(super) fn key_words<Kind>(key: AnnexKey<Kind>) -> [u32; 7] {
    key.words()
}

pub(super) fn key_from_record<Kind>(record: NodeRecord) -> AnnexKey<Kind> {
    let words = record.words();
    AnnexKey::from_words(words)
}

pub(super) fn set_fixed_box_word(
    pool: &mut ChunkPool<u32>,
    arena: &mut ForkArena<u32, NodeAnnexLane>,
    key: AnnexKey<BoxPayload>,
    offset: usize,
    value: Scaled,
) -> Result<Scaled, crate::fork_arena::ForkArenaError> {
    use crate::fork_arena::ForkArenaError;

    // Validate the complete typed fixed record before taking a mutable
    // single-word view. The sealed list coordinate and child fields stay put.
    NodeAnnexView::new(pool, arena)
        .fixed_words(key, BOX_PAYLOAD_WORDS)
        .ok_or(ForkArenaError::InvalidRange)?;
    let list = key
        .list(pool.logical_space(), pool.chunk_capacity())
        .ok_or(ForkArenaError::InvalidRange)?;
    let mut scratch = Vec::new();
    let word = arena.slice_list(pool, list, offset..offset + 1, &mut scratch)?;
    arena.with_single_value_mut(pool, word, |old| {
        let previous = Scaled::from_raw(*old as i32);
        *old = value.raw() as u32;
        previous
    })
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

pub(super) fn encode_standalone_box_payload(value: BoxNode<PageListId>) -> Vec<u32> {
    let mut words = encode_box_payload(value);
    words.resize(BOX_PAYLOAD_WORDS, 0);
    words
}

pub(super) fn decode_box_payload(words: &[u32]) -> Option<BoxNode<PageListId>> {
    if words.len() != 28 && words.len() != BOX_PAYLOAD_WORDS {
        return None;
    }
    decode_box_construction_descriptor(words)?;
    let words = &words[..28];
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
            body.len() <= MAX_FIXED_COPY_BODY_WORDS,
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

    pub(super) fn stamp_box_segment(
        &mut self,
        key: AnnexKey<BoxPayload>,
        segment: crate::node_region::PageBoxSegment,
        migrations: Option<crate::page_node_arena::PageBoxMigrationKey>,
    ) -> Option<()> {
        let list = key.list(self.pool.logical_space(), self.pool.chunk_capacity())?;
        let mut metadata = [0; 15];
        metadata[..8].copy_from_slice(&segment.words());
        if let Some(migrations) = migrations {
            metadata[8..].copy_from_slice(&migrations.words());
        }
        self.arena
            .stamp_unsealed_zero_range(self.pool, list, 29, metadata)
            .ok()
    }

    pub(crate) fn publish_box_migration_segments(
        &mut self,
        segments: &[crate::node_region::PageBoxSegment],
    ) -> crate::page_node_arena::PageBoxMigrationKey {
        let words = std::iter::once(BOX_MIGRATION_TAG).chain(segments.iter().flat_map(|segment| {
            let words = segment.words();
            [words[4], words[5], words[6], words[7]]
        }));
        let key = self.append_span_iter::<BoxMigrationSegments>(words);
        crate::page_node_arena::PageBoxMigrationKey::from_words(key.words())
    }

    /// Publishes independent, already-staged fixed bodies through admitted
    /// physical word runs. Each leading placeholder becomes a fresh serial;
    /// every returned key names only its own authenticated record.
    pub(crate) fn append_fixed_flat(
        &mut self,
        words: &mut [u32],
        lengths: &[u16],
        mut prepare_body: impl FnMut(usize, usize, &mut [u32]) -> Result<(), ForkArenaError>,
    ) -> Result<SmallVec<[AnnexKey<()>; 16]>, ForkArenaError> {
        let mut serials = SmallVec::<[u32; 16]>::new();
        let mut offset = 0usize;
        for &len in lengths {
            let len = usize::from(len);
            if len == 0 || len > MAX_FIXED_COPY_BODY_WORDS + 1 {
                return Err(ForkArenaError::InvalidRange);
            }
            let serial = self.pool.next_publication_serial();
            *words.get_mut(offset).ok_or(ForkArenaError::InvalidRange)? = serial;
            serials.push(serial);
            offset = offset
                .checked_add(len)
                .ok_or(ForkArenaError::InvalidRange)?;
        }
        if offset != words.len() {
            return Err(ForkArenaError::InvalidRange);
        }
        let mut keys = SmallVec::<[AnnexKey<()>; 16]>::new();
        let mut first_list = None;
        self.arena.append_unsealed_fixed_batch_copy_parts(
            self.pool,
            words,
            lengths,
            |index, position, fixed| prepare_body(index, position, &mut fixed[1..]),
            |list| {
                first_list.get_or_insert(list);
                keys.push(AnnexKey::from_list(list, serials[keys.len()]));
            },
        )?;
        if let Some(first_list) = first_list {
            let position = self
                .arena
                .owner_relative_head_position(self.pool, first_list)?;
            self.dependency_floor = self.dependency_floor.min(position);
        }
        Ok(keys)
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

    /// Appends a typed span from a caller-owned iterator directly into the
    /// annex arena. The iterator is consumed by the arena's append run, so
    /// variable-length payloads do not need a temporary word vector.
    pub(crate) fn append_span_iter<Kind>(
        &mut self,
        body: impl IntoIterator<Item = u32>,
    ) -> AnnexKey<Kind> {
        let publication_serial = self.pool.next_publication_serial();
        let list = self
            .arena
            .append_unsealed_list(self.pool, std::iter::once(publication_serial).chain(body))
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

    #[cfg(test)]
    pub(crate) fn resolve_fixed_shared<Kind>(self, key: AnnexKey<Kind>) -> Option<Vec<u32>> {
        self.detach_span(key)
    }

    fn fixed_words<Kind>(self, key: AnnexKey<Kind>, body_words: usize) -> Option<&'a [u32]> {
        let list = key.list(self.pool.logical_space(), self.pool.chunk_capacity())?;
        let view = self.arena.list(self.pool, list).ok()?;
        let words = view.contiguous_packed_slice()?;
        (words.len() == body_words.checked_add(1)? && words.first()? == &key.words[6])
            .then_some(words)
    }

    pub(super) fn inspect_fixed<Kind, Result>(
        self,
        key: AnnexKey<Kind>,
        body_words: usize,
        inspect: impl FnOnce(&'a [u32]) -> Option<Result>,
    ) -> Option<Result> {
        inspect(self.fixed_words(key, body_words)?)
    }

    pub(super) fn resolve_fixed_array<Kind, const N: usize>(
        self,
        key: AnnexKey<Kind>,
    ) -> Option<[u32; N]> {
        self.fixed_words(key, N)?.get(1..)?.try_into().ok()
    }

    pub(super) fn fixed_block_range<Kind>(
        self,
        key: AnnexKey<Kind>,
        body_words: usize,
    ) -> Option<std::ops::Range<usize>> {
        self.fixed_words(key, body_words)?;
        let list = key.list(self.pool.logical_space(), self.pool.chunk_capacity())?;
        self.arena
            .owner_relative_list_block_range(self.pool, list)
            .ok()
    }

    pub(super) fn key_block_range<Kind>(
        self,
        key: AnnexKey<Kind>,
    ) -> Option<std::ops::Range<usize>> {
        self.list(key)?;
        let list = key.list(self.pool.logical_space(), self.pool.chunk_capacity())?;
        self.arena
            .owner_relative_list_block_range(self.pool, list)
            .ok()
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

    pub(super) fn box_migration_segments(
        self,
        key: crate::page_node_arena::PageBoxMigrationKey,
        segment: crate::node_region::PageBoxSegment,
    ) -> Option<(
        Vec<crate::node_region::PageBoxSegment>,
        std::ops::Range<usize>,
    )> {
        self.box_migration_segments_rebased(key, segment, segment)
    }

    pub(super) fn box_migration_segments_rebased(
        self,
        key: crate::page_node_arena::PageBoxMigrationKey,
        original: crate::node_region::PageBoxSegment,
        segment: crate::node_region::PageBoxSegment,
    ) -> Option<(
        Vec<crate::node_region::PageBoxSegment>,
        std::ops::Range<usize>,
    )> {
        let key = AnnexKey::<BoxMigrationSegments>::from_words(key.words());
        let list = key.list(self.pool.logical_space(), self.pool.chunk_capacity())?;
        let view = self.list(key)?;
        let sidecar_range = self
            .arena
            .owner_relative_list_block_range(self.pool, list)
            .ok()?;
        let box_annex = segment.annex_range();
        if sidecar_range.start < box_annex.start
            || sidecar_range.end != box_annex.end.checked_sub(1)?
            || view.len() < 6
            || (view.len() - 2) % 4 != 0
        {
            return None;
        }
        let words = self.detach_span(key)?;
        if words.first().copied()? != BOX_MIGRATION_TAG {
            return None;
        }
        let mut exclusions = Vec::with_capacity((words.len() - 1) / 4 + 1);
        let body_node_end = segment.node_range().end.checked_sub(1)?;
        for &bounds in words[1..].as_chunks::<4>().0 {
            let exclusion = original
                .exclusion_from_bounds(bounds)?
                .shifted_like(original, segment)?;
            if exclusion.node_range().start < segment.node_range().start
                || exclusion.node_range().end > body_node_end
                || exclusion.annex_range().start < box_annex.start
                || exclusion.annex_range().end > sidecar_range.start
            {
                return None;
            }
            exclusions.push(exclusion);
        }
        if !valid_box_exclusions(segment.words()[..4].try_into().ok()?, &exclusions) {
            return None;
        }
        let sidecar_node = u32::try_from(body_node_end).ok()?;
        let sidecar = segment.exclusion_from_bounds([
            sidecar_node,
            sidecar_node,
            u32::try_from(sidecar_range.start).ok()?,
            u32::try_from(sidecar_range.end).ok()?,
        ])?;
        exclusions.push(sidecar);
        Some((exclusions, sidecar_range))
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

    pub(super) fn source_view(&self) -> NodeAnnexView<'_> {
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

    pub(super) fn detach_span<Kind>(&self, key: AnnexKey<Kind>) -> Option<SmallVec<[u32; 64]>> {
        let view = self.source_view().list(key)?;
        let mut words = SmallVec::new();
        if let Some(source) = view.contiguous_packed_slice() {
            words.extend_from_slice(source.get(1..)?);
        } else {
            view.visit_range_chunks(1..view.len(), |chunk| {
                if let Some(source) = chunk.packed_slice() {
                    words.extend_from_slice(source);
                } else {
                    chunk.for_each(|word| words.push(*word));
                }
            });
        }
        Some(words)
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
