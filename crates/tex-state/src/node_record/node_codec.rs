use super::*;

impl NodeRecord<PageMaterialLane> {
    pub(crate) fn visit_node_lists(
        self,
        annex: NodeAnnexView<'_>,
        mut visit: impl FnMut(PageListId),
    ) -> Option<()> {
        fn child(words: &[u32], offset: usize) -> Option<PageListId> {
            PageListId::from_words(
                words
                    .get(offset..offset.checked_add(10)?)?
                    .try_into()
                    .ok()?,
            )
        }
        fn math_field(
            words: &[u32],
            offset: usize,
            visit: &mut impl FnMut(PageListId),
        ) -> Option<()> {
            match *words.get(offset)? {
                0..=2 => Some(()),
                3 | 4 => {
                    visit(child(words, offset + 1)?);
                    Some(())
                }
                _ => None,
            }
        }

        match self.kind()? {
            NodeKind::Glue if matches!(self.flags() & 3, 2 | 3) => {
                let payload =
                    annex.resolve_fixed_array::<LeaderBoxPayload, 32>(key_from_record(self))?;
                visit(child(&payload, 11)?);
                visit(child(&payload, 21)?);
            }
            NodeKind::HList | NodeKind::VList => {
                let payload = annex.resolve_fixed_array::<BoxPayload, 28>(key_from_record(self))?;
                visit(child(&payload, 7)?);
                visit(child(&payload, 17)?);
            }
            NodeKind::Unset => {
                let payload =
                    annex.resolve_fixed_array::<UnsetPayload, 15>(key_from_record(self))?;
                visit(child(&payload, 0)?);
            }
            NodeKind::Disc => {
                let payload =
                    annex.resolve_fixed_array::<DiscPayload, 30>(key_from_record(self))?;
                for offset in [0, 10, 20] {
                    visit(child(&payload, offset)?);
                }
            }
            NodeKind::Ins => {
                let payload =
                    annex.resolve_fixed_array::<InsertionPayload, 17>(key_from_record(self))?;
                visit(child(&payload, 0)?);
            }
            NodeKind::MathNoad => {
                let payload =
                    annex.resolve_fixed_array::<MathNoadPayload, 36>(key_from_record(self))?;
                for offset in [3, 14, 25] {
                    math_field(&payload, offset, &mut visit)?;
                }
            }
            NodeKind::FractionNoad => {
                let payload =
                    annex.resolve_fixed_array::<FractionPayload, 23>(key_from_record(self))?;
                visit(child(&payload, 0)?);
                visit(child(&payload, 10)?);
            }
            NodeKind::MathChoice => {
                let payload =
                    annex.resolve_fixed_array::<MathChoicePayload, 40>(key_from_record(self))?;
                for offset in [0, 10, 20, 30] {
                    visit(child(&payload, offset)?);
                }
            }
            NodeKind::MathList | NodeKind::Adjust => {
                let payload =
                    annex.resolve_fixed_array::<ListPayload, 10>(key_from_record(self))?;
                visit(child(&payload, 0)?);
            }
            _ => {}
        }
        Some(())
    }

    pub(crate) fn reencode_same_region(
        self,
        pool: &mut crate::fork_arena::ChunkPool<u32>,
        arena: &mut crate::fork_arena::ForkArena<u32, crate::node_region::NodeAnnexLane>,
        map_child: impl FnMut(PageListId) -> Option<PageListId>,
    ) -> Option<(Self, Option<usize>)> {
        let mut annex = NodeAnnexCopier::same_region(pool, arena);
        self.reencode_into(&mut annex, map_child)
    }

    pub(crate) fn reencode_between_regions(
        self,
        pool: &mut crate::fork_arena::ChunkPool<u32>,
        source: &crate::fork_arena::ForkArena<u32, crate::node_region::NodeAnnexLane>,
        destination: &mut crate::fork_arena::ForkArena<u32, crate::node_region::NodeAnnexLane>,
        map_child: impl FnMut(PageListId) -> Option<PageListId>,
    ) -> Option<(Self, Option<usize>)> {
        let mut annex = NodeAnnexCopier::between_regions(pool, source, destination);
        self.reencode_into(&mut annex, map_child)
    }

