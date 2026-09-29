use super::annex::{BoxMigrationSegments, BoxPositiveRanges};
use super::box_descriptor::{BoxConstructionDescriptor, decode_box_construction_descriptor};
use super::*;

/// Offsets of independently owned child-list coordinates in one fixed body.
#[derive(Clone, Copy)]
pub(crate) struct FixedCopyFields {
    offsets: [u8; 4],
    count: usize,
}

impl FixedCopyFields {
    fn new(record: NodeRecord<PageMaterialLane>, body: &[u32]) -> Option<Self> {
        let mut fields = Self {
            offsets: [0; 4],
            count: 0,
        };
        let mut child_at = |offset: usize| {
            fields.offsets[fields.count] = offset as u8;
            fields.count += 1;
        };
        match record.kind()? {
            NodeKind::Glue => {
                child_at(11);
                child_at(21);
            }
            NodeKind::HList | NodeKind::VList => {
                child_at(7);
                child_at(17);
            }
            NodeKind::Unset | NodeKind::Ins | NodeKind::MathList | NodeKind::Adjust => child_at(0),
            NodeKind::Disc => {
                for offset in [0, 10, 20] {
                    child_at(offset);
                }
            }
            NodeKind::MathNoad => {
                for offset in [3, 14, 25] {
                    match *body.get(offset)? {
                        0..=2 => {}
                        3 | 4 => child_at(offset + 1),
                        _ => return None,
                    }
                }
            }
            NodeKind::FractionNoad => {
                child_at(0);
                child_at(10);
            }
            NodeKind::MathChoice => {
                for offset in [0, 10, 20, 30] {
                    child_at(offset);
                }
            }
            _ => return None,
        }
        Some(fields)
    }

    pub(crate) fn offsets(&self) -> &[u8] {
        &self.offsets[..self.count]
    }
}

impl NodeRecord<PageMaterialLane> {
    /// Inline leaves contain neither node-region children nor annex coordinates.
    /// Their complete record may cross an explicit copy boundary unchanged.
    pub(crate) fn is_inline_leaf(self) -> bool {
        match self.kind() {
            Some(
                NodeKind::Char
                | NodeKind::Kern
                | NodeKind::MarginKern
                | NodeKind::Penalty
                | NodeKind::Rule
                | NodeKind::Mark
                | NodeKind::MathOn
                | NodeKind::MathOff
                | NodeKind::Direction
                | NodeKind::MathStyle
                | NodeKind::Nonscript,
            ) => true,
            Some(NodeKind::Glue) => !matches!(self.flags() & 3, 2 | 3),
            _ => false,
        }
    }

    pub(crate) fn has_fixed_copy_payload(self) -> bool {
        matches!(
            self.kind(),
            Some(
                NodeKind::HList
                    | NodeKind::VList
                    | NodeKind::Unset
                    | NodeKind::Disc
                    | NodeKind::Ins
                    | NodeKind::MathNoad
                    | NodeKind::FractionNoad
                    | NodeKind::MathChoice
                    | NodeKind::MathList
                    | NodeKind::Adjust
            )
        ) || (self.kind() == Some(NodeKind::Glue) && matches!(self.flags() & 3, 2 | 3))
    }

    pub(crate) fn with_relocated_fixed_key(self, key: AnnexKey<()>) -> Option<Self> {
        Some(Self::with_key(
            self.kind()?,
            self.subtype(),
            self.flags(),
            key,
        ))
    }

    /// Authenticates a fixed source body and lends its words for direct batch
    /// staging. The borrow ends before the destination annex is mutated.
    pub(crate) fn with_fixed_copy_body<R>(
        self,
        annex: NodeAnnexView<'_>,
        visit: impl FnOnce(&[u32], FixedCopyFields) -> R,
    ) -> Option<R> {
        let len = self.fixed_copy_body_len()?;
        annex.inspect_fixed(key_from_record::<()>(self), len, |words| {
            let body = words.get(1..)?;
            let fields = FixedCopyFields::new(self, body)?;
            Some(visit(body, fields))
        })
    }

    /// Uses the same typed child-field decoder after operation-scoped source
    /// chunk admission. The source body borrow ends before destination writes.
    pub(crate) fn with_cached_fixed_copy_body<R>(
        self,
        annex: &mut NodeAnnexCopyReader<'_>,
        pool: &crate::fork_arena::ChunkPool<u32>,
        visit: impl FnOnce(&[u32], FixedCopyFields) -> R,
    ) -> Option<R> {
        let len = self.fixed_copy_body_len()?;
        annex.inspect_fixed(pool, key_from_record::<()>(self), len, |words| {
            let body = words.get(1..)?;
            let fields = FixedCopyFields::new(self, body)?;
            Some(visit(body, fields))
        })
    }

    fn fixed_copy_body_len(self) -> Option<usize> {
        Some(match self.kind()? {
            NodeKind::Glue if matches!(self.flags() & 3, 2 | 3) => 32,
            NodeKind::HList | NodeKind::VList => BOX_PAYLOAD_WORDS,
            NodeKind::Unset => 15,
            NodeKind::Disc => 30,
            NodeKind::Ins => 17,
            NodeKind::MathNoad => 36,
            NodeKind::FractionNoad => 23,
            NodeKind::MathChoice => 40,
            NodeKind::MathList | NodeKind::Adjust => 10,
            _ => return None,
        })
    }

    pub(crate) fn direct_font(self, annex: NodeAnnexView<'_>) -> Option<FontId> {
        match self.kind()? {
            NodeKind::Char => self.character().map(|(font, _, _)| font),
            NodeKind::Lig => self.visit_ligature_source(annex, |_, _| {}),
            NodeKind::MarginKern => FontId::from_words(self.words()[1..5].try_into().ok()?),
            _ => None,
        }
    }

    pub(crate) fn tex_memory_words(
        self,
        annex: NodeAnnexView<'_>,
        etex_node_sizes: bool,
    ) -> (usize, usize) {
        let synctex_extra = usize::from(etex_node_sizes) * 2;
        let Some(kind) = self.kind() else {
            return (0, 0);
        };
        match kind {
            NodeKind::Char => (0, 1),
            NodeKind::Lig => {
                let mut length = 0;
                let _ = self.visit_ligature_source(annex, |_, _| length += 1);
                (2, length)
            }
            NodeKind::HList | NodeKind::VList | NodeKind::Unset => (7 + synctex_extra, 0),
            NodeKind::Rule => (4 + synctex_extra, 0),
            NodeKind::Ins => (5, 0),
            NodeKind::MathNoad => (self.math_noad_memory_words(annex).unwrap_or(4), 0),
            NodeKind::FractionNoad => (6, 0),
            NodeKind::MathStyle | NodeKind::MathChoice | NodeKind::MarginKern => (3, 0),
            NodeKind::Kern
            | NodeKind::Glue
            | NodeKind::Penalty
            | NodeKind::MathOn
            | NodeKind::MathOff
            | NodeKind::Nonscript => (2 + synctex_extra, 0),
            NodeKind::Direction if etex_node_sizes => (2 + synctex_extra, 0),
            NodeKind::Disc
            | NodeKind::Mark
            | NodeKind::Whatsit
            | NodeKind::Direction
            | NodeKind::MathList
            | NodeKind::Adjust => (2, 0),
        }
    }

