use super::*;

/// Incremental decoder for the canonical page artifact wire layout.
///
/// Metadata and effects are retained, but the root list is decoded one direct
/// child at a time so replay never constructs an owned whole-page node tree.
pub(crate) struct V10PageDecoder<'a> {
    pub(crate) page: PageArtifact,
    pub(crate) root_vertical: bool,
    reader: Reader<'a>,
    remaining: usize,
    font_ids: std::collections::BTreeMap<u32, bool>,
}

impl<'a> V10PageDecoder<'a> {
    pub(crate) fn new(bytes: &'a [u8], limits: ArtifactCodecLimits) -> Result<Self, ParseError> {
        if bytes.len() > limits.max_bytes {
            return Err(ParseError::LimitExceeded {
                kind: CodecLimitKind::Bytes,
                actual: bytes.len(),
                limit: limits.max_bytes,
            });
        }
        let mut scan = Reader::new(bytes, limits);
        let (job, fonts, counts) = scan.header()?;
        let root_start = scan.offset;
        scan.skip_node()?;
        let effects = scan.effects()?;
        let math_events = scan.math_events()?;
        scan.finish()?;

        let mut reader = Reader::new_at(bytes, limits, root_start);
        let tag = reader.u8()?;
        let root_vertical = match tag {
            wire::node::HLIST => false,
            wire::node::VLIST => true,
            _ => {
                return Err(ParseError::Validation(
                    crate::ArtifactValidationError::RootNotBox,
                ));
            }
        };
        let fields = reader.box_fields()?;
        let remaining = reader.collection_len(5)?;
        let root = if root_vertical {
            PageNode::VList(fields.finish(Vec::new()))
        } else {
            PageNode::HList(fields.finish(Vec::new()))
        };
        let page = UnvalidatedPageArtifact {
            job,
            fonts,
            counts,
            root,
            effects,
            math_events,
        }
        .validate()?;
        let font_ids = page
            .fonts
            .iter()
            .map(|font| (font.font_id, font.opentype.is_some()))
            .collect();
        Ok(Self {
            page,
            root_vertical,
            reader,
            remaining,
            font_ids,
        })
    }

    pub(crate) fn stream_children(&mut self) -> V10NodeListSlice<'_, 'a> {
        let remaining = std::mem::take(&mut self.remaining);
        V10NodeListSlice {
            bytes: self.reader.bytes,
            start: self.reader.offset,
            count: remaining,
            depth: 2,
            limits: self.reader.limits,
            font_ids: &self.font_ids,
            effects_len: self.page.effects.len(),
        }
    }
}

pub(crate) struct V10NodeListReader<'r, 'a> {
    reader: Reader<'a>,
    remaining: usize,
    depth: usize,
    font_ids: &'r std::collections::BTreeMap<u32, bool>,
    effects_len: usize,
}

pub(crate) enum V10StreamNode<'r, 'a> {
    Char {
        font_id: u32,
        ch: u32,
        width: Scaled,
    },
    Kern(Scaled),
    Glue {
        spec: GlueSpec,
        kind: GlueKind,
        leader: V10StreamLeader<'r, 'a>,
    },
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    Box {
        vertical: bool,
        fields: BoxNode,
        children: V10NodeListSlice<'r, 'a>,
    },
    WhatsitAnchor(u32),
    Math(Scaled),
    Ignored(Vec<V10NodeListSlice<'r, 'a>>),
}

