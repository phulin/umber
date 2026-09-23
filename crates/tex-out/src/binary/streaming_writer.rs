use super::*;

/// Canonical artifact encoder fed one detached root child at a time.
///
/// This is the fresh-shipout counterpart of [`ArtifactPageDecoder`]: callers may
/// lower, encode, and release each direct page child without retaining a
/// recursive whole-page `PageArtifact`.
pub struct ArtifactEmitter {
    job: crate::JobInfo,
    counts: [i32; 10],
    root: Writer,
    child_count_offset: usize,
    child_count: u32,
    limits: ArtifactCodecLimits,
}

impl ArtifactEmitter {
    #[must_use]
    pub fn new(job: crate::JobInfo, counts: [i32; 10], root: &BoxNode, vertical: bool) -> Self {
        let limits = ArtifactCodecLimits::default();
        let mut writer = Writer::new(limits);
        writer.nodes_seen = 1;
        writer.u8(if vertical {
            wire::node::VLIST
        } else {
            wire::node::HLIST
        });
        writer.box_fields(root);
        let child_count_offset = writer.bytes.len();
        writer.u32(0);
        Self {
            job,
            counts,
            root: writer,
            child_count_offset,
            child_count: 0,
            limits,
        }
    }

    pub fn push_node(&mut self, node: &PageNode) -> Result<(), SerializeError> {
        self.child_count = self
            .child_count
            .checked_add(1)
            .ok_or(SerializeError::LengthOverflow)?;
        self.root.node(node);
        if let Some(error) = self.root.error.clone() {
            return Err(error);
        }
        Ok(())
    }

    /// Writes one root child directly into the canonical artifact stream.
    ///
    /// Unlike [`Self::push_node`], this API never requires an owned recursive
    /// [`PageNode`]. The closure writes the child's nested lists directly into
    /// the final artifact buffer, with collection lengths backpatched after
    /// each list completes.
    pub fn push_streamed_node<E>(
        &mut self,
        write: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        let count = self.stream_root_nodes(write)?;
        if count != 1 {
            return Err(SerializeError::LengthOverflow.into());
        }
        Ok(())
    }

    pub fn stream_root_nodes<E>(
        &mut self,
        write: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<u32, E>
    where
        E: From<SerializeError>,
    {
        let count = {
            let mut nodes = ArtifactNodeListEmitter::new(&mut self.root, 1);
            write(&mut nodes)?;
            nodes.count
        };
        self.child_count = self
            .child_count
            .checked_add(count)
            .ok_or(SerializeError::LengthOverflow)
            .map_err(E::from)?;
        if let Some(error) = self.root.error.clone() {
            return Err(error.into());
        }
        Ok(count)
    }

    pub fn finish(
        self,
        fonts: &[FontResource],
        effects: &[PageEffect],
    ) -> Result<Vec<u8>, SerializeError> {
        self.finish_with_math(fonts, effects, &[])
    }

    /// Finishes an artifact with its detached fixed-position math overlay.
    pub fn finish_with_math(
        self,
        fonts: &[FontResource],
        effects: &[PageEffect],
        math_events: &[MathOutputEvent],
    ) -> Result<Vec<u8>, SerializeError> {
        let mut this = self;
        let end = this.child_count_offset + 4;
        this.root.bytes[this.child_count_offset..end]
            .copy_from_slice(&this.child_count.to_le_bytes());
        let root = this.root.finish()?;

        let mut writer = Writer::new(this.limits);
        writer.raw(MAGIC);
        writer.u8(VERSION);
        writer.i32(this.job.mag);
        writer.str(&this.job.banner);
        writer.scaled(this.job.h_offset);
        writer.scaled(this.job.v_offset);
        writer.scaled(this.job.page_origin_x);
        writer.scaled(this.job.page_origin_y);
        writer.scaled(this.job.page_width);
        writer.scaled(this.job.page_height);
        writer.fonts(fonts);
        for value in this.counts {
            writer.i32(value);
        }
        writer.raw(&root);
        writer.effects(effects);
        writer.math_events(math_events);
        writer.finish()
    }
}

/// Direct canonical node-list writer used by fresh shipout.
///
/// The writer is a cursor into the artifact's final byte buffer, not a page
/// node or event collection. Nested closures serialize immediately and retain
/// only a byte offset for backpatching their direct-child count.
pub struct ArtifactNodeListEmitter<'a> {
    writer: &'a mut Writer,
    depth: usize,
    count: u32,
}

impl<'a> ArtifactNodeListEmitter<'a> {
    fn new(writer: &'a mut Writer, depth: usize) -> Self {
        Self {
            writer,
            depth,
            count: 0,
        }
    }

