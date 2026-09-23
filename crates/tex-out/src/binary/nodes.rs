use super::*;

impl Reader<'_> {
    pub(super) fn node(&mut self) -> Result<PageNode, ParseError> {
        let mut frames = Vec::new();
        loop {
            let depth = frames.len() + 1;
            if depth > self.limits.max_depth {
                return Err(ParseError::LimitExceeded {
                    kind: CodecLimitKind::Depth,
                    actual: depth,
                    limit: self.limits.max_depth,
                });
            }
            self.nodes_seen = self
                .nodes_seen
                .checked_add(1)
                .ok_or(ParseError::LengthOverflow)?;
            if self.nodes_seen > self.limits.max_nodes {
                return Err(ParseError::LimitExceeded {
                    kind: CodecLimitKind::Nodes,
                    actual: self.nodes_seen,
                    limit: self.limits.max_nodes,
                });
            }

            let mut completed = match self
                .node_head(NodeReadMode::Owned)?
                .parsed
                .expect("retained node scan always returns a parsed node")
            {
                ParsedNode::Complete(node) => node,
                ParsedNode::Frame(mut frame) => match frame.advance(None, self)? {
                    FrameProgress::NeedChild => {
                        frames.push(frame);
                        continue;
                    }
                    FrameProgress::Complete(node) => node,
                },
            };

            loop {
                let Some(mut frame) = frames.pop() else {
                    return Ok(completed);
                };
                match frame.advance(Some(completed), self)? {
                    FrameProgress::NeedChild => {
                        frames.push(frame);
                        break;
                    }
                    FrameProgress::Complete(node) => completed = node,
                }
            }
        }
    }

    pub(super) fn skip_node(&mut self) -> Result<(), ParseError> {
        let mut frames = Vec::new();
        loop {
            let depth = frames.len() + 1;
            self.begin_node(depth)?;
            if let Some(mut frame) = self.node_head(NodeReadMode::Scan)?.skip
                && frame.ready(self)?
            {
                frames.push(frame);
                continue;
            }
            loop {
                let Some(frame) = frames.last_mut() else {
                    return Ok(());
                };
                if frame.child_finished(self)? {
                    break;
                }
                frames.pop();
            }
        }
    }

    pub(super) fn begin_node(&mut self, depth: usize) -> Result<(), ParseError> {
        if depth > self.limits.max_depth {
            return Err(ParseError::LimitExceeded {
                kind: CodecLimitKind::Depth,
                actual: depth,
                limit: self.limits.max_depth,
            });
        }
        self.nodes_seen = self
            .nodes_seen
            .checked_add(1)
            .ok_or(ParseError::LengthOverflow)?;
        if self.nodes_seen > self.limits.max_nodes {
            return Err(ParseError::LimitExceeded {
                kind: CodecLimitKind::Nodes,
                actual: self.nodes_seen,
                limit: self.limits.max_nodes,
            });
        }
        Ok(())
    }

    fn node_head(&mut self, mode: NodeReadMode) -> Result<ScannedNodeHead, ParseError> {
        let retain = mode == NodeReadMode::Owned;
        let tag = self.u8()?;
        let parsed = match tag {
            wire::node::CHAR => Ok(ParsedNode::Complete(PageNode::Char {
                font_id: self.u32()?,
                ch: self.u32()?,
                width: self.scaled()?,
            })),
            wire::node::LIG => Ok(ParsedNode::Complete(PageNode::Lig {
                font_id: self.u32()?,
                ch: self.u32()?,
                width: self.scaled()?,
                source: {
                    let count = self.u32()? as usize;
                    if count == 0 || count > 63 {
                        return Err(ParseError::Validation(
                            crate::ArtifactValidationError::InvalidLigatureSourceLength { count },
                        ));
                    }
                    (0..count)
                        .map(|_| self.u32())
                        .collect::<Result<Vec<_>, _>>()?
                },
            })),
            wire::node::KERN => Ok(ParsedNode::Complete(PageNode::Kern {
                amount: self.scaled()?,
                kind: parse_kern_kind(self.u8()?)?,
            })),
            wire::node::MARGIN_KERN => Ok(ParsedNode::Complete(PageNode::MarginKern {
                amount: self.scaled()?,
                side: match self.u8()? {
                    0 => MarginKernSide::Left,
                    1 => MarginKernSide::Right,
                    value => {
                        return Err(ParseError::InvalidTag {
                            kind: "margin kern side",
                            tag: value,
                        });
                    }
                },
                font_id: self.u32()?,
                ch: self.u8()?,
            })),
            wire::node::GLUE => {
                let spec = self.glue_spec()?;
                let kind = parse_glue_kind(self.u8()?)?;
                match self.u8()? {
                    wire::leader::NONE => Ok(ParsedNode::Complete(PageNode::Glue {
                        spec,
                        kind,
                        leader: None,
                    })),
                    tag @ (wire::leader::HLIST | wire::leader::VLIST) => {
                        let fields = self.box_fields()?;
                        let remaining = self.collection_len(5)?;
                        Ok(ParsedNode::Frame(DecodeFrame::LeaderBox {
                            spec,
                            kind,
                            vertical: tag == wire::leader::VLIST,
                            fields,
                            children: Vec::with_capacity(if retain { remaining } else { 0 }),
                            remaining,
                        }))
                    }
                    wire::leader::RULE => Ok(ParsedNode::Complete(PageNode::Glue {
                        spec,
                        kind,
                        leader: Some(LeaderPayload::Rule {
                            width: self.optional_scaled()?,
                            height: self.optional_scaled()?,
                            depth: self.optional_scaled()?,
                        }),
                    })),
                    tag => Err(ParseError::InvalidTag {
                        kind: "leader payload",
                        tag,
                    }),
                }
            }
            wire::node::PENALTY => Ok(ParsedNode::Complete(PageNode::Penalty(self.i32()?))),
            wire::node::RULE => Ok(ParsedNode::Complete(PageNode::Rule {
                width: self.optional_scaled()?,
                height: self.optional_scaled()?,
                depth: self.optional_scaled()?,
            })),
            tag @ (wire::node::HLIST | wire::node::VLIST) => {
                let fields = self.box_fields()?;
                let remaining = self.collection_len(5)?;
                Ok(ParsedNode::Frame(DecodeFrame::Box {
                    vertical: tag == wire::node::VLIST,
                    fields,
                    children: Vec::with_capacity(if retain { remaining } else { 0 }),
                    remaining,
                }))
            }
            wire::node::WHATSIT_ANCHOR => Ok(ParsedNode::Complete(PageNode::WhatsitAnchor {
                effect_index: self.u32()?,
            })),
            wire::node::MATH_ON => Ok(ParsedNode::Complete(PageNode::MathOn(self.scaled()?))),
            wire::node::MATH_OFF => Ok(ParsedNode::Complete(PageNode::MathOff(self.scaled()?))),
            wire::node::DISC => {
                let kind = parse_disc_kind(self.u8()?)?;
                let remaining = self.collection_len(5)?;
                Ok(ParsedNode::Frame(DecodeFrame::Disc {
                    kind,
                    phase: 0,
                    pre: Vec::with_capacity(if retain { remaining } else { 0 }),
                    post: Vec::new(),
                    replace: Vec::new(),
                    remaining,
                }))
            }
            wire::node::MARK => Ok(ParsedNode::Complete(PageNode::Mark {
                class: self.u16()?,
                tokens: self.tokens_with_mode(mode)?,
            })),
            wire::node::INSERT => {
                let class = self.u16()?;
                let remaining = self.collection_len(5)?;
                Ok(ParsedNode::Frame(DecodeFrame::Insert {
                    class,
                    content: Vec::with_capacity(if retain { remaining } else { 0 }),
                    remaining,
                }))
            }
            wire::node::ADJUST => {
                let remaining = self.collection_len(5)?;
                Ok(ParsedNode::Frame(DecodeFrame::Adjust {
                    content: Vec::with_capacity(if retain { remaining } else { 0 }),
                    remaining,
                }))
            }
            tag => Err(ParseError::InvalidTag { kind: "node", tag }),
        }?;
        Ok(ScannedNodeHead::from_parsed(mode, parsed))
    }

    fn tokens_with_mode(&mut self, mode: NodeReadMode) -> Result<Vec<PageToken>, ParseError> {
        let retain = mode == NodeReadMode::Owned;
        let len = self.collection_len(2)?;
        let mut tokens = Vec::with_capacity(if retain { len } else { 0 });
        for _ in 0..len {
            let token = match self.u8()? {
                wire::token::CHAR => PageToken::Char {
                    ch: self.u32()?,
                    cat: parse_token_catcode(self.u8()?)?,
                },
                wire::token::CONTROL_SEQUENCE => {
                    let name = self.str_ref()?;
                    PageToken::ControlSequence(if retain {
                        name.to_owned()
                    } else {
                        String::new()
                    })
                }
                wire::token::PARAM => PageToken::Param(self.u8()?),
                wire::token::ACTIVE_CONTROL_SEQUENCE => {
                    PageToken::ActiveControlSequence(self.u32()?)
                }
                tag => {
                    return Err(ParseError::InvalidTag { kind: "token", tag });
                }
            };
            if retain {
                tokens.push(token);
            }
        }
        Ok(tokens)
    }

    pub(super) fn box_fields(&mut self) -> Result<BoxFields, ParseError> {
        let width = self.scaled()?;
        let height = self.scaled()?;
        let depth = self.scaled()?;
        let shift = self.scaled()?;
        let numerator = self.i32()?;
        let denominator = self.i32()?;
        let glue_set =
            GlueSetRatio::try_from_ratio_parts(numerator, denominator).map_err(|_| {
                ParseError::InvalidGlueSetRatio {
                    numerator,
                    denominator,
                }
            })?;
        let glue_sign = parse_glue_sign(self.u8()?)?;
        let glue_order = parse_glue_order(self.u8()?)?;
        Ok(BoxFields {
            width,
            height,
            depth,
            shift,
            glue_set,
            glue_sign,
            glue_order,
        })
    }

    pub(super) fn glue_spec(&mut self) -> Result<GlueSpec, ParseError> {
        Ok(GlueSpec {
            width: self.scaled()?,
            stretch: self.scaled()?,
            stretch_order: parse_glue_order(self.u8()?)?,
            shrink: self.scaled()?,
            shrink_order: parse_glue_order(self.u8()?)?,
        })
    }
}