pub(crate) enum V10StreamLeader<'r, 'a> {
    None,
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    Box {
        vertical: bool,
        fields: BoxNode,
        children: V10NodeListSlice<'r, 'a>,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct V10NodeListSlice<'r, 'a> {
    bytes: &'a [u8],
    start: usize,
    count: usize,
    depth: usize,
    limits: ArtifactCodecLimits,
    font_ids: &'r std::collections::BTreeMap<u32, bool>,
    effects_len: usize,
}

impl<'r, 'a> V10NodeListSlice<'r, 'a> {
    pub(crate) fn reader(self) -> V10NodeListReader<'r, 'a> {
        V10NodeListReader {
            reader: Reader::new_at(self.bytes, self.limits, self.start),
            remaining: self.count,
            depth: self.depth,
            font_ids: self.font_ids,
            effects_len: self.effects_len,
        }
    }

    pub(crate) fn validate_all(self) -> Result<(), ParseError> {
        let mut readers = vec![self.reader()];
        let mut validated_nodes = 1usize;
        while let Some(reader) = readers.last_mut() {
            let Some(node) = reader.next(true)? else {
                readers.pop();
                continue;
            };
            validated_nodes = validated_nodes
                .checked_add(1)
                .ok_or(ParseError::LengthOverflow)?;
            if validated_nodes > self.limits.max_nodes {
                return Err(ParseError::Validation(
                    crate::ArtifactValidationError::TooManyNodes {
                        count: validated_nodes,
                        limit: self.limits.max_nodes,
                    },
                ));
            }
            let depth = reader.depth;
            if depth > self.limits.max_depth {
                return Err(ParseError::Validation(
                    crate::ArtifactValidationError::NestingTooDeep {
                        depth,
                        limit: self.limits.max_depth,
                    },
                ));
            }
            match node {
                V10StreamNode::Box { children, .. }
                | V10StreamNode::Glue {
                    leader: V10StreamLeader::Box { children, .. },
                    ..
                } => readers.push(children.reader()),
                V10StreamNode::Ignored(children) => {
                    readers.extend(children.into_iter().rev().map(Self::reader));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl<'r, 'a> V10NodeListReader<'r, 'a> {
    pub(crate) fn is_empty(&self) -> bool {
        self.remaining == 0
    }

    pub(crate) fn next(
        &mut self,
        validate: bool,
    ) -> Result<Option<V10StreamNode<'r, 'a>>, ParseError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;

        let tag = self.reader.u8()?;
        Ok(Some(match tag {
            wire::node::CHAR => {
                let font_id = self.reader.u32()?;
                let ch = self.reader.u32()?;
                let width = self.reader.scaled()?;
                if validate {
                    validate_streamed_char(self.font_ids, font_id, ch)?;
                }
                V10StreamNode::Char { font_id, ch, width }
            }
            wire::node::LIG => {
                let font_id = self.reader.u32()?;
                let ch = self.reader.u32()?;
                let width = self.reader.scaled()?;
                let count = self.reader.u32()? as usize;
                if count == 0 || count > 63 {
                    return Err(ParseError::Validation(
                        crate::ArtifactValidationError::InvalidLigatureSourceLength { count },
                    ));
                }
                if validate {
                    validate_streamed_char(self.font_ids, font_id, ch)?;
                }
                for _ in 0..count {
                    let source = self.reader.u32()?;
                    if validate {
                        validate_streamed_char(self.font_ids, font_id, source)?;
                    }
                }
                V10StreamNode::Char { font_id, ch, width }
            }
            wire::node::KERN => {
                let amount = self.reader.scaled()?;
                parse_kern_kind(self.reader.u8()?)?;
                V10StreamNode::Kern(amount)
            }
            wire::node::MARGIN_KERN => {
                let amount = self.reader.scaled()?;
                match self.reader.u8()? {
                    0 | 1 => {}
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "margin kern side",
                            tag,
                        });
                    }
                }
                let font_id = self.reader.u32()?;
                let ch = self.reader.u8()?;
                if validate {
                    validate_streamed_char(self.font_ids, font_id, u32::from(ch))?;
                }
                V10StreamNode::Kern(amount)
            }
            wire::node::GLUE => {
                let spec = self.reader.glue_spec()?;
                let kind = parse_glue_kind(self.reader.u8()?)?;
                let leader = match self.reader.u8()? {
                    wire::leader::NONE => V10StreamLeader::None,
                    wire::leader::RULE => V10StreamLeader::Rule {
                        width: self.reader.optional_scaled()?,
                        height: self.reader.optional_scaled()?,
                        depth: self.reader.optional_scaled()?,
                    },
                    tag @ (wire::leader::HLIST | wire::leader::VLIST) => {
                        let fields = self.reader.box_fields()?.finish(Vec::new());
                        let count = self.reader.collection_len(5)?;
                        let children = self.read_list(count, self.depth + 1)?;
                        V10StreamLeader::Box {
                            vertical: tag == wire::leader::VLIST,
                            fields,
                            children,
                        }
                    }
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "leader payload",
                            tag,
                        });
                    }
                };
                V10StreamNode::Glue { spec, kind, leader }
            }
            wire::node::PENALTY => {
                self.reader.i32()?;
                V10StreamNode::Ignored(Vec::new())
            }
            wire::node::RULE => V10StreamNode::Rule {
                width: self.reader.optional_scaled()?,
                height: self.reader.optional_scaled()?,
                depth: self.reader.optional_scaled()?,
            },
            tag @ (wire::node::HLIST | wire::node::VLIST) => {
                let fields = self.reader.box_fields()?.finish(Vec::new());
                let remaining = self.reader.collection_len(5)?;
                let children = self.read_list(remaining, self.depth + 1)?;
                V10StreamNode::Box {
                    vertical: tag == wire::node::VLIST,
                    fields,
                    children,
                }
            }
            wire::node::WHATSIT_ANCHOR => {
                let effect_index = self.reader.u32()?;
                if validate
                    && usize::try_from(effect_index).unwrap_or(usize::MAX) >= self.effects_len
                {
                    return Err(ParseError::Validation(
                        crate::ArtifactValidationError::MissingEffect { effect_index },
                    ));
                }
                V10StreamNode::WhatsitAnchor(effect_index)
            }
            wire::node::MATH_ON | wire::node::MATH_OFF => {
                V10StreamNode::Math(self.reader.scaled()?)
            }
            wire::node::DISC => {
                parse_disc_kind(self.reader.u8()?)?;
                let mut children = Vec::with_capacity(3);
                for _ in 0..3 {
                    let remaining = self.reader.collection_len(5)?;
                    children.push(self.read_list(remaining, self.depth + 1)?);
                }
                V10StreamNode::Ignored(children)
            }
            wire::node::MARK => {
                self.reader.u16()?;
                self.validate_tokens(validate)?;
                V10StreamNode::Ignored(Vec::new())
            }
            wire::node::INSERT | wire::node::ADJUST => {
                if tag == wire::node::INSERT {
                    self.reader.u16()?;
                }
                let remaining = self.reader.collection_len(5)?;
                V10StreamNode::Ignored(vec![self.read_list(remaining, self.depth + 1)?])
            }
            tag => return Err(ParseError::InvalidTag { kind: "node", tag }),
        }))
    }