    #[inline]
    fn begin_node(&mut self) -> Result<(), SerializeError> {
        if self.depth > self.writer.limits.max_depth {
            return Err(SerializeError::LimitExceeded {
                kind: CodecLimitKind::Depth,
                actual: self.depth,
                limit: self.writer.limits.max_depth,
            });
        }
        self.writer.nodes_seen = self
            .writer
            .nodes_seen
            .checked_add(1)
            .ok_or(SerializeError::LengthOverflow)?;
        if self.writer.nodes_seen > self.writer.limits.max_nodes {
            return Err(SerializeError::LimitExceeded {
                kind: CodecLimitKind::Nodes,
                actual: self.writer.nodes_seen,
                limit: self.writer.limits.max_nodes,
            });
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or(SerializeError::LengthOverflow)?;
        Ok(())
    }

    fn nested_list<E>(
        &mut self,
        write: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E> {
        let count_offset = self.writer.bytes.len();
        self.writer.u32(0);
        let count = {
            let mut children = ArtifactNodeListEmitter::new(self.writer, self.depth + 1);
            write(&mut children)?;
            children.count
        };
        let end = count_offset + 4;
        self.writer.bytes[count_offset..end].copy_from_slice(&count.to_le_bytes());
        Ok(())
    }

    #[inline]
    pub fn char(&mut self, font_id: u32, ch: u32, width: Scaled) -> Result<(), SerializeError> {
        self.begin_node()?;
        let mut bytes = [0; 13];
        bytes[0] = wire::node::CHAR;
        bytes[1..5].copy_from_slice(&font_id.to_le_bytes());
        bytes[5..9].copy_from_slice(&ch.to_le_bytes());
        bytes[9..13].copy_from_slice(&width.raw().to_le_bytes());
        self.writer.raw(&bytes);
        Ok(())
    }

    pub fn lig(
        &mut self,
        font_id: u32,
        ch: u32,
        source: impl ExactSizeIterator<Item = u32>,
        width: Scaled,
    ) -> Result<(), SerializeError> {
        self.begin_node()?;
        let count = source.len();
        if count == 0 || count > 63 {
            return Err(SerializeError::LengthOverflow);
        }
        let mut bytes = [0; 17];
        bytes[0] = wire::node::LIG;
        bytes[1..5].copy_from_slice(&font_id.to_le_bytes());
        bytes[5..9].copy_from_slice(&ch.to_le_bytes());
        bytes[9..13].copy_from_slice(&width.raw().to_le_bytes());
        bytes[13..17].copy_from_slice(&(count as u32).to_le_bytes());
        self.writer.raw(&bytes);
        for code in source {
            self.writer.u32(code);
        }
        Ok(())
    }

    #[inline]
    pub fn kern(&mut self, amount: Scaled, kind: KernKind) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.u8(wire::node::KERN);
        self.writer.scaled(amount);
        self.writer.u8(kern_kind_tag(kind));
        Ok(())
    }

    pub fn margin_kern(
        &mut self,
        amount: Scaled,
        side: MarginKernSide,
        font_id: u32,
        ch: u8,
    ) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.u8(wire::node::MARGIN_KERN);
        self.writer.scaled(amount);
        self.writer
            .u8(u8::from(matches!(side, MarginKernSide::Right)));
        self.writer.u32(font_id);
        self.writer.u8(ch);
        Ok(())
    }