    pub(crate) fn math_noad_memory_words(self, annex: NodeAnnexView<'_>) -> Option<usize> {
        if self.kind()? != NodeKind::MathNoad || self.subtype() != 0 || self.flags() != 0 {
            return None;
        }
        annex.inspect_fixed(key_from_record::<MathNoadPayload>(self), 36, |payload| {
            let tag = *payload.get(1)? >> 8;
            Some(if matches!(tag, 2 | 3) { 5 } else { 4 })
        })
    }

    pub(crate) fn visit_node_lists(
        self,
        annex: NodeAnnexView<'_>,
        mut visit: impl FnMut(PageListId),
    ) -> Option<()> {
        self.kind()?;
        if self.has_fixed_copy_payload() {
            self.with_fixed_copy_body(annex, |body, fields| {
                for &offset in fields.offsets() {
                    let offset = usize::from(offset);
                    visit(PageListId::from_words(
                        body[offset..offset + 10].try_into().ok()?,
                    )?);
                }
                Some(())
            })?
        } else {
            Some(())
        }
    }

    /// Visits this record's direct child lists, then its direct typed-annex
    /// blocks, exactly as [`Self::visit_node_lists`] followed by
    /// [`Self::visit_annex_block_ranges`], but admits a fixed payload once.
    pub(crate) fn visit_closure_references(
        self,
        annex: NodeAnnexView<'_>,
        mut visit_list: impl FnMut(PageListId),
        mut visit_annex: impl FnMut(std::ops::Range<usize>),
    ) -> Option<()> {
        let kind = self.kind()?;
        if self.is_inline_leaf() {
            return Some(());
        }
        if !self.has_fixed_copy_payload() {
            return self.visit_annex_block_ranges(annex, visit_annex);
        }
        let (words, range) = annex.fixed_words_and_block_range(
            key_from_record::<()>(self),
            self.fixed_copy_body_len()?,
        )?;
        let body = words.get(1..)?;
        let fields = FixedCopyFields::new(self, body)?;
        for &offset in fields.offsets() {
            let offset = usize::from(offset);
            visit_list(PageListId::from_words(
                body.get(offset..offset + 10)?.try_into().ok()?,
            )?);
        }
        visit_annex(range);
        if matches!(kind, NodeKind::HList | NodeKind::VList) {
            match decode_box_construction_descriptor(body)? {
                BoxConstructionDescriptor::Original {
                    migrations: Some(key),
                    ..
                } => {
                    visit_annex(annex.key_block_range(
                        AnnexKey::<BoxMigrationSegments>::from_words(key.words()),
                    )?)
                }
                BoxConstructionDescriptor::Positive { key, .. } => visit_annex(
                    annex
                        .key_block_range(AnnexKey::<BoxPositiveRanges>::from_words(key.words()))?,
                ),
                _ => {}
            }
        }
        Some(())
    }

    /// Visits the direct typed-annex blocks held by this record. A partition
    /// preflight calls this once per selected record; it does not follow TeX
    /// child lists or scan unrelated page roots.
    pub(crate) fn visit_annex_block_ranges(
        self,
        annex: NodeAnnexView<'_>,
        mut visit: impl FnMut(std::ops::Range<usize>),
    ) -> Option<()> {
        if self.is_inline_leaf() {
            return Some(());
        }
        if self.has_fixed_copy_payload() {
            self.with_fixed_copy_body(annex, |_, _| ())?;
            visit(annex.key_block_range(key_from_record::<()>(self))?);
            if matches!(self.kind(), Some(NodeKind::HList | NodeKind::VList)) {
                let payload = annex
                    .resolve_fixed_array::<BoxPayload, BOX_PAYLOAD_WORDS>(key_from_record(self))?;
                match decode_box_construction_descriptor(&payload)? {
                    BoxConstructionDescriptor::Original {
                        migrations: Some(key),
                        ..
                    } => visit(annex.key_block_range(
                        AnnexKey::<BoxMigrationSegments>::from_words(key.words()),
                    )?),
                    BoxConstructionDescriptor::Positive { key, .. } => visit(
                        annex.key_block_range(AnnexKey::<BoxPositiveRanges>::from_words(
                            key.words(),
                        ))?,
                    ),
                    _ => {}
                }
            }
            return Some(());
        }
        match self.kind()? {
            NodeKind::Lig => {
                let payload =
                    annex.resolve_fixed_array::<LigaturePayload, 12>(key_from_record(self))?;
                let source =
                    AnnexKey::<LigatureSource>::from_words(payload[5..12].try_into().ok()?);
                visit(annex.key_block_range(source)?);
                visit(annex.key_block_range(key_from_record::<LigaturePayload>(self))?);
                Some(())
            }
            NodeKind::Whatsit => self.visit_whatsit_annex_block_ranges(annex, visit),
            _ => None,
        }
    }

    pub(crate) fn reencode_same_region(
        self,
        pool: &mut crate::fork_arena::ChunkPool<u32>,
        arena: &mut crate::fork_arena::ForkArena<u32, crate::node_region::NodeAnnexLane>,
        map_child: impl FnMut(PageListId) -> Option<PageListId>,
    ) -> Option<(Self, Option<usize>)> {
        let mut annex = NodeAnnexCopier::same_region(pool, arena);
        let relocated = self.reencode_into(&mut annex, map_child);
        if relocated.is_some() {
            annex.commit_reencoded();
        }
        relocated
    }

    pub(crate) fn reencode_between_regions(
        self,
        pool: &mut crate::fork_arena::ChunkPool<u32>,
        source: &mut NodeAnnexCopyReader<'_>,
        destination: &mut crate::fork_arena::ForkArena<u32, crate::node_region::NodeAnnexLane>,
        map_child: impl FnMut(PageListId) -> Option<PageListId>,
    ) -> Option<(Self, Option<usize>)> {
        let mut annex = NodeAnnexCopier::between_regions(pool, source, destination);
        let relocated = self.reencode_into(&mut annex, map_child);
        if relocated.is_some() {
            annex.commit_reencoded();
        }
        relocated
    }