    fn read_list(
        &mut self,
        count: usize,
        depth: usize,
    ) -> Result<V10NodeListSlice<'r, 'a>, ParseError> {
        let start = self.reader.offset;
        for _ in 0..count {
            self.reader.skip_node()?;
        }
        Ok(V10NodeListSlice {
            bytes: self.reader.bytes,
            start,
            count,
            depth,
            limits: self.reader.limits,
            font_ids: self.font_ids,
            effects_len: self.effects_len,
        })
    }

    fn validate_tokens(&mut self, validate: bool) -> Result<(), ParseError> {
        let len = self.reader.collection_len(2)?;
        for _ in 0..len {
            match self.reader.u8()? {
                wire::token::CHAR => {
                    let ch = self.reader.u32()?;
                    parse_token_catcode(self.reader.u8()?)?;
                    if validate && char::from_u32(ch).is_none() {
                        return Err(ParseError::Validation(
                            crate::ArtifactValidationError::InvalidTokenScalar { ch },
                        ));
                    }
                }
                wire::token::CONTROL_SEQUENCE => {
                    self.reader.str()?;
                }
                wire::token::PARAM => {
                    let slot = self.reader.u8()?;
                    if validate && !(1..=9).contains(&slot) {
                        return Err(ParseError::Validation(
                            crate::ArtifactValidationError::InvalidTokenScalar {
                                ch: u32::from(slot),
                            },
                        ));
                    }
                }
                wire::token::ACTIVE_CONTROL_SEQUENCE => {
                    let ch = self.reader.u32()?;
                    if validate && char::from_u32(ch).is_none() {
                        return Err(ParseError::Validation(
                            crate::ArtifactValidationError::InvalidTokenScalar { ch },
                        ));
                    }
                }
                tag => return Err(ParseError::InvalidTag { kind: "token", tag }),
            }
        }
        Ok(())
    }
}

pub(super) fn validate_streamed_char(
    fonts: &std::collections::BTreeMap<u32, bool>,
    font_id: u32,
    ch: u32,
) -> Result<(), ParseError> {
    let Some(allows_unicode) = fonts.get(&font_id).copied() else {
        return Err(ParseError::Validation(
            crate::ArtifactValidationError::MissingFont { font_id },
        ));
    };
    if (!allows_unicode && ch > u32::from(u8::MAX)) || char::from_u32(ch).is_none() {
        return Err(ParseError::Validation(
            crate::ArtifactValidationError::CharacterOutOfRange { ch },
        ));
    }
    Ok(())
}