    pub fn penalty(&mut self, value: i32) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.tagged_i32(wire::node::PENALTY, value);
        Ok(())
    }

    pub fn rule(
        &mut self,
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    ) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.u8(wire::node::RULE);
        self.writer.optional_scaled(width);
        self.writer.optional_scaled(height);
        self.writer.optional_scaled(depth);
        Ok(())
    }

    pub fn box_node<E>(
        &mut self,
        vertical: bool,
        fields: &BoxNode,
        children: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        self.begin_node().map_err(E::from)?;
        self.writer.u8(if vertical {
            wire::node::VLIST
        } else {
            wire::node::HLIST
        });
        self.writer.box_fields(fields);
        self.nested_list(children)
    }

    pub fn glue(&mut self, spec: GlueSpec, kind: GlueKind) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.u8(wire::node::GLUE);
        self.writer.glue_spec(spec);
        self.writer.u8(glue_kind_tag(kind));
        self.writer.u8(wire::leader::NONE);
        Ok(())
    }

    pub fn glue_rule_leader(
        &mut self,
        spec: GlueSpec,
        kind: GlueKind,
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    ) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.u8(wire::node::GLUE);
        self.writer.glue_spec(spec);
        self.writer.u8(glue_kind_tag(kind));
        self.writer.u8(wire::leader::RULE);
        self.writer.optional_scaled(width);
        self.writer.optional_scaled(height);
        self.writer.optional_scaled(depth);
        Ok(())
    }

    pub fn glue_box_leader<E>(
        &mut self,
        spec: GlueSpec,
        kind: GlueKind,
        vertical: bool,
        fields: &BoxNode,
        children: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        self.begin_node().map_err(E::from)?;
        self.writer.u8(wire::node::GLUE);
        self.writer.glue_spec(spec);
        self.writer.u8(glue_kind_tag(kind));
        self.writer.u8(if vertical {
            wire::leader::VLIST
        } else {
            wire::leader::HLIST
        });
        self.writer.box_fields(fields);
        self.nested_list(children)
    }

    pub fn disc<E>(
        &mut self,
        kind: DiscKind,
        write: impl FnOnce(&mut ArtifactDiscEmitter<'_, '_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        self.begin_node().map_err(E::from)?;
        self.writer.u8(wire::node::DISC);
        self.writer.u8(disc_kind_tag(kind));
        let mut disc = ArtifactDiscEmitter {
            nodes: self,
            phase: 0,
        };
        write(&mut disc)?;
        if disc.phase != 3 {
            return Err(SerializeError::LengthOverflow.into());
        }
        Ok(())
    }

    pub fn mark(&mut self, class: u16, tokens: &[PageToken]) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.u8(wire::node::MARK);
        self.writer.u16(class);
        self.writer.tokens(tokens);
        Ok(())
    }

    /// Writes a mark token list directly from a borrowed source.
    ///
    /// Control-sequence spellings are copied straight into the canonical byte
    /// buffer, avoiding the temporary `Vec<PageToken>` and owned `String`s
    /// used by the compatibility model.
    pub fn mark_stream<E>(
        &mut self,
        class: u16,
        write: impl FnOnce(&mut ArtifactTokenEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        self.begin_node().map_err(E::from)?;
        self.writer.u8(wire::node::MARK);
        self.writer.u16(class);
        let count_offset = self.writer.bytes.len();
        self.writer.u32(0);
        let count = {
            let mut tokens = ArtifactTokenEmitter {
                writer: self.writer,
                count: 0,
            };
            write(&mut tokens)?;
            tokens.count
        };
        let end = count_offset + 4;
        self.writer.bytes[count_offset..end].copy_from_slice(&count.to_le_bytes());
        Ok(())
    }

    pub fn insert<E>(
        &mut self,
        class: u16,
        content: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        self.begin_node().map_err(E::from)?;
        self.writer.u8(wire::node::INSERT);
        self.writer.u16(class);
        self.nested_list(content)
    }

    pub fn adjust<E>(
        &mut self,
        content: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        self.begin_node().map_err(E::from)?;
        self.writer.u8(wire::node::ADJUST);
        self.nested_list(content)
    }

    pub fn whatsit_anchor(&mut self, effect_index: u32) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer
            .tagged_u32(wire::node::WHATSIT_ANCHOR, effect_index);
        Ok(())
    }

    pub fn math_on(&mut self, width: Scaled) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.tagged_i32(wire::node::MATH_ON, width.raw());
        Ok(())
    }

    pub fn math_off(&mut self, width: Scaled) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.tagged_i32(wire::node::MATH_OFF, width.raw());
        Ok(())
    }
}

pub struct ArtifactTokenEmitter<'a> {
    writer: &'a mut Writer,
    count: u32,
}

impl ArtifactTokenEmitter<'_> {
    fn begin(&mut self) -> Result<(), SerializeError> {
        self.count = self
            .count
            .checked_add(1)
            .ok_or(SerializeError::LengthOverflow)?;
        let actual = usize::try_from(self.count).map_err(|_| SerializeError::LengthOverflow)?;
        if actual > self.writer.limits.max_collection_len {
            return Err(SerializeError::LimitExceeded {
                kind: CodecLimitKind::CollectionLength,
                actual,
                limit: self.writer.limits.max_collection_len,
            });
        }
        Ok(())
    }

    pub fn char(&mut self, ch: u32, cat: TokenCatcode) -> Result<(), SerializeError> {
        self.begin()?;
        self.writer.u8(wire::token::CHAR);
        self.writer.u32(ch);
        self.writer.u8(token_catcode_tag(cat));
        Ok(())
    }

    pub fn control_sequence(&mut self, name: &str) -> Result<(), SerializeError> {
        self.begin()?;
        self.writer.u8(wire::token::CONTROL_SEQUENCE);
        self.writer.str(name);
        self.writer.error.clone().map_or(Ok(()), Err)
    }

    pub fn param(&mut self, slot: u8) -> Result<(), SerializeError> {
        self.begin()?;
        self.writer.u8(wire::token::PARAM);
        self.writer.u8(slot);
        Ok(())
    }
}

pub struct ArtifactDiscEmitter<'a, 'b> {
    nodes: &'a mut ArtifactNodeListEmitter<'b>,
    phase: u8,
}

impl ArtifactDiscEmitter<'_, '_> {
    pub fn pre<E>(
        &mut self,
        write: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        if self.phase != 0 {
            return Err(SerializeError::LengthOverflow.into());
        }
        self.nodes.nested_list(write)?;
        self.phase = 1;
        Ok(())
    }

    pub fn post<E>(
        &mut self,
        write: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        if self.phase != 1 {
            return Err(SerializeError::LengthOverflow.into());
        }
        self.nodes.nested_list(write)?;
        self.phase = 2;
        Ok(())
    }

    pub fn replace<E>(
        &mut self,
        write: impl FnOnce(&mut ArtifactNodeListEmitter<'_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        if self.phase != 2 {
            return Err(SerializeError::LengthOverflow.into());
        }
        self.nodes.nested_list(write)?;
        self.phase = 3;
        Ok(())
    }
}