    fn reencode_into(
        self,
        annex: &mut NodeAnnexCopier<'_, '_>,
        mut map_child: impl FnMut(PageListId) -> Option<PageListId>,
    ) -> Option<(Self, Option<usize>)> {
        if self.has_fixed_copy_payload() {
            let (mut body, len, fields) =
                self.with_fixed_copy_body(annex.source_view(), |source, fields| {
                    let mut body = [0; MAX_FIXED_COPY_BODY_WORDS];
                    body[..source.len()].copy_from_slice(source);
                    (body, source.len(), fields)
                })?;
            if matches!(self.kind(), Some(NodeKind::HList | NodeKind::VList)) {
                body[28..BOX_PAYLOAD_WORDS].fill(0);
            }
            for &offset in fields.offsets() {
                let offset = usize::from(offset);
                let source = PageListId::from_words(body[offset..offset + 10].try_into().ok()?)?;
                body[offset..offset + 10].copy_from_slice(&map_child(source)?.words());
            }
            let key = annex.append_fixed::<()>(&body[..len]);
            return Some((
                self.with_relocated_fixed_key(key)?,
                annex.dependency_floor(),
            ));
        }

        let kind = self.kind()?;
        let subtype = self.subtype();
        let flags = self.flags();
        match kind {
            NodeKind::Lig => {
                let mut payload =
                    annex.resolve_fixed_array::<LigaturePayload, 12>(key_from_record(self))?;
                let source =
                    AnnexKey::<LigatureSource>::from_words(payload[5..12].try_into().ok()?);
                let source = annex.detach_span(source)?;
                let source = annex.append_span::<LigatureSource>(&source);
                payload[5..12].copy_from_slice(&source.words());
                let key = annex.append_fixed::<LigaturePayload>(&payload);
                Some((
                    Self::with_key(kind, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            NodeKind::Whatsit => self.reencode_whatsit(annex),
            NodeKind::Char
            | NodeKind::Kern
            | NodeKind::MarginKern
            | NodeKind::Glue
            | NodeKind::Penalty
            | NodeKind::Rule
            | NodeKind::Mark
            | NodeKind::MathOn
            | NodeKind::MathOff
            | NodeKind::Direction
            | NodeKind::MathStyle
            | NodeKind::Nonscript => Some((self, None)),
            _ => None,
        }
    }

    pub(crate) fn character(self) -> Option<(FontId, char, OriginId)> {
        (self.kind()? == NodeKind::Char
            && self.subtype() == 0
            && self.flags() == 0
            && self.words()[6] == 0)
            .then(|| {
                let words = self.words();
                Some((
                    FontId::from_words(words[..4].try_into().ok()?)?,
                    char::from_u32(words[4])?,
                    OriginId::from_raw(words[5]),
                ))
            })?
    }

    pub(crate) fn glyph(self, annex: NodeAnnexView<'_>) -> Option<(FontId, char)> {
        match self.kind()? {
            NodeKind::Char => self.character().map(|(font, ch, _)| (font, ch)),
            NodeKind::Lig if self.subtype() == 0 && self.flags() & !3 == 0 => {
                annex.inspect_fixed(key_from_record::<LigaturePayload>(self), 12, |payload| {
                    let mut font = [0; 4];
                    for (index, word) in font.iter_mut().enumerate() {
                        *word = *payload.get(index + 1)?;
                    }
                    Some((FontId::from_words(font)?, char::from_u32(*payload.get(5)?)?))
                })
            }
            _ => None,
        }
    }

    pub(crate) fn kern(self) -> Option<(Scaled, KernKind)> {
        (self.kind()? == NodeKind::Kern
            && self.flags() == 0
            && self.words()[1..].iter().all(|word| *word == 0))
        .then(|| {
            Some((
                decode_scaled(self.words()[0]),
                decode_kern_kind(self.subtype())?,
            ))
        })?
    }

    pub(crate) fn margin_kern_amount(self) -> Option<Scaled> {
        (self.kind()? == NodeKind::MarginKern
            && self.flags() == 0
            && self.words()[6] == 0
            && self.words()[5] <= u8::MAX as u32
            && decode_margin_side(self.subtype()).is_some()
            && FontId::from_words(self.words()[1..5].try_into().ok()?).is_some())
        .then(|| decode_scaled(self.words()[0]))
    }

    pub(crate) fn is_font_kern(self) -> bool {
        self.kind() == Some(NodeKind::Kern)
            && self.flags() == 0
            && self.words()[1..].iter().all(|word| *word == 0)
            && decode_kern_kind(self.subtype()) == Some(KernKind::Font)
    }

    pub(crate) fn is_glue(self) -> bool {
        self.kind() == Some(NodeKind::Glue)
    }

    pub(crate) fn penalty(self) -> Option<i32> {
        (self.kind()? == NodeKind::Penalty
            && self.subtype() == 0
            && self.flags() == 0
            && self.words()[1..].iter().all(|word| *word == 0))
        .then(|| self.words()[0] as i32)
    }

    pub(crate) fn rule_width(self) -> Option<Option<Scaled>> {
        (self.kind()? == NodeKind::Rule
            && self.subtype() == 0
            && self.flags() & !7 == 0
            && self.words()[3..].iter().all(|word| *word == 0))
        .then(|| (self.flags() & 1 != 0).then(|| decode_scaled(self.words()[0])))
    }

    pub(crate) fn box_width(self, annex: NodeAnnexView<'_>) -> Option<Scaled> {
        (matches!(self.kind()?, NodeKind::HList | NodeKind::VList)
            && self.subtype() == 0
            && self.flags() == 0)
            .then(|| {
                annex.inspect_fixed(
                    key_from_record::<BoxPayload>(self),
                    BOX_PAYLOAD_WORDS,
                    |payload| payload.get(1).copied().map(decode_scaled),
                )
            })?
    }

    /// The original box wrapper's construction range, if it was stamped by
    /// its semantic list owner before the wrapper was sealed. A structural
    /// re-encode clears this hint, so it cannot duplicate move authority.
    pub(crate) fn box_segment(
        self,
        annex: NodeAnnexView<'_>,
    ) -> Option<crate::node_region::PageBoxSegment> {
        if !matches!(self.kind()?, NodeKind::HList | NodeKind::VList)
            || self.subtype() != 0
            || self.flags() != 0
        {
            return None;
        }
        let payload =
            annex.resolve_fixed_array::<BoxPayload, BOX_PAYLOAD_WORDS>(key_from_record(self))?;
        match decode_box_construction_descriptor(&payload)? {
            BoxConstructionDescriptor::Original { segment, .. } => Some(segment),
            _ => None,
        }
    }

    pub(crate) fn box_migration_metadata(
        self,
        annex: NodeAnnexView<'_>,
    ) -> Option<crate::page_node_arena::PageBoxMigrationMetadata> {
        if !matches!(self.kind()?, NodeKind::HList | NodeKind::VList)
            || self.subtype() != 0
            || self.flags() != 0
        {
            return None;
        }
        let payload =
            annex.resolve_fixed_array::<BoxPayload, BOX_PAYLOAD_WORDS>(key_from_record(self))?;
        let (segment, migrations) = match decode_box_construction_descriptor(&payload)? {
            BoxConstructionDescriptor::Original {
                segment,
                migrations,
            } => (segment, migrations),
            _ => return None,
        };
        let (exclusions, sidecar_annex_range) = if let Some(key) = migrations {
            let (exclusions, range) = annex.box_migration_segments(key, segment)?;
            (exclusions, Some(range))
        } else {
            (Vec::new(), None)
        };
        Some(crate::page_node_arena::PageBoxMigrationMetadata {
            segment,
            exclusions,
            positive: None,
            sidecar_annex_range,
            wrapper_rebuild: false,
        })
    }

    pub(crate) fn box_payload_block_range(
        self,
        annex: NodeAnnexView<'_>,
    ) -> Option<std::ops::Range<usize>> {
        if !matches!(self.kind()?, NodeKind::HList | NodeKind::VList) {
            return None;
        }
        annex.fixed_block_range(key_from_record::<BoxPayload>(self), BOX_PAYLOAD_WORDS)
    }

    pub(crate) fn box_migration_metadata_rebased(
        self,
        annex: NodeAnnexView<'_>,
        segment: crate::node_region::PageBoxSegment,
    ) -> Option<crate::page_node_arena::PageBoxMigrationMetadata> {
        if !matches!(self.kind()?, NodeKind::HList | NodeKind::VList)
            || self.subtype() != 0
            || self.flags() != 0
        {
            return None;
        }
        let payload =
            annex.resolve_fixed_array::<BoxPayload, BOX_PAYLOAD_WORDS>(key_from_record(self))?;
        let (original, migrations) = match decode_box_construction_descriptor(&payload)? {
            BoxConstructionDescriptor::Original {
                segment,
                migrations,
            } => (segment, migrations),
            _ => return None,
        };
        let (exclusions, sidecar_annex_range) = if let Some(key) = migrations {
            let (exclusions, range) =
                annex.box_migration_segments_rebased(key, original, segment)?;
            (exclusions, Some(range))
        } else {
            (Vec::new(), None)
        };
        Some(crate::page_node_arena::PageBoxMigrationMetadata {
            segment,
            exclusions,
            positive: None,
            sidecar_annex_range,
            wrapper_rebuild: false,
        })
    }

    pub(crate) fn copied_box_body_stamp(
        self,
        annex: NodeAnnexView<'_>,
    ) -> Option<crate::node_record::CopiedBoxBodyStamp> {
        if !matches!(self.kind()?, NodeKind::HList | NodeKind::VList)
            || self.subtype() != 0
            || self.flags() != 0
        {
            return None;
        }
        let payload =
            annex.resolve_fixed_array::<BoxPayload, BOX_PAYLOAD_WORDS>(key_from_record(self))?;
        match decode_box_construction_descriptor(&payload)? {
            BoxConstructionDescriptor::Copied(stamp) => Some(stamp),
            _ => None,
        }
    }

    pub(crate) fn positive_box_metadata_at_wrapper(
        self,
        annex: NodeAnnexView<'_>,
        region: crate::node_region::NodeRegionId,
        wrapper_node: usize,
        wrapper_annex: usize,
    ) -> Option<crate::page_node_arena::PageBoxMigrationMetadata> {
        if !matches!(self.kind()?, NodeKind::HList | NodeKind::VList)
            || self.subtype() != 0
            || self.flags() != 0
        {
            return None;
        }
        let payload =
            annex.resolve_fixed_array::<BoxPayload, BOX_PAYLOAD_WORDS>(key_from_record(self))?;
        let BoxConstructionDescriptor::Positive {
            region: original_region,
            key,
            wrapper_node: original_node,
            wrapper_annex: original_annex,
        } = decode_box_construction_descriptor(&payload)?
        else {
            return None;
        };
        if original_region.words()[..2] != region.words()[..2] {
            return None;
        }
        let node_shift = i64::try_from(wrapper_node).ok()? - i64::try_from(original_node).ok()?;
        let annex_shift =
            i64::try_from(wrapper_annex).ok()? - i64::try_from(original_annex).ok()?;
        let ranges =
            annex.box_positive_ranges(key, wrapper_annex, node_shift, annex_shift, wrapper_node)?;
        let segment = crate::node_region::PageBoxSegment::from_exclusion_bounds(
            region,
            [
                u32::try_from(wrapper_node).ok()?,
                u32::try_from(wrapper_node.checked_add(1)?).ok()?,
                u32::try_from(ranges.sidecar.start).ok()?,
                u32::try_from(wrapper_annex.checked_add(1)?).ok()?,
            ],
        )?;
        Some(crate::page_node_arena::PageBoxMigrationMetadata {
            segment,
            exclusions: Vec::new(),
            positive: Some(crate::page_node_arena::PageBoxPositiveSelection {
                nodes: ranges.nodes,
                annex: ranges.annex,
                node_cuts: ranges.node_cuts,
                annex_cuts: ranges.annex_cuts,
            }),
            sidecar_annex_range: Some(ranges.sidecar),
            wrapper_rebuild: true,
        })
    }

    pub(crate) fn stamp_box_positive(
        self,
        annex: &mut NodeAnnexWriter<'_>,
        region: crate::node_region::NodeRegionId,
        key: crate::page_node_arena::PageBoxPositiveKey,
        wrapper_node: usize,
        wrapper_annex: usize,
    ) -> Option<()> {
        if !matches!(self.kind()?, NodeKind::HList | NodeKind::VList)
            || self.subtype() != 0
            || self.flags() != 0
        {
            return None;
        }
        annex.stamp_box_positive(
            key_from_record(self),
            region,
            key,
            wrapper_node,
            wrapper_annex,
        )
    }

    pub(crate) fn stamp_box_segment(
        self,
        annex: &mut NodeAnnexWriter<'_>,
        segment: crate::node_region::PageBoxSegment,
        migrations: Option<crate::page_node_arena::PageBoxMigrationKey>,
    ) -> Option<()> {
        if !matches!(self.kind()?, NodeKind::HList | NodeKind::VList)
            || self.subtype() != 0
            || self.flags() != 0
        {
            return None;
        }
        if let Some(key) = migrations {
            annex.view().box_migration_segments(key, segment)?;
        }
        annex.stamp_box_segment(key_from_record(self), segment, migrations)
    }

    pub(crate) fn unset_width(self, annex: NodeAnnexView<'_>) -> Option<Scaled> {
        (self.kind()? == NodeKind::Unset && self.subtype() == 0 && self.flags() & !0x1f_ffff == 0)
            .then(|| {
            annex.inspect_fixed(key_from_record::<UnsetPayload>(self), 15, |payload| {
                payload.get(11).copied().map(decode_scaled)
            })
        })?
    }

    pub(crate) fn math_boundary(self) -> Option<(bool, Scaled)> {
        let kind = self.kind()?;
        (matches!(kind, NodeKind::MathOn | NodeKind::MathOff)
            && self.subtype() == 0
            && self.flags() == 0
            && self.words()[1..].iter().all(|word| *word == 0))
        .then(|| (kind == NodeKind::MathOn, decode_scaled(self.words()[0])))
    }

    pub(crate) fn direction(self) -> Option<crate::node::Direction> {
        (self.kind()? == NodeKind::Direction
            && self.flags() == 0
            && self.words().iter().all(|word| *word == 0))
        .then(|| {
            Some(match self.subtype() {
                0 => crate::node::Direction::BeginL,
                1 => crate::node::Direction::EndL,
                2 => crate::node::Direction::BeginR,
                3 => crate::node::Direction::EndR,
                4 => crate::node::Direction::BeginM,
                5 => crate::node::Direction::EndM,
                _ => return None,
            })
        })?
    }

    pub(crate) fn pdf_image_width(self) -> Option<Scaled> {
        (self.kind()? == NodeKind::Whatsit
            && matches!(self.subtype(), 21 | 22)
            && self.flags() == 0
            && self.words()[4..].iter().all(|word| *word == 0))
        .then(|| decode_scaled(self.words()[1]))
    }

    pub(crate) fn glue_spec_kind(self, annex: NodeAnnexView<'_>) -> Option<(GlueSpec, GlueKind)> {
        if self.kind()? != NodeKind::Glue {
            return None;
        }
        let kind = decode_glue_kind(self.subtype())?;
        let glue = match self.flags() & 3 {
            0 | 1 => self.words()[..4].try_into().ok()?,
            2 | 3 => {
                annex.inspect_fixed(key_from_record::<LeaderBoxPayload>(self), 32, |payload| {
                    let mut words = [0; 4];
                    for (index, word) in words.iter_mut().enumerate() {
                        *word = *payload.get(index + 1)?;
                    }
                    Some(words)
                })?
            }
            _ => return None,
        };
        let spec = decode_glue(glue)?;
        if self.glue_origin()? == crate::node::GlueSpecOrigin::SharedZero && spec != GlueSpec::ZERO
        {
            return None;
        }
        Some((spec, kind))
    }

    pub(crate) fn glue_origin(self) -> Option<crate::node::GlueSpecOrigin> {
        (self.kind()? == NodeKind::Glue).then(|| {
            if self.flags() & 0x20 != 0 {
                crate::node::GlueSpecOrigin::SharedZero
            } else {
                crate::node::GlueSpecOrigin::Owned
            }
        })
    }

    pub(crate) fn glue_leader(
        self,
        annex: NodeAnnexView<'_>,
    ) -> Option<Option<LeaderPayload<PageListId>>> {
        if self.kind()? != NodeKind::Glue {
            return None;
        }
        let words = self.words();
        Some(match self.flags() & 3 {
            0 => None,
            1 => Some(LeaderPayload::Rule {
                width: (self.flags() & 4 != 0).then(|| decode_scaled(words[4])),
                height: (self.flags() & 8 != 0).then(|| decode_scaled(words[5])),
                depth: (self.flags() & 16 != 0).then(|| decode_scaled(words[6])),
            }),
            leader @ (2 | 3) => {
                let payload =
                    annex.resolve_fixed_array::<LeaderBoxPayload, 32>(key_from_record(self))?;
                let boxed = decode_box_payload(&payload[4..])?;
                Some(if leader == 2 {
                    LeaderPayload::HList(boxed)
                } else {
                    LeaderPayload::VList(boxed)
                })
            }
            _ => return None,
        })
    }

    pub(crate) fn is_math_on(self) -> bool {
        self.kind() == Some(NodeKind::MathOn)
    }

    pub(crate) fn is_math_off(self) -> bool {
        self.kind() == Some(NodeKind::MathOff)
    }

    pub(crate) fn language(self) -> Option<(u8, u8, u8)> {
        let words = self.words();
        (self.kind()? == NodeKind::Whatsit
            && self.subtype() == 26
            && self.flags() == 0
            && words[0] >> 24 == 0
            && words[1..].iter().all(|word| *word == 0))
        .then_some((
            words[0] as u8,
            (words[0] >> 8) as u8,
            (words[0] >> 16) as u8,
        ))
    }

    pub(crate) fn math_list(self, annex: NodeAnnexView<'_>) -> Option<MathListNode<PageListId>> {
        if self.kind()? != NodeKind::MathList || self.subtype() != 0 || self.flags() > 1 {
            return None;
        }
        let payload = annex.resolve_fixed_array::<ListPayload, 10>(key_from_record(self))?;
        let mut cursor = 0;
        Some(MathListNode {
            display: self.flags() == 1,
            content: decode_page_list(&payload, &mut cursor)?,
        })
    }

    pub(crate) fn discretionary(
        self,
        annex: NodeAnnexView<'_>,
    ) -> Option<(DiscKind, PageListId, PageListId, PageListId, u8)> {
        if self.kind()? != NodeKind::Disc || self.flags() > u8::MAX as u32 {
            return None;
        }
        annex.inspect_fixed(key_from_record::<DiscPayload>(self), 30, |payload| {
            let list = |start: usize| {
                let mut words = [0; 10];
                for (index, word) in words.iter_mut().enumerate() {
                    *word = *payload.get(start + index + 1)?;
                }
                PageListId::from_words(words)
            };
            Some((
                decode_disc_kind(self.subtype())?,
                list(0)?,
                list(10)?,
                list(20)?,
                self.flags() as u8,
            ))
        })
    }

    pub(crate) fn discretionary_break(
        self,
        annex: NodeAnnexView<'_>,
    ) -> Option<(DiscKind, PageListId, PageListId)> {
        if self.kind()? != NodeKind::Disc || self.flags() > u8::MAX as u32 {
            return None;
        }
        annex.inspect_fixed(key_from_record::<DiscPayload>(self), 30, |payload| {
            let list = |start: usize| {
                let mut words = [0; 10];
                for (index, word) in words.iter_mut().enumerate() {
                    *word = *payload.get(start + index + 1)?;
                }
                PageListId::from_words(words)
            };
            Some((decode_disc_kind(self.subtype())?, list(0)?, list(10)?))
        })
    }

    pub(crate) fn discretionary_replace(self, annex: NodeAnnexView<'_>) -> Option<PageListId> {
        if self.kind()? != NodeKind::Disc || self.flags() > u8::MAX as u32 {
            return None;
        }
        annex.inspect_fixed(key_from_record::<DiscPayload>(self), 30, |payload| {
            let mut words = [0; 10];
            for (index, word) in words.iter_mut().enumerate() {
                *word = *payload.get(index + 21)?;
            }
            PageListId::from_words(words)
        })
    }

    pub(crate) fn visit_ligature_source(
        self,
        annex: NodeAnnexView<'_>,
        mut visit: impl FnMut(char, OriginId),
    ) -> Option<FontId> {
        if self.kind()? != NodeKind::Lig || self.subtype() != 0 || self.flags() & !3 != 0 {
            return None;
        }
        let payload = annex.resolve_fixed_array::<LigaturePayload, 12>(key_from_record(self))?;
        let mut cursor = 0;
        let font = decode_font(&payload, &mut cursor)?;
        char::from_u32(*payload.get(cursor)?)?;
        cursor += 1;
        let source = AnnexKey::<LigatureSource>::from_words(take_words(&payload, &mut cursor)?);
        let mut header = [0; 2];
        let mut index = 0_usize;
        let mut pair = [0; 2];
        let mut valid = true;
        annex.visit_span(source, |word| {
            if index < 2 {
                header[index] = word;
            } else {
                pair[(index - 2) % 2] = word;
                if (index - 2) % 2 == 1 {
                    let Some(ch) = char::from_u32(pair[0]) else {
                        valid = false;
                        index += 1;
                        return;
                    };
                    if header[1] > 1 || (header[1] == 1 && pair[1] != 0) {
                        valid = false;
                    }
                    visit(ch, OriginId::from_raw(pair[1]));
                }
            }
            index += 1;
        })?;
        (valid && header[0] as usize * 2 + 2 == index).then_some(font)
    }

    pub(super) fn with_key<Kind>(
        kind: NodeKind,
        subtype: u8,
        flags: u32,
        key: AnnexKey<Kind>,
    ) -> Self {
        let key = key_words(key);
        Self::new(kind, subtype, flags, key)
    }

    pub(crate) fn encode_owned(node: Node, annex: &mut NodeAnnexWriter<'_>) -> Self {
        match node {
            Node::Char { font, ch, origin } => {
                let font = font.words();
                Self::new(
                    NodeKind::Char,
                    0,
                    0,
                    [
                        font[0],
                        font[1],
                        font[2],
                        font[3],
                        ch as u32,
                        origin.raw(),
                        0,
                    ],
                )
            }
            Node::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
            } => {
                assert!(
                    origins.is_empty() || origins.len() == orig.len(),
                    "ligature origin rows are empty or parallel to source characters"
                );
                let mut source = Vec::with_capacity(2 + orig.len() * 2);
                source.push(u32::try_from(orig.len()).expect("ligature source length fits u32"));
                source.push(bool_word(origins.is_empty()));
                for (index, ch) in orig.into_iter().enumerate() {
                    source.push(ch as u32);
                    source.push(origins.get(index).copied().unwrap_or_default().raw());
                }
                let source = annex.append_span::<LigatureSource>(&source);
                let mut payload = Vec::with_capacity(12);
                encode_font(&mut payload, font);
                payload.push(ch as u32);
                append_words(&mut payload, source.words());
                let key = annex.append_fixed::<LigaturePayload>(&payload);
                Self::with_key(
                    NodeKind::Lig,
                    0,
                    bool_word(left_hit) | (bool_word(right_hit) << 1),
                    key,
                )
            }
            Node::Kern { amount, kind } => Self::new(
                NodeKind::Kern,
                encode_kern_kind(kind),
                0,
                [scaled_word(amount), 0, 0, 0, 0, 0, 0],
            ),
            Node::MarginKern {
                amount,
                side,
                font,
                ch,
            } => {
                let font = font.words();
                Self::new(
                    NodeKind::MarginKern,
                    encode_margin_side(side),
                    0,
                    [
                        scaled_word(amount),
                        font[0],
                        font[1],
                        font[2],
                        font[3],
                        u32::from(ch),
                        0,
                    ],
                )
            }
            Node::Glue {
                spec,
                kind,
                origin,
                leader,
            } => {
                assert!(
                    origin != crate::node::GlueSpecOrigin::SharedZero || spec == GlueSpec::ZERO,
                    "shared zero_glue origin requires the zero specification"
                );
                let glue = encode_glue(spec);
                let origin_flag = if origin == crate::node::GlueSpecOrigin::SharedZero {
                    0x20
                } else {
                    0
                };
                match leader {
                    None => Self::new(
                        NodeKind::Glue,
                        encode_glue_kind(kind),
                        origin_flag,
                        [glue[0], glue[1], glue[2], glue[3], 0, 0, 0],
                    ),
                    Some(LeaderPayload::Rule {
                        width,
                        height,
                        depth,
                    }) => {
                        let flags = origin_flag
                            | 1
                            | (bool_word(width.is_some()) << 2)
                            | (bool_word(height.is_some()) << 3)
                            | (bool_word(depth.is_some()) << 4);
                        Self::new(
                            NodeKind::Glue,
                            encode_glue_kind(kind),
                            flags,
                            [
                                glue[0],
                                glue[1],
                                glue[2],
                                glue[3],
                                width.map_or(0, scaled_word),
                                height.map_or(0, scaled_word),
                                depth.map_or(0, scaled_word),
                            ],
                        )
                    }
                    Some(LeaderPayload::HList(boxed) | LeaderPayload::VList(boxed)) => {
                        let is_vertical = matches!(leader, Some(LeaderPayload::VList(_)));
                        let mut payload = Vec::with_capacity(32);
                        append_words(&mut payload, glue);
                        payload.extend(encode_box_payload(boxed));
                        let key = annex.append_fixed::<LeaderBoxPayload>(&payload);
                        Self::with_key(
                            NodeKind::Glue,
                            encode_glue_kind(kind),
                            origin_flag | if is_vertical { 3 } else { 2 },
                            key,
                        )
                    }
                }
            }
            Node::Penalty(value) => {
                Self::new(NodeKind::Penalty, 0, 0, [value as u32, 0, 0, 0, 0, 0, 0])
            }
            Node::Rule {
                width,
                height,
                depth,
            } => Self::new(
                NodeKind::Rule,
                0,
                bool_word(width.is_some())
                    | (bool_word(height.is_some()) << 1)
                    | (bool_word(depth.is_some()) << 2),
                [
                    width.map_or(0, scaled_word),
                    height.map_or(0, scaled_word),
                    depth.map_or(0, scaled_word),
                    0,
                    0,
                    0,
                    0,
                ],
            ),
            Node::HList(value) | Node::VList(value) => {
                let vertical = matches!(node, Node::VList(_));
                let key = annex.append_fixed::<BoxPayload>(&encode_standalone_box_payload(value));
                Self::with_key(
                    if vertical {
                        NodeKind::VList
                    } else {
                        NodeKind::HList
                    },
                    0,
                    0,
                    key,
                )
            }
            Node::Unset(value) => {
                let mut payload = Vec::with_capacity(15);
                encode_page_list(&mut payload, value.children);
                payload.extend([
                    scaled_word(value.width),
                    scaled_word(value.height),
                    scaled_word(value.depth),
                    scaled_word(value.stretch),
                    scaled_word(value.shrink),
                ]);
                let flags = u32::from(value.span_count)
                    | ((value.stretch_order as u32) << 16)
                    | ((value.shrink_order as u32) << 18)
                    | (u32::from(matches!(value.kind, UnsetKind::VBox)) << 20);
                Self::with_key(
                    NodeKind::Unset,
                    0,
                    flags,
                    annex.append_fixed::<UnsetPayload>(&payload),
                )
            }
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => {
                let mut payload = Vec::with_capacity(30);
                encode_page_list(&mut payload, pre);
                encode_page_list(&mut payload, post);
                encode_page_list(&mut payload, replace);
                Self::with_key(
                    NodeKind::Disc,
                    encode_disc_kind(kind),
                    u32::from(physical_replace_count),
                    annex.append_fixed::<DiscPayload>(&payload),
                )
            }
            Node::Mark { class, tokens } => {
                let token = tokens.coordinates();
                Self::new(
                    NodeKind::Mark,
                    0,
                    u32::from(class),
                    [
                        token[0], token[1], token[2], token[3], token[4], token[5], 0,
                    ],
                )
            }
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => {
                let mut payload = Vec::with_capacity(17);
                encode_page_list(&mut payload, content);
                append_words(&mut payload, encode_glue(split_top_skip));
                payload.extend([
                    scaled_word(size),
                    scaled_word(split_max_depth),
                    floating_penalty as u32,
                ]);
                Self::with_key(
                    NodeKind::Ins,
                    0,
                    u32::from(class),
                    annex.append_fixed::<InsertionPayload>(&payload),
                )
            }
            Node::Whatsit(value) => encode_whatsit(value, annex),
            Node::MathOn(value) => Self::new(
                NodeKind::MathOn,
                0,
                0,
                [scaled_word(value), 0, 0, 0, 0, 0, 0],
            ),
            Node::MathOff(value) => Self::new(
                NodeKind::MathOff,
                0,
                0,
                [scaled_word(value), 0, 0, 0, 0, 0, 0],
            ),
            Node::Direction(value) => Self::new(NodeKind::Direction, value as u8, 0, [0; 7]),
            Node::MathNoad(value) => {
                let mut payload = Vec::with_capacity(36);
                encode_noad_kind(&mut payload, value.kind);
                encode_math_field(&mut payload, value.nucleus);
                encode_math_field(&mut payload, value.subscript);
                encode_math_field(&mut payload, value.superscript);
                debug_assert_eq!(payload.len(), 36);
                Self::with_key(
                    NodeKind::MathNoad,
                    0,
                    0,
                    annex.append_fixed::<MathNoadPayload>(&payload),
                )
            }
            Node::FractionNoad(value) => {
                let mut payload = Vec::with_capacity(23);
                encode_page_list(&mut payload, value.numerator);
                encode_page_list(&mut payload, value.denominator);
                let (thickness, default_thickness) = match value.thickness {
                    FractionThickness::Default => (0, true),
                    FractionThickness::Explicit(value) => (scaled_word(value), false),
                };
                payload.push(thickness);
                payload.push(value.left_delimiter.unwrap_or_default());
                payload.push(value.right_delimiter.unwrap_or_default());
                let flags = bool_word(value.left_delimiter.is_some())
                    | (bool_word(value.right_delimiter.is_some()) << 1)
                    | (bool_word(default_thickness) << 2);
                Self::with_key(
                    NodeKind::FractionNoad,
                    0,
                    flags,
                    annex.append_fixed::<FractionPayload>(&payload),
                )
            }
            Node::MathStyle(style) => {
                Self::new(NodeKind::MathStyle, encode_math_style(style), 0, [0; 7])
            }
            Node::MathChoice(value) => {
                let mut payload = Vec::with_capacity(40);
                encode_page_list(&mut payload, value.display);
                encode_page_list(&mut payload, value.text);
                encode_page_list(&mut payload, value.script);
                encode_page_list(&mut payload, value.script_script);
                Self::with_key(
                    NodeKind::MathChoice,
                    0,
                    0,
                    annex.append_fixed::<MathChoicePayload>(&payload),
                )
            }
            Node::MathList(value) => {
                let mut payload = Vec::with_capacity(10);
                encode_page_list(&mut payload, value.content);
                Self::with_key(
                    NodeKind::MathList,
                    0,
                    bool_word(value.display),
                    annex.append_fixed::<ListPayload>(&payload),
                )
            }
            Node::Nonscript => Self::new(NodeKind::Nonscript, 0, 0, [0; 7]),
            Node::Adjust(value) => {
                let mut payload = Vec::with_capacity(10);
                encode_page_list(&mut payload, value.content);
                Self::with_key(
                    NodeKind::Adjust,
                    0,
                    bool_word(value.pre),
                    annex.append_fixed::<ListPayload>(&payload),
                )
            }
        }
    }

    /// Encodes the compact character record without constructing an owned
    /// compatibility node first.
    pub(crate) fn encode_char(font: FontId, ch: char, origin: OriginId) -> Self {
        let font = font.words();
        Self::new(
            NodeKind::Char,
            0,
            0,
            [
                font[0],
                font[1],
                font[2],
                font[3],
                ch as u32,
                origin.raw(),
                0,
            ],
        )
    }

    /// Encodes a ligature's variable source span directly into its annex.
    /// The source iterator is exact, which lets the fixed two-word source
    /// header be emitted without staging a temporary `Vec<u32>`.
    #[allow(clippy::too_many_arguments)] // Direct encoding keeps all fixed payload fields at the destination boundary.
    pub(crate) fn encode_ligature(
        font: FontId,
        ch: char,
        source_len: usize,
        origins_empty: bool,
        left_hit: bool,
        right_hit: bool,
        source: &mut dyn ExactSizeIterator<Item = (char, OriginId)>,
        annex: &mut NodeAnnexWriter<'_>,
    ) -> Self {
        assert_eq!(
            source.len(),
            source_len,
            "ligature source length remains exact"
        );
        let source = annex.append_span_iter::<LigatureSource>(
            std::iter::once(u32::try_from(source_len).expect("ligature source length fits u32"))
                .chain(std::iter::once(bool_word(origins_empty)))
                .chain(source.flat_map(|(ch, origin)| [ch as u32, origin.raw()])),
        );
        let font = font.words();
        let mut payload = [0_u32; 12];
        payload[..4].copy_from_slice(&font);
        payload[4] = ch as u32;
        payload[5..].copy_from_slice(&source.words());
        let key = annex.append_fixed::<LigaturePayload>(&payload);
        Self::with_key(
            NodeKind::Lig,
            0,
            bool_word(left_hit) | (bool_word(right_hit) << 1),
            key,
        )
    }

    /// Encodes a compact kern record without constructing an owned node.
    pub(crate) fn encode_kern(amount: Scaled, kind: KernKind) -> Self {
        Self::new(
            NodeKind::Kern,
            encode_kern_kind(kind),
            0,
            [scaled_word(amount), 0, 0, 0, 0, 0, 0],
        )
    }

    pub(crate) fn decode_owned(self, annex: NodeAnnexView<'_>) -> Option<Node> {
        let kind = self.kind()?;
        let subtype = self.subtype();
        let flags = self.flags();
        let words = self.words();
        match kind {
            NodeKind::Char if subtype == 0 && flags == 0 && words[6] == 0 => Some(Node::Char {
                font: FontId::from_words(words[..4].try_into().ok()?)?,
                ch: char::from_u32(words[4])?,
                origin: OriginId::from_raw(words[5]),
            }),
            NodeKind::Lig if subtype == 0 && flags & !3 == 0 => {
                let payload =
                    annex.resolve_fixed_array::<LigaturePayload, 12>(key_from_record(self))?;
                let mut cursor = 0;
                let font = decode_font(&payload, &mut cursor)?;
                let ch = char::from_u32(*payload.get(cursor)?)?;
                cursor += 1;
                let source =
                    AnnexKey::<LigatureSource>::from_words(take_words(&payload, &mut cursor)?);
                let source = annex.detach_span(source)?;
                let count = *source.first()? as usize;
                let origins_empty = decode_bool(*source.get(1)?)?;
                if source.len() != 2 + count * 2 {
                    return None;
                }
                let mut orig = Vec::with_capacity(count);
                let mut origins = (!origins_empty).then(|| Vec::with_capacity(count));
                for pair in source[2..].as_chunks::<2>().0 {
                    orig.push(char::from_u32(pair[0])?);
                    if let Some(origins) = &mut origins {
                        origins.push(OriginId::from_raw(pair[1]));
                    } else if pair[1] != 0 {
                        return None;
                    }
                }
                Some(Node::Lig {
                    font,
                    ch,
                    orig,
                    left_hit: flags & 1 != 0,
                    right_hit: flags & 2 != 0,
                    origins: origins.unwrap_or_default(),
                })
            }
            NodeKind::Kern if flags == 0 && words[1..].iter().all(|word| *word == 0) => {
                Some(Node::Kern {
                    amount: decode_scaled(words[0]),
                    kind: decode_kern_kind(subtype)?,
                })
            }
            NodeKind::MarginKern if flags == 0 && words[6] == 0 && words[5] <= u8::MAX as u32 => {
                Some(Node::MarginKern {
                    amount: decode_scaled(words[0]),
                    side: decode_margin_side(subtype)?,
                    font: FontId::from_words(words[1..5].try_into().ok()?)?,
                    ch: words[5] as u8,
                })
            }
            NodeKind::Glue => {
                let kind = decode_glue_kind(subtype)?;
                let origin = self.glue_origin()?;
                let valid_origin = |spec: GlueSpec| {
                    origin != crate::node::GlueSpecOrigin::SharedZero || spec == GlueSpec::ZERO
                };
                match flags & 3 {
                    0 if flags & !0x20 == 0 && words[4..].iter().all(|word| *word == 0) => {
                        let spec = decode_glue(words[..4].try_into().ok()?)?;
                        if !valid_origin(spec) {
                            return None;
                        }
                        Some(Node::Glue {
                            spec,
                            kind,
                            origin,
                            leader: None,
                        })
                    }
                    1 if flags & !0x3d == 0 => {
                        let spec = decode_glue(words[..4].try_into().ok()?)?;
                        if !valid_origin(spec) {
                            return None;
                        }
                        Some(Node::Glue {
                            spec,
                            kind,
                            origin,
                            leader: Some(LeaderPayload::Rule {
                                width: (flags & 4 != 0).then(|| decode_scaled(words[4])),
                                height: (flags & 8 != 0).then(|| decode_scaled(words[5])),
                                depth: (flags & 16 != 0).then(|| decode_scaled(words[6])),
                            }),
                        })
                    }
                    leader @ (2 | 3) if flags & !0x20 == leader => {
                        let payload = annex
                            .resolve_fixed_array::<LeaderBoxPayload, 32>(key_from_record(self))?;
                        let spec = decode_glue(payload[..4].try_into().ok()?)?;
                        if !valid_origin(spec) {
                            return None;
                        }
                        let boxed = decode_box_payload(&payload[4..])?;
                        Some(Node::Glue {
                            spec,
                            kind,
                            origin,
                            leader: Some(if leader == 2 {
                                LeaderPayload::HList(boxed)
                            } else {
                                LeaderPayload::VList(boxed)
                            }),
                        })
                    }
                    _ => None,
                }
            }
            NodeKind::Penalty
                if subtype == 0 && flags == 0 && words[1..].iter().all(|word| *word == 0) =>
            {
                Some(Node::Penalty(words[0] as i32))
            }
            NodeKind::Rule
                if subtype == 0 && flags & !7 == 0 && words[3..].iter().all(|word| *word == 0) =>
            {
                Some(Node::Rule {
                    width: (flags & 1 != 0).then(|| decode_scaled(words[0])),
                    height: (flags & 2 != 0).then(|| decode_scaled(words[1])),
                    depth: (flags & 4 != 0).then(|| decode_scaled(words[2])),
                })
            }
            NodeKind::HList | NodeKind::VList if subtype == 0 && flags == 0 => {
                let payload = annex
                    .resolve_fixed_array::<BoxPayload, BOX_PAYLOAD_WORDS>(key_from_record(self))?;
                let boxed = decode_box_payload(&payload)?;
                Some(if kind == NodeKind::HList {
                    Node::HList(boxed)
                } else {
                    Node::VList(boxed)
                })
            }
            NodeKind::Unset if subtype == 0 => {
                let payload =
                    annex.resolve_fixed_array::<UnsetPayload, 15>(key_from_record(self))?;
                if flags & !0x1f_ffff != 0 {
                    return None;
                }
                let mut cursor = 0;
                let children = decode_page_list(&payload, &mut cursor)?;
                let values: [u32; 5] = take_words(&payload, &mut cursor)?;
                Some(Node::Unset(UnsetNode::new(UnsetNodeFields {
                    kind: if flags & (1 << 20) == 0 {
                        UnsetKind::HBox
                    } else {
                        UnsetKind::VBox
                    },
                    width: decode_scaled(values[0]),
                    height: decode_scaled(values[1]),
                    depth: decode_scaled(values[2]),
                    span_count: flags as u16,
                    stretch: decode_scaled(values[3]),
                    stretch_order: decode_order((flags >> 16) & 3)?,
                    shrink: decode_scaled(values[4]),
                    shrink_order: decode_order((flags >> 18) & 3)?,
                    children,
                })))
            }
            NodeKind::Disc if flags <= u8::MAX as u32 => {
                let payload =
                    annex.resolve_fixed_array::<DiscPayload, 30>(key_from_record(self))?;
                let mut cursor = 0;
                Some(Node::Disc {
                    kind: decode_disc_kind(subtype)?,
                    pre: decode_page_list(&payload, &mut cursor)?,
                    post: decode_page_list(&payload, &mut cursor)?,
                    replace: decode_page_list(&payload, &mut cursor)?,
                    physical_replace_count: flags as u8,
                })
            }
            NodeKind::Mark if subtype == 0 && flags <= u16::MAX as u32 && words[6] == 0 => {
                Some(Node::Mark {
                    class: flags as u16,
                    tokens: NodeTokenKey::from_coordinates(words[..6].try_into().ok()?),
                })
            }
            NodeKind::Ins if subtype == 0 && flags <= u16::MAX as u32 => {
                let payload =
                    annex.resolve_fixed_array::<InsertionPayload, 17>(key_from_record(self))?;
                let mut cursor = 0;
                let content = decode_page_list(&payload, &mut cursor)?;
                let split_top_skip = decode_glue(take_words(&payload, &mut cursor)?)?;
                let scalar: [u32; 3] = take_words(&payload, &mut cursor)?;
                Some(Node::Ins {
                    class: flags as u16,
                    size: decode_scaled(scalar[0]),
                    split_top_skip,
                    split_max_depth: decode_scaled(scalar[1]),
                    floating_penalty: scalar[2] as i32,
                    content,
                })
            }
            NodeKind::Whatsit => decode_whatsit(self, annex),
            NodeKind::MathOn | NodeKind::MathOff
                if subtype == 0 && flags == 0 && words[1..].iter().all(|word| *word == 0) =>
            {
                Some(if kind == NodeKind::MathOn {
                    Node::MathOn(decode_scaled(words[0]))
                } else {
                    Node::MathOff(decode_scaled(words[0]))
                })
            }
            NodeKind::Direction if flags == 0 && words.iter().all(|word| *word == 0) => {
                Some(Node::Direction(match subtype {
                    0 => crate::node::Direction::BeginL,
                    1 => crate::node::Direction::EndL,
                    2 => crate::node::Direction::BeginR,
                    3 => crate::node::Direction::EndR,
                    4 => crate::node::Direction::BeginM,
                    5 => crate::node::Direction::EndM,
                    _ => return None,
                }))
            }
            NodeKind::MathNoad if subtype == 0 && flags == 0 => {
                let payload =
                    annex.resolve_fixed_array::<MathNoadPayload, 36>(key_from_record(self))?;
                let mut cursor = 0;
                Some(Node::MathNoad(MathNoad {
                    kind: decode_noad_kind(&payload, &mut cursor)?,
                    nucleus: decode_math_field(&payload, &mut cursor)?,
                    subscript: decode_math_field(&payload, &mut cursor)?,
                    superscript: decode_math_field(&payload, &mut cursor)?,
                }))
            }
            NodeKind::FractionNoad if subtype == 0 && flags & !7 == 0 => {
                let payload =
                    annex.resolve_fixed_array::<FractionPayload, 23>(key_from_record(self))?;
                let mut cursor = 0;
                let numerator = decode_page_list(&payload, &mut cursor)?;
                let denominator = decode_page_list(&payload, &mut cursor)?;
                let thickness = *payload.get(cursor)?;
                cursor += 1;
                let delimiters: [u32; 2] = take_words(&payload, &mut cursor)?;
                Some(Node::FractionNoad(MathFraction {
                    numerator,
                    denominator,
                    thickness: if flags & 4 != 0 {
                        FractionThickness::Default
                    } else {
                        FractionThickness::Explicit(decode_scaled(thickness))
                    },
                    left_delimiter: (flags & 1 != 0).then_some(delimiters[0]),
                    right_delimiter: (flags & 2 != 0).then_some(delimiters[1]),
                }))
            }
            NodeKind::MathStyle if flags == 0 && words.iter().all(|word| *word == 0) => {
                Some(Node::MathStyle(decode_math_style(subtype)?))
            }
            NodeKind::MathChoice if subtype == 0 && flags == 0 => {
                let payload =
                    annex.resolve_fixed_array::<MathChoicePayload, 40>(key_from_record(self))?;
                let mut cursor = 0;
                Some(Node::MathChoice(MathChoice {
                    display: decode_page_list(&payload, &mut cursor)?,
                    text: decode_page_list(&payload, &mut cursor)?,
                    script: decode_page_list(&payload, &mut cursor)?,
                    script_script: decode_page_list(&payload, &mut cursor)?,
                }))
            }
            NodeKind::MathList if subtype == 0 && flags <= 1 => {
                let payload =
                    annex.resolve_fixed_array::<ListPayload, 10>(key_from_record(self))?;
                let mut cursor = 0;
                Some(Node::MathList(MathListNode {
                    display: flags == 1,
                    content: decode_page_list(&payload, &mut cursor)?,
                }))
            }
            NodeKind::Nonscript
                if subtype == 0 && flags == 0 && words.iter().all(|word| *word == 0) =>
            {
                Some(Node::Nonscript)
            }
            NodeKind::Adjust if subtype == 0 && flags <= 1 => {
                let payload =
                    annex.resolve_fixed_array::<ListPayload, 10>(key_from_record(self))?;
                let mut cursor = 0;
                Some(Node::Adjust(AdjustNode {
                    content: decode_page_list(&payload, &mut cursor)?,
                    pre: flags == 1,
                }))
            }
            _ => None,
        }
    }
}