    fn reencode_into(
        self,
        annex: &mut NodeAnnexCopier<'_>,
        mut map_child: impl FnMut(PageListId) -> Option<PageListId>,
    ) -> Option<(Self, Option<usize>)> {
        fn rewrite_child(
            words: &mut [u32],
            offset: usize,
            map: &mut impl FnMut(PageListId) -> Option<PageListId>,
        ) -> Option<()> {
            let end = offset.checked_add(10)?;
            let child = PageListId::from_words(words.get(offset..end)?.try_into().ok()?)?;
            words
                .get_mut(offset..end)?
                .copy_from_slice(&map(child)?.words());
            Some(())
        }

        fn rewrite_math_field(
            words: &mut [u32],
            offset: usize,
            map: &mut impl FnMut(PageListId) -> Option<PageListId>,
        ) -> Option<()> {
            match *words.get(offset)? {
                0..=2 => Some(()),
                3 | 4 => rewrite_child(words, offset + 1, map),
                _ => None,
            }
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
            NodeKind::Glue if matches!(flags & 3, 2 | 3) => {
                let mut payload =
                    annex.resolve_fixed_array::<LeaderBoxPayload, 32>(key_from_record(self))?;
                rewrite_child(&mut payload, 11, &mut map_child)?;
                rewrite_child(&mut payload, 21, &mut map_child)?;
                let key = annex.append_fixed::<LeaderBoxPayload>(&payload);
                Some((
                    Self::with_key(kind, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            NodeKind::HList | NodeKind::VList => {
                let mut payload =
                    annex.resolve_fixed_array::<BoxPayload, 28>(key_from_record(self))?;
                rewrite_child(&mut payload, 7, &mut map_child)?;
                rewrite_child(&mut payload, 17, &mut map_child)?;
                let key = annex.append_fixed::<BoxPayload>(&payload);
                Some((
                    Self::with_key(kind, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            NodeKind::Unset => {
                let mut payload =
                    annex.resolve_fixed_array::<UnsetPayload, 15>(key_from_record(self))?;
                rewrite_child(&mut payload, 0, &mut map_child)?;
                let key = annex.append_fixed::<UnsetPayload>(&payload);
                Some((
                    Self::with_key(kind, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            NodeKind::Disc => {
                let mut payload =
                    annex.resolve_fixed_array::<DiscPayload, 30>(key_from_record(self))?;
                rewrite_child(&mut payload, 0, &mut map_child)?;
                rewrite_child(&mut payload, 10, &mut map_child)?;
                rewrite_child(&mut payload, 20, &mut map_child)?;
                let key = annex.append_fixed::<DiscPayload>(&payload);
                Some((
                    Self::with_key(kind, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            NodeKind::Ins => {
                let mut payload =
                    annex.resolve_fixed_array::<InsertionPayload, 17>(key_from_record(self))?;
                rewrite_child(&mut payload, 0, &mut map_child)?;
                let key = annex.append_fixed::<InsertionPayload>(&payload);
                Some((
                    Self::with_key(kind, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            NodeKind::MathNoad => {
                let mut payload =
                    annex.resolve_fixed_array::<MathNoadPayload, 36>(key_from_record(self))?;
                rewrite_math_field(&mut payload, 3, &mut map_child)?;
                rewrite_math_field(&mut payload, 14, &mut map_child)?;
                rewrite_math_field(&mut payload, 25, &mut map_child)?;
                let key = annex.append_fixed::<MathNoadPayload>(&payload);
                Some((
                    Self::with_key(kind, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            NodeKind::FractionNoad => {
                let mut payload =
                    annex.resolve_fixed_array::<FractionPayload, 23>(key_from_record(self))?;
                rewrite_child(&mut payload, 0, &mut map_child)?;
                rewrite_child(&mut payload, 10, &mut map_child)?;
                let key = annex.append_fixed::<FractionPayload>(&payload);
                Some((
                    Self::with_key(kind, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            NodeKind::MathChoice => {
                let mut payload =
                    annex.resolve_fixed_array::<MathChoicePayload, 40>(key_from_record(self))?;
                for offset in [0, 10, 20, 30] {
                    rewrite_child(&mut payload, offset, &mut map_child)?;
                }
                let key = annex.append_fixed::<MathChoicePayload>(&payload);
                Some((
                    Self::with_key(kind, subtype, flags, key),
                    annex.dependency_floor(),
                ))
            }
            NodeKind::MathList | NodeKind::Adjust => {
                let mut payload =
                    annex.resolve_fixed_array::<ListPayload, 10>(key_from_record(self))?;
                rewrite_child(&mut payload, 0, &mut map_child)?;
                let key = annex.append_fixed::<ListPayload>(&payload);
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
                annex.inspect_fixed(key_from_record::<BoxPayload>(self), 28, |payload| {
                    payload.get(1).copied().map(decode_scaled)
                })
            })?
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
        Some((decode_glue(glue)?, kind))
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
            Node::Glue { spec, kind, leader } => {
                let glue = encode_glue(spec);
                match leader {
                    None => Self::new(
                        NodeKind::Glue,
                        encode_glue_kind(kind),
                        0,
                        [glue[0], glue[1], glue[2], glue[3], 0, 0, 0],
                    ),
                    Some(LeaderPayload::Rule {
                        width,
                        height,
                        depth,
                    }) => {
                        let flags = 1
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
                            if is_vertical { 3 } else { 2 },
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
                let key = annex.append_fixed::<BoxPayload>(&encode_box_payload(value));
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
                    annex.resolve_fixed_shared(key_from_record::<LigaturePayload>(self))?;
                if payload.len() != 12 {
                    return None;
                }
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
                for pair in source[2..].chunks_exact(2) {
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
                match flags & 3 {
                    0 if flags == 0 && words[4..].iter().all(|word| *word == 0) => {
                        Some(Node::Glue {
                            spec: decode_glue(words[..4].try_into().ok()?)?,
                            kind,
                            leader: None,
                        })
                    }
                    1 if flags & !0x1d == 0 => Some(Node::Glue {
                        spec: decode_glue(words[..4].try_into().ok()?)?,
                        kind,
                        leader: Some(LeaderPayload::Rule {
                            width: (flags & 4 != 0).then(|| decode_scaled(words[4])),
                            height: (flags & 8 != 0).then(|| decode_scaled(words[5])),
                            depth: (flags & 16 != 0).then(|| decode_scaled(words[6])),
                        }),
                    }),
                    leader @ (2 | 3) if flags == leader => {
                        let payload = annex
                            .resolve_fixed_shared(key_from_record::<LeaderBoxPayload>(self))?;
                        if payload.len() != 32 {
                            return None;
                        }
                        let spec = decode_glue(payload[..4].try_into().ok()?)?;
                        let boxed = decode_box_payload(&payload[4..])?;
                        Some(Node::Glue {
                            spec,
                            kind,
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
                let payload = annex.resolve_fixed_shared(key_from_record::<BoxPayload>(self))?;
                let boxed = decode_box_payload(&payload)?;
                Some(if kind == NodeKind::HList {
                    Node::HList(boxed)
                } else {
                    Node::VList(boxed)
                })
            }
            NodeKind::Unset if subtype == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<UnsetPayload>(self))?;
                if payload.len() != 15 || flags & !0x1f_ffff != 0 {
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
                let payload = annex.resolve_fixed_shared(key_from_record::<DiscPayload>(self))?;
                if payload.len() != 30 {
                    return None;
                }
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
                    annex.resolve_fixed_shared(key_from_record::<InsertionPayload>(self))?;
                if payload.len() != 17 {
                    return None;
                }
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
                    annex.resolve_fixed_shared(key_from_record::<MathNoadPayload>(self))?;
                if payload.len() != 36 {
                    return None;
                }
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
                    annex.resolve_fixed_shared(key_from_record::<FractionPayload>(self))?;
                if payload.len() != 23 {
                    return None;
                }
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
                    annex.resolve_fixed_shared(key_from_record::<MathChoicePayload>(self))?;
                if payload.len() != 40 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::MathChoice(MathChoice {
                    display: decode_page_list(&payload, &mut cursor)?,
                    text: decode_page_list(&payload, &mut cursor)?,
                    script: decode_page_list(&payload, &mut cursor)?,
                    script_script: decode_page_list(&payload, &mut cursor)?,
                }))
            }
            NodeKind::MathList if subtype == 0 && flags <= 1 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<ListPayload>(self))?;
                if payload.len() != 10 {
                    return None;
                }
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
                let payload = annex.resolve_fixed_shared(key_from_record::<ListPayload>(self))?;
                if payload.len() != 10 {
                    return None;
                }
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
