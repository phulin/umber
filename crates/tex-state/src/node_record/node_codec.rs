use super::*;

impl NodeRecord<PageMaterialLane> {
    pub(super) fn with_key<Kind>(
        kind: NodeKind,
        subtype: u8,
        flags: u32,
        key: AnnexKey<Kind>,
    ) -> Self {
        let key = key_words(key);
        Self::new(
            kind,
            subtype,
            flags,
            [key[0], key[1], key[2], key[3], key[4], key[5], 0],
        )
    }

    pub(crate) fn encode_owned(node: Node, annex: &mut NodeAnnexArena) -> Self {
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
                let mut payload = Vec::with_capacity(11);
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

    pub(crate) fn decode_owned(self, annex: &NodeAnnexArena) -> Option<Node> {
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
            NodeKind::Lig if subtype == 0 && flags & !3 == 0 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<LigaturePayload>(self))?;
                if payload.len() != 11 {
                    return None;
                }
                let mut cursor = 0;
                let font = decode_font(payload, &mut cursor)?;
                let ch = char::from_u32(*payload.get(cursor)?)?;
                cursor += 1;
                let source =
                    AnnexKey::<LigatureSource>::from_words(take_words(payload, &mut cursor)?);
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
                    leader @ (2 | 3) if flags == leader && words[6] == 0 => {
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
            NodeKind::HList | NodeKind::VList if subtype == 0 && flags == 0 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<BoxPayload>(self))?;
                let boxed = decode_box_payload(payload)?;
                Some(if kind == NodeKind::HList {
                    Node::HList(boxed)
                } else {
                    Node::VList(boxed)
                })
            }
            NodeKind::Unset if subtype == 0 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<UnsetPayload>(self))?;
                if payload.len() != 15 || flags & !0x1f_ffff != 0 {
                    return None;
                }
                let mut cursor = 0;
                let children = decode_page_list(payload, &mut cursor)?;
                let values: [u32; 5] = take_words(payload, &mut cursor)?;
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
            NodeKind::Disc if flags <= u8::MAX as u32 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<DiscPayload>(self))?;
                if payload.len() != 30 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::Disc {
                    kind: decode_disc_kind(subtype)?,
                    pre: decode_page_list(payload, &mut cursor)?,
                    post: decode_page_list(payload, &mut cursor)?,
                    replace: decode_page_list(payload, &mut cursor)?,
                    physical_replace_count: flags as u8,
                })
            }
            NodeKind::Mark if subtype == 0 && flags <= u16::MAX as u32 && words[6] == 0 => {
                Some(Node::Mark {
                    class: flags as u16,
                    tokens: NodeTokenKey::from_coordinates(words[..6].try_into().ok()?),
                })
            }
            NodeKind::Ins if subtype == 0 && flags <= u16::MAX as u32 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<InsertionPayload>(self))?;
                if payload.len() != 17 {
                    return None;
                }
                let mut cursor = 0;
                let content = decode_page_list(payload, &mut cursor)?;
                let split_top_skip = decode_glue(take_words(payload, &mut cursor)?)?;
                let scalar: [u32; 3] = take_words(payload, &mut cursor)?;
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
            NodeKind::MathNoad if subtype == 0 && flags == 0 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<MathNoadPayload>(self))?;
                if payload.len() != 36 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::MathNoad(MathNoad {
                    kind: decode_noad_kind(payload, &mut cursor)?,
                    nucleus: decode_math_field(payload, &mut cursor)?,
                    subscript: decode_math_field(payload, &mut cursor)?,
                    superscript: decode_math_field(payload, &mut cursor)?,
                }))
            }
            NodeKind::FractionNoad if subtype == 0 && flags & !7 == 0 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<FractionPayload>(self))?;
                if payload.len() != 23 {
                    return None;
                }
                let mut cursor = 0;
                let numerator = decode_page_list(payload, &mut cursor)?;
                let denominator = decode_page_list(payload, &mut cursor)?;
                let thickness = *payload.get(cursor)?;
                cursor += 1;
                let delimiters: [u32; 2] = take_words(payload, &mut cursor)?;
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
            NodeKind::MathChoice if subtype == 0 && flags == 0 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<MathChoicePayload>(self))?;
                if payload.len() != 40 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::MathChoice(MathChoice {
                    display: decode_page_list(payload, &mut cursor)?,
                    text: decode_page_list(payload, &mut cursor)?,
                    script: decode_page_list(payload, &mut cursor)?,
                    script_script: decode_page_list(payload, &mut cursor)?,
                }))
            }
            NodeKind::MathList if subtype == 0 && flags <= 1 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<ListPayload>(self))?;
                if payload.len() != 10 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::MathList(MathListNode {
                    display: flags == 1,
                    content: decode_page_list(payload, &mut cursor)?,
                }))
            }
            NodeKind::Nonscript
                if subtype == 0 && flags == 0 && words.iter().all(|word| *word == 0) =>
            {
                Some(Node::Nonscript)
            }
            NodeKind::Adjust if subtype == 0 && flags <= 1 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<ListPayload>(self))?;
                if payload.len() != 10 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::Adjust(AdjustNode {
                    content: decode_page_list(payload, &mut cursor)?,
                    pre: flags == 1,
                }))
            }
            _ => None,
        }
    }
}