pub(super) struct BoxFields {
    width: Scaled,
    height: Scaled,
    depth: Scaled,
    shift: Scaled,
    glue_set: GlueSetRatio,
    glue_sign: GlueSign,
    glue_order: GlueOrder,
}

impl BoxFields {
    pub(super) fn finish(self, children: Vec<PageNode>) -> BoxNode {
        BoxNode {
            width: self.width,
            height: self.height,
            depth: self.depth,
            shift: self.shift,
            glue_set: self.glue_set,
            glue_sign: self.glue_sign,
            glue_order: self.glue_order,
            children,
        }
    }
}

struct ScannedNodeHead {
    parsed: Option<ParsedNode>,
    skip: Option<SkipFrame>,
}

impl ScannedNodeHead {
    fn from_parsed(mode: NodeReadMode, parsed: ParsedNode) -> Self {
        if mode != NodeReadMode::Scan {
            return Self {
                parsed: Some(parsed),
                skip: None,
            };
        }
        let skip = match &parsed {
            ParsedNode::Complete(_) => None,
            ParsedNode::Frame(
                DecodeFrame::Box { remaining, .. }
                | DecodeFrame::LeaderBox { remaining, .. }
                | DecodeFrame::Insert { remaining, .. }
                | DecodeFrame::Adjust { remaining, .. },
            ) => Some(SkipFrame::List(*remaining)),
            ParsedNode::Frame(DecodeFrame::Disc { remaining, .. }) => Some(SkipFrame::Disc {
                phase: 0,
                remaining: *remaining,
            }),
        };
        Self {
            parsed: Some(parsed),
            skip,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum NodeReadMode {
    Owned,
    Scan,
}

enum ParsedNode {
    Complete(PageNode),
    Frame(DecodeFrame),
}

enum FrameProgress {
    NeedChild,
    Complete(PageNode),
}

enum SkipFrame {
    List(usize),
    Disc { phase: u8, remaining: usize },
}

impl SkipFrame {
    fn ready(&mut self, reader: &mut Reader<'_>) -> Result<bool, ParseError> {
        loop {
            match self {
                Self::List(remaining) => return Ok(*remaining > 0),
                Self::Disc { remaining, .. } if *remaining > 0 => return Ok(true),
                Self::Disc {
                    phase, remaining, ..
                } if *phase < 2 => {
                    *phase += 1;
                    *remaining = reader.collection_len(5)?;
                }
                Self::Disc { .. } => return Ok(false),
            }
        }
    }

    /// Records one completed child and reports whether another is required.
    fn child_finished(&mut self, reader: &mut Reader<'_>) -> Result<bool, ParseError> {
        match self {
            Self::List(remaining) | Self::Disc { remaining, .. } => {
                debug_assert!(*remaining > 0);
                *remaining -= 1;
            }
        }
        self.ready(reader)
    }
}

enum DecodeFrame {
    Box {
        vertical: bool,
        fields: BoxFields,
        children: Vec<PageNode>,
        remaining: usize,
    },
    LeaderBox {
        spec: GlueSpec,
        kind: GlueKind,
        vertical: bool,
        fields: BoxFields,
        children: Vec<PageNode>,
        remaining: usize,
    },
    Disc {
        kind: DiscKind,
        phase: u8,
        pre: Vec<PageNode>,
        post: Vec<PageNode>,
        replace: Vec<PageNode>,
        remaining: usize,
    },
    Insert {
        class: u16,
        content: Vec<PageNode>,
        remaining: usize,
    },
    Adjust {
        content: Vec<PageNode>,
        remaining: usize,
    },
}

impl DecodeFrame {
    fn advance(
        &mut self,
        child: Option<PageNode>,
        reader: &mut Reader<'_>,
    ) -> Result<FrameProgress, ParseError> {
        if let Some(child) = child {
            match self {
                Self::Box {
                    children,
                    remaining,
                    ..
                }
                | Self::LeaderBox {
                    children,
                    remaining,
                    ..
                } => {
                    children.push(child);
                    *remaining -= 1;
                }
                Self::Disc {
                    phase,
                    pre,
                    post,
                    replace,
                    remaining,
                    ..
                } => {
                    match phase {
                        0 => pre.push(child),
                        1 => post.push(child),
                        _ => replace.push(child),
                    }
                    *remaining -= 1;
                }
                Self::Insert {
                    content, remaining, ..
                }
                | Self::Adjust { content, remaining } => {
                    content.push(child);
                    *remaining -= 1;
                }
            }
        }

        loop {
            match self {
                Self::Box { remaining, .. }
                | Self::LeaderBox { remaining, .. }
                | Self::Insert { remaining, .. }
                | Self::Adjust { remaining, .. }
                    if *remaining > 0 =>
                {
                    return Ok(FrameProgress::NeedChild);
                }
                Self::Disc { remaining, .. } if *remaining > 0 => {
                    return Ok(FrameProgress::NeedChild);
                }
                Self::Disc {
                    phase,
                    post,
                    replace,
                    remaining,
                    ..
                } if *phase < 2 => {
                    *phase += 1;
                    *remaining = reader.collection_len(5)?;
                    let target = if *phase == 1 { post } else { replace };
                    target.reserve(*remaining);
                }
                _ => break,
            }
        }

        let frame = std::mem::replace(
            self,
            Self::Adjust {
                content: Vec::new(),
                remaining: 0,
            },
        );
        Ok(FrameProgress::Complete(match frame {
            Self::Box {
                vertical,
                fields,
                children,
                ..
            } => {
                let box_node = fields.finish(children);
                if vertical {
                    PageNode::VList(box_node)
                } else {
                    PageNode::HList(box_node)
                }
            }
            Self::LeaderBox {
                spec,
                kind,
                vertical,
                fields,
                children,
                ..
            } => {
                let box_node = fields.finish(children);
                PageNode::Glue {
                    spec,
                    kind,
                    leader: Some(if vertical {
                        LeaderPayload::VList(box_node)
                    } else {
                        LeaderPayload::HList(box_node)
                    }),
                }
            }
            Self::Disc {
                kind,
                pre,
                post,
                replace,
                ..
            } => PageNode::Disc {
                kind,
                pre,
                post,
                replace,
            },
            Self::Insert { class, content, .. } => PageNode::Insert { class, content },
            Self::Adjust { content, .. } => PageNode::Adjust(content),
        }))
    }
}
