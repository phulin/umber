use crate::{
    BoxNode, ContentHash, DiscKind, EffectSink, FontResource, FontResourceConstruction, GlueKind,
    GlueOrder, GlueSetRatio, GlueSign, GlueSpec, KernKind, LeaderPayload, PageArtifact, PageEffect,
    PageNode, PageToken, PdfAccessibilityEffect, PdfAnnotationEffect, TokenCatcode,
    UnvalidatedPageArtifact,
};
use std::fmt;
use tex_arith::Scaled;

const MAGIC: &[u8; 4] = b"UMPG";
const VERSION: u8 = 17;
const PRE_ANNOTATION_VERSION: u8 = 16;
const PDF_ACCESSIBILITY_VERSION: u8 = 15;
const FONT_CONSTRUCTION_VERSION: u8 = 14;
const OPENTYPE_FONT_VERSION: u8 = 13;
const LEGACY_VERSION: u8 = 12;

mod wire {
    pub mod node {
        pub const CHAR: u8 = 0;
        pub const LIG: u8 = 1;
        pub const KERN: u8 = 2;
        pub const GLUE: u8 = 3;
        pub const PENALTY: u8 = 4;
        pub const RULE: u8 = 5;
        pub const HLIST: u8 = 6;
        pub const VLIST: u8 = 7;
        pub const WHATSIT_ANCHOR: u8 = 9;
        pub const MATH_ON: u8 = 10;
        pub const MATH_OFF: u8 = 11;
        pub const DISC: u8 = 12;
        pub const MARK: u8 = 13;
        pub const INSERT: u8 = 14;
        pub const ADJUST: u8 = 15;
    }

    pub mod leader {
        pub const NONE: u8 = 0;
        pub const HLIST: u8 = 1;
        pub const VLIST: u8 = 2;
        pub const RULE: u8 = 3;
    }

    pub mod effect {
        pub const OPEN_OUT: u8 = 0;
        pub const CLOSE_OUT: u8 = 1;
        pub const WRITE: u8 = 2;
        pub const SPECIAL: u8 = 3;
        pub const PDF_ACCESSIBILITY: u8 = 4;
        // Tags 5..=15 are reserved by independently developed artifact effects.
        pub const PDF_ANNOTATION: u8 = 16;
    }

    pub mod token {
        pub const CHAR: u8 = 0;
        pub const CONTROL_SEQUENCE: u8 = 1;
        pub const PARAM: u8 = 2;
        pub const ACTIVE_CONTROL_SEQUENCE: u8 = 3;
    }

    pub mod sink {
        pub const TERMINAL: u8 = 0;
        pub const LOG: u8 = 1;
        pub const TERMINAL_AND_LOG: u8 = 2;
        pub const STREAM: u8 = 3;
    }

    pub mod font_construction {
        pub const LOADED: u8 = 0;
        pub const COPIED: u8 = 1;
        pub const LETTERSPACED: u8 = 2;
        pub const EXPANDED: u8 = 3;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactCodecLimits {
    pub max_bytes: usize,
    pub max_nodes: usize,
    pub max_collection_len: usize,
    pub max_collection_items: usize,
    pub max_depth: usize,
}

impl Default for ArtifactCodecLimits {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024 * 1024,
            max_nodes: 1_000_000,
            max_collection_len: 1_000_000,
            max_collection_items: 4_000_000,
            max_depth: 4096,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecLimitKind {
    Bytes,
    Nodes,
    CollectionLength,
    CollectionItems,
    Depth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SerializeError {
    LengthOverflow,
    LimitExceeded {
        kind: CodecLimitKind,
        actual: usize,
        limit: usize,
    },
}

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthOverflow => f.write_str("page artifact length exceeds the wire format"),
            Self::LimitExceeded {
                kind,
                actual,
                limit,
            } => write!(
                f,
                "page artifact {kind:?} limit exceeded: {actual} > {limit}"
            ),
        }
    }
}

impl std::error::Error for SerializeError {}

/// Binary parse failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    InvalidMagic,
    UnsupportedVersion(u8),
    UnexpectedEof,
    TrailingBytes {
        offset: usize,
        len: usize,
    },
    InvalidUtf8,
    LengthOverflow,
    InvalidTag {
        kind: &'static str,
        tag: u8,
    },
    InvalidGlueSetRatio {
        numerator: i32,
        denominator: i32,
    },
    LimitExceeded {
        kind: CodecLimitKind,
        actual: usize,
        limit: usize,
    },
    Validation(crate::ArtifactValidationError),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => f.write_str("invalid page artifact magic"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported page artifact version {version}")
            }
            Self::UnexpectedEof => f.write_str("truncated page artifact"),
            Self::TrailingBytes { offset, len } => {
                write!(f, "page artifact has trailing bytes at {offset} of {len}")
            }
            Self::InvalidUtf8 => f.write_str("page artifact contains invalid UTF-8"),
            Self::LengthOverflow => f.write_str("page artifact length exceeds this platform"),
            Self::InvalidTag { kind, tag } => write!(f, "invalid {kind} tag {tag}"),
            Self::InvalidGlueSetRatio {
                numerator,
                denominator,
            } => write!(
                f,
                "invalid glue-set ratio {numerator}/{denominator} in page artifact"
            ),
            Self::LimitExceeded {
                kind,
                actual,
                limit,
            } => write!(
                f,
                "page artifact {kind:?} limit exceeded: {actual} > {limit}"
            ),
            Self::Validation(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<crate::ArtifactValidationError> for ParseError {
    fn from(value: crate::ArtifactValidationError) -> Self {
        Self::Validation(value)
    }
}

pub(crate) fn to_bytes(
    artifact: &PageArtifact,
    limits: ArtifactCodecLimits,
) -> Result<Vec<u8>, SerializeError> {
    let mut writer = Writer::new(limits);
    writer.raw(MAGIC);
    writer.u8(VERSION);
    writer.i32(artifact.job.mag);
    writer.str(&artifact.job.banner);
    writer.scaled(artifact.job.h_offset);
    writer.scaled(artifact.job.v_offset);
    writer.fonts(&artifact.fonts);
    for value in artifact.counts {
        writer.i32(value);
    }
    writer.node(&artifact.root);
    writer.effects(&artifact.effects);
    writer.finish()
}

pub(crate) fn from_bytes(
    bytes: &[u8],
    limits: ArtifactCodecLimits,
) -> Result<UnvalidatedPageArtifact, ParseError> {
    if bytes.len() > limits.max_bytes {
        return Err(ParseError::LimitExceeded {
            kind: CodecLimitKind::Bytes,
            actual: bytes.len(),
            limit: limits.max_bytes,
        });
    }
    let mut reader = Reader {
        bytes,
        offset: 0,
        limits,
        nodes_seen: 0,
        collection_items_seen: 0,
    };
    reader.expect_magic()?;
    let version = reader.u8()?;
    if version != VERSION
        && version != PRE_ANNOTATION_VERSION
        && version != PDF_ACCESSIBILITY_VERSION
        && version != FONT_CONSTRUCTION_VERSION
        && version != OPENTYPE_FONT_VERSION
        && version != LEGACY_VERSION
    {
        return Err(ParseError::UnsupportedVersion(version));
    }
    let mag = reader.i32()?;
    let banner = reader.str()?;
    let h_offset = reader.scaled()?;
    let v_offset = reader.scaled()?;
    let fonts = reader.fonts(version)?;
    let mut counts = [0; 10];
    for value in &mut counts {
        *value = reader.i32()?;
    }
    let root = reader.node()?;
    let effects = reader.effects(version)?;
    reader.finish()?;
    Ok(UnvalidatedPageArtifact {
        job: crate::JobInfo {
            mag,
            banner,
            h_offset,
            v_offset,
        },
        fonts,
        counts,
        root,
        effects,
    })
}

/// Incremental decoder for the canonical page artifact wire layout.
///
/// Metadata and effects are retained, but the root list is decoded one direct
/// child at a time so replay never constructs an owned whole-page node tree.
pub(crate) struct V10PageDecoder<'a> {
    pub(crate) page: PageArtifact,
    pub(crate) root_vertical: bool,
    reader: Reader<'a>,
    remaining: usize,
    font_ids: std::collections::BTreeSet<u32>,
    validated_nodes: usize,
}

/// Canonical artifact encoder fed one detached root child at a time.
///
/// This is the fresh-shipout counterpart of [`V10PageDecoder`]: callers may
/// lower, encode, and release each direct page child without retaining a
/// recursive whole-page `PageArtifact`.
pub struct V10ArtifactBuilder {
    job: crate::JobInfo,
    counts: [i32; 10],
    root: Writer,
    child_count_offset: usize,
    child_count: u32,
    limits: ArtifactCodecLimits,
}

impl V10ArtifactBuilder {
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
        write: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
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
        write: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
    ) -> Result<u32, E>
    where
        E: From<SerializeError>,
    {
        let count = {
            let mut nodes = V10NodeListWriter::new(&mut self.root, 1);
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
        writer.fonts(fonts);
        for value in this.counts {
            writer.i32(value);
        }
        writer.raw(&root);
        writer.effects(effects);
        writer.finish()
    }
}

/// Direct canonical node-list writer used by fresh shipout.
///
/// The writer is a cursor into the artifact's final byte buffer, not a page
/// node or event collection. Nested closures serialize immediately and retain
/// only a byte offset for backpatching their direct-child count.
pub struct V10NodeListWriter<'a> {
    writer: &'a mut Writer,
    depth: usize,
    count: u32,
}

impl<'a> V10NodeListWriter<'a> {
    fn new(writer: &'a mut Writer, depth: usize) -> Self {
        Self {
            writer,
            depth,
            count: 0,
        }
    }

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
        write: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
    ) -> Result<(), E> {
        let count_offset = self.writer.bytes.len();
        self.writer.u32(0);
        let count = {
            let mut children = V10NodeListWriter::new(self.writer, self.depth + 1);
            write(&mut children)?;
            children.count
        };
        let end = count_offset + 4;
        self.writer.bytes[count_offset..end].copy_from_slice(&count.to_le_bytes());
        Ok(())
    }

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

    pub fn kern(&mut self, amount: Scaled, kind: KernKind) -> Result<(), SerializeError> {
        self.begin_node()?;
        self.writer.u8(wire::node::KERN);
        self.writer.scaled(amount);
        self.writer.u8(kern_kind_tag(kind));
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
        children: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
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
        children: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
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
        write: impl FnOnce(&mut V10DiscWriter<'_, '_>) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<SerializeError>,
    {
        self.begin_node().map_err(E::from)?;
        self.writer.u8(wire::node::DISC);
        self.writer.u8(disc_kind_tag(kind));
        let mut disc = V10DiscWriter {
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
        write: impl FnOnce(&mut V10TokenWriter<'_>) -> Result<(), E>,
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
            let mut tokens = V10TokenWriter {
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
        content: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
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
        content: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
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

pub struct V10TokenWriter<'a> {
    writer: &'a mut Writer,
    count: u32,
}

impl V10TokenWriter<'_> {
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

pub struct V10DiscWriter<'a, 'b> {
    nodes: &'a mut V10NodeListWriter<'b>,
    phase: u8,
}

impl V10DiscWriter<'_, '_> {
    pub fn pre<E>(
        &mut self,
        write: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
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
        write: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
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
        write: impl FnOnce(&mut V10NodeListWriter<'_>) -> Result<(), E>,
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
        let (version, job, fonts, counts) = scan.header()?;
        let root_start = scan.offset;
        scan.skip_node()?;
        let effects = scan.effects(version)?;
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
        }
        .validate()?;
        let font_ids = page.fonts.iter().map(|font| font.font_id).collect();
        Ok(Self {
            page,
            root_vertical,
            reader,
            remaining,
            font_ids,
            validated_nodes: 1,
        })
    }

    pub(crate) fn stream_children(&mut self) -> V10NodeListReader<'_, 'a> {
        let remaining = std::mem::take(&mut self.remaining);
        V10NodeListReader {
            reader: &mut self.reader,
            remaining,
            depth: 2,
            font_ids: &self.font_ids,
            effects_len: self.page.effects.len(),
            validated_nodes: &mut self.validated_nodes,
            validate: true,
        }
    }
}

pub(crate) struct V10NodeListReader<'r, 'a> {
    reader: &'r mut Reader<'a>,
    remaining: usize,
    depth: usize,
    font_ids: &'r std::collections::BTreeSet<u32>,
    effects_len: usize,
    validated_nodes: &'r mut usize,
    validate: bool,
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
        children: V10NodeListReader<'r, 'a>,
    },
    WhatsitAnchor(u32),
    Math(Scaled),
    Ignored,
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

pub(crate) struct V10NodeListSlice<'r, 'a> {
    bytes: &'a [u8],
    start: usize,
    count: usize,
    depth: usize,
    limits: ArtifactCodecLimits,
    font_ids: &'r std::collections::BTreeSet<u32>,
    effects_len: usize,
}

impl<'r, 'a> V10NodeListSlice<'r, 'a> {
    pub(crate) fn with_reader<T>(
        &self,
        read: impl FnOnce(&mut V10NodeListReader<'_, 'a>) -> T,
    ) -> T {
        let mut reader = Reader::new_at(self.bytes, self.limits, self.start);
        let mut ignored_count = 0;
        let mut nodes = V10NodeListReader {
            reader: &mut reader,
            remaining: self.count,
            depth: self.depth,
            font_ids: self.font_ids,
            effects_len: self.effects_len,
            validated_nodes: &mut ignored_count,
            validate: false,
        };
        read(&mut nodes)
    }
}

impl<'r, 'a> V10NodeListReader<'r, 'a> {
    pub(crate) fn is_empty(&self) -> bool {
        self.remaining == 0
    }

    pub(crate) fn next(&mut self) -> Result<Option<V10StreamNode<'_, 'a>>, ParseError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        if self.validate {
            *self.validated_nodes = self
                .validated_nodes
                .checked_add(1)
                .ok_or(ParseError::LengthOverflow)?;
            if *self.validated_nodes > self.reader.limits.max_nodes {
                return Err(ParseError::Validation(
                    crate::ArtifactValidationError::TooManyNodes {
                        count: *self.validated_nodes,
                        limit: self.reader.limits.max_nodes,
                    },
                ));
            }
            if self.depth > self.reader.limits.max_depth {
                return Err(ParseError::Validation(
                    crate::ArtifactValidationError::NestingTooDeep {
                        depth: self.depth,
                        limit: self.reader.limits.max_depth,
                    },
                ));
            }
        }

        let tag = self.reader.u8()?;
        Ok(Some(match tag {
            wire::node::CHAR => {
                let font_id = self.reader.u32()?;
                let ch = self.reader.u32()?;
                let width = self.reader.scaled()?;
                if self.validate {
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
                if self.validate {
                    validate_streamed_char(self.font_ids, font_id, ch)?;
                }
                for _ in 0..count {
                    let source = self.reader.u32()?;
                    if self.validate {
                        validate_streamed_scalar(source)?;
                    }
                }
                V10StreamNode::Char { font_id, ch, width }
            }
            wire::node::KERN => {
                let amount = self.reader.scaled()?;
                parse_kern_kind(self.reader.u8()?)?;
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
                        let start = self.reader.offset;
                        {
                            let mut children = V10NodeListReader {
                                reader: self.reader,
                                remaining: count,
                                depth: self.depth + 1,
                                font_ids: self.font_ids,
                                effects_len: self.effects_len,
                                validated_nodes: self.validated_nodes,
                                validate: self.validate,
                            };
                            children.validate_all()?;
                        }
                        V10StreamLeader::Box {
                            vertical: tag == wire::leader::VLIST,
                            fields,
                            children: V10NodeListSlice {
                                bytes: self.reader.bytes,
                                start,
                                count,
                                depth: self.depth + 1,
                                limits: self.reader.limits,
                                font_ids: self.font_ids,
                                effects_len: self.effects_len,
                            },
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
                V10StreamNode::Ignored
            }
            wire::node::RULE => V10StreamNode::Rule {
                width: self.reader.optional_scaled()?,
                height: self.reader.optional_scaled()?,
                depth: self.reader.optional_scaled()?,
            },
            tag @ (wire::node::HLIST | wire::node::VLIST) => {
                let fields = self.reader.box_fields()?.finish(Vec::new());
                let remaining = self.reader.collection_len(5)?;
                V10StreamNode::Box {
                    vertical: tag == wire::node::VLIST,
                    fields,
                    children: V10NodeListReader {
                        reader: self.reader,
                        remaining,
                        depth: self.depth + 1,
                        font_ids: self.font_ids,
                        effects_len: self.effects_len,
                        validated_nodes: self.validated_nodes,
                        validate: self.validate,
                    },
                }
            }
            wire::node::WHATSIT_ANCHOR => {
                let effect_index = self.reader.u32()?;
                if self.validate
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
                for _ in 0..3 {
                    let remaining = self.reader.collection_len(5)?;
                    let mut children = V10NodeListReader {
                        reader: self.reader,
                        remaining,
                        depth: self.depth + 1,
                        font_ids: self.font_ids,
                        effects_len: self.effects_len,
                        validated_nodes: self.validated_nodes,
                        validate: self.validate,
                    };
                    children.validate_all()?;
                }
                V10StreamNode::Ignored
            }
            wire::node::MARK => {
                self.reader.u16()?;
                self.validate_tokens()?;
                V10StreamNode::Ignored
            }
            wire::node::INSERT | wire::node::ADJUST => {
                if tag == wire::node::INSERT {
                    self.reader.u16()?;
                }
                let remaining = self.reader.collection_len(5)?;
                let mut children = V10NodeListReader {
                    reader: self.reader,
                    remaining,
                    depth: self.depth + 1,
                    font_ids: self.font_ids,
                    effects_len: self.effects_len,
                    validated_nodes: self.validated_nodes,
                    validate: self.validate,
                };
                children.validate_all()?;
                V10StreamNode::Ignored
            }
            tag => return Err(ParseError::InvalidTag { kind: "node", tag }),
        }))
    }

    pub(crate) fn validate_all(&mut self) -> Result<(), ParseError> {
        while let Some(node) = self.next()? {
            if let V10StreamNode::Box { mut children, .. } = node {
                children.validate_all()?;
            }
        }
        Ok(())
    }

    fn validate_tokens(&mut self) -> Result<(), ParseError> {
        let len = self.reader.collection_len(2)?;
        for _ in 0..len {
            match self.reader.u8()? {
                wire::token::CHAR => {
                    let ch = self.reader.u32()?;
                    parse_token_catcode(self.reader.u8()?)?;
                    if self.validate && char::from_u32(ch).is_none() {
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
                    if self.validate && !(1..=9).contains(&slot) {
                        return Err(ParseError::Validation(
                            crate::ArtifactValidationError::InvalidTokenScalar {
                                ch: u32::from(slot),
                            },
                        ));
                    }
                }
                wire::token::ACTIVE_CONTROL_SEQUENCE => {
                    let ch = self.reader.u32()?;
                    if self.validate && char::from_u32(ch).is_none() {
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

struct Writer {
    bytes: Vec<u8>,
    limits: ArtifactCodecLimits,
    error: Option<SerializeError>,
    nodes_seen: usize,
    collection_items_seen: usize,
}

impl Writer {
    fn new(limits: ArtifactCodecLimits) -> Self {
        Self {
            bytes: Vec::new(),
            limits,
            error: None,
            nodes_seen: 0,
            collection_items_seen: 0,
        }
    }

    fn finish(self) -> Result<Vec<u8>, SerializeError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(self.bytes)
    }

    fn raw(&mut self, bytes: &[u8]) {
        if self.error.is_some() {
            return;
        }
        let Some(actual) = self.bytes.len().checked_add(bytes.len()) else {
            self.error = Some(SerializeError::LengthOverflow);
            return;
        };
        if actual > self.limits.max_bytes {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::Bytes,
                actual,
                limit: self.limits.max_bytes,
            });
            return;
        }
        self.bytes.extend_from_slice(bytes);
    }

    fn u8(&mut self, value: u8) {
        self.raw(&[value]);
    }

    fn u32(&mut self, value: u32) {
        self.raw(&value.to_le_bytes());
    }

    fn u16(&mut self, value: u16) {
        self.raw(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.raw(&value.to_le_bytes());
    }

    fn scaled(&mut self, value: Scaled) {
        self.i32(value.raw());
    }

    fn len(&mut self, len: usize) {
        match u32::try_from(len) {
            Ok(len) => self.u32(len),
            Err(_) => self.error = Some(SerializeError::LengthOverflow),
        }
    }

    fn collection_len(&mut self, len: usize) {
        if len > self.limits.max_collection_len {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::CollectionLength,
                actual: len,
                limit: self.limits.max_collection_len,
            });
            return;
        }
        let Some(actual) = self.collection_items_seen.checked_add(len) else {
            self.error = Some(SerializeError::LengthOverflow);
            return;
        };
        if actual > self.limits.max_collection_items {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::CollectionItems,
                actual,
                limit: self.limits.max_collection_items,
            });
            return;
        }
        self.collection_items_seen = actual;
        self.len(len);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        if bytes.len() > self.limits.max_collection_len {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::CollectionLength,
                actual: bytes.len(),
                limit: self.limits.max_collection_len,
            });
            return;
        }
        self.len(bytes.len());
        self.raw(bytes);
    }

    fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn hash(&mut self, value: ContentHash) {
        self.raw(&value.bytes());
    }

    fn optional_scaled(&mut self, value: Option<Scaled>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.scaled(value);
            }
            None => self.u8(0),
        }
    }

    fn fonts(&mut self, fonts: &[FontResource]) {
        self.collection_len(fonts.len());
        for font in fonts {
            if self.error.is_some() {
                return;
            }
            self.u32(font.font_id);
            self.str(&font.name);
            self.hash(font.tfm_content_hash);
            self.u32(font.tfm_checksum);
            self.scaled(font.design_size);
            self.scaled(font.at_size);
            match &font.opentype {
                Some(opentype) => {
                    self.u8(1);
                    self.raw(&opentype.program_identity.bytes());
                    self.raw(&opentype.object_identity.bytes());
                    self.raw(&opentype.instance_identity.bytes());
                    self.u8(opentype.container as u8);
                }
                None => self.u8(0),
            }
            self.raw(&font.semantic_identity.bytes());
            match font.construction {
                FontResourceConstruction::Loaded => self.u8(wire::font_construction::LOADED),
                FontResourceConstruction::Copied {
                    source_font_id,
                    source_identity,
                } => {
                    self.u8(wire::font_construction::COPIED);
                    self.u32(source_font_id);
                    self.raw(&source_identity.bytes());
                }
                FontResourceConstruction::Letterspaced {
                    source_font_id,
                    source_identity,
                    amount,
                    no_ligatures,
                } => {
                    self.u8(wire::font_construction::LETTERSPACED);
                    self.u32(source_font_id);
                    self.raw(&source_identity.bytes());
                    self.raw(&amount.to_le_bytes());
                    self.u8(u8::from(no_ligatures));
                }
                FontResourceConstruction::Expanded {
                    source_font_id,
                    source_identity,
                    ratio,
                } => {
                    self.u8(wire::font_construction::EXPANDED);
                    self.u32(source_font_id);
                    self.raw(&source_identity.bytes());
                    self.raw(&ratio.to_le_bytes());
                }
            }
        }
    }

    fn effects(&mut self, effects: &[PageEffect]) {
        self.collection_len(effects.len());
        for effect in effects {
            if self.error.is_some() {
                return;
            }
            match effect {
                PageEffect::OpenOut { stream, path } => {
                    self.u8(wire::effect::OPEN_OUT);
                    self.u8(*stream);
                    self.str(path);
                }
                PageEffect::CloseOut { stream } => {
                    self.u8(wire::effect::CLOSE_OUT);
                    self.u8(*stream);
                }
                PageEffect::Write { sink, text } => {
                    self.u8(wire::effect::WRITE);
                    self.sink(*sink);
                    self.str(text);
                }
                PageEffect::Special { class, payload } => {
                    self.u8(wire::effect::SPECIAL);
                    self.str(class);
                    self.bytes(payload);
                }
                PageEffect::PdfAccessibility(control) => {
                    self.u8(wire::effect::PDF_ACCESSIBILITY);
                    self.u8(match control {
                        PdfAccessibilityEffect::InterwordSpaceOn => 0,
                        PdfAccessibilityEffect::InterwordSpaceOff => 1,
                        PdfAccessibilityEffect::FakeSpace => 2,
                    });
                }
                PageEffect::PdfAnnotation(marker) => {
                    self.u8(wire::effect::PDF_ANNOTATION);
                    match marker {
                        PdfAnnotationEffect::Annotation { object } => {
                            self.u8(0);
                            self.u32(*object);
                        }
                        PdfAnnotationEffect::LinkStart { object } => {
                            self.u8(1);
                            self.u32(*object);
                        }
                        PdfAnnotationEffect::LinkEnd { object } => {
                            self.u8(2);
                            self.u32(*object);
                        }
                        PdfAnnotationEffect::RunningLink(enabled) => {
                            self.u8(3);
                            self.u8(u8::from(*enabled));
                        }
                    }
                }
            }
        }
    }

    fn sink(&mut self, sink: EffectSink) {
        match sink {
            EffectSink::Terminal => self.u8(wire::sink::TERMINAL),
            EffectSink::Log => self.u8(wire::sink::LOG),
            EffectSink::TerminalAndLog => self.u8(wire::sink::TERMINAL_AND_LOG),
            EffectSink::Stream(stream) => {
                self.u8(wire::sink::STREAM);
                self.u8(stream);
            }
        }
    }

    fn node(&mut self, node: &PageNode) {
        let mut tasks = Vec::new();
        self.write_node(node, 1, &mut tasks);
        while let Some(task) = tasks.pop() {
            if self.error.is_some() {
                return;
            }
            match task {
                WriteTask::Node(node, depth) => self.write_node(node, depth, &mut tasks),
                WriteTask::NodeList(nodes, depth) => {
                    self.collection_len(nodes.len());
                    if self.error.is_some() {
                        continue;
                    }
                    tasks.extend(nodes.iter().rev().map(|node| WriteTask::Node(node, depth)));
                }
            }
        }
    }

    fn write_node<'a>(&mut self, node: &'a PageNode, depth: usize, tasks: &mut Vec<WriteTask<'a>>) {
        if self.error.is_some() {
            return;
        }
        if depth > self.limits.max_depth {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::Depth,
                actual: depth,
                limit: self.limits.max_depth,
            });
            return;
        }
        self.nodes_seen += 1;
        if self.nodes_seen > self.limits.max_nodes {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::Nodes,
                actual: self.nodes_seen,
                limit: self.limits.max_nodes,
            });
            return;
        }
        self.node_head(node, depth, tasks);
    }

    fn node_head<'a>(&mut self, node: &'a PageNode, depth: usize, tasks: &mut Vec<WriteTask<'a>>) {
        match node {
            PageNode::Char { font_id, ch, width } => {
                let mut bytes = [0; 13];
                bytes[0] = wire::node::CHAR;
                bytes[1..5].copy_from_slice(&font_id.to_le_bytes());
                bytes[5..9].copy_from_slice(&ch.to_le_bytes());
                bytes[9..13].copy_from_slice(&width.raw().to_le_bytes());
                self.raw(&bytes);
            }
            PageNode::Lig {
                font_id,
                ch,
                source,
                width,
            } => {
                let mut bytes = [0; 17];
                bytes[0] = wire::node::LIG;
                bytes[1..5].copy_from_slice(&font_id.to_le_bytes());
                bytes[5..9].copy_from_slice(&ch.to_le_bytes());
                bytes[9..13].copy_from_slice(&width.raw().to_le_bytes());
                bytes[13..17].copy_from_slice(&(source.len() as u32).to_le_bytes());
                self.raw(&bytes);
                for code in source {
                    self.u32(*code);
                }
            }
            PageNode::Kern { amount, kind } => {
                let mut bytes = [0; 6];
                bytes[0] = wire::node::KERN;
                bytes[1..5].copy_from_slice(&amount.raw().to_le_bytes());
                bytes[5] = kern_kind_tag(*kind);
                self.raw(&bytes);
            }
            PageNode::Glue { spec, kind, leader } => {
                self.u8(wire::node::GLUE);
                self.glue_spec(*spec);
                self.u8(glue_kind_tag(*kind));
                match leader {
                    None => self.u8(wire::leader::NONE),
                    Some(LeaderPayload::HList(box_node)) => {
                        self.u8(wire::leader::HLIST);
                        self.box_fields(box_node);
                        tasks.push(WriteTask::NodeList(&box_node.children, depth + 1));
                    }
                    Some(LeaderPayload::VList(box_node)) => {
                        self.u8(wire::leader::VLIST);
                        self.box_fields(box_node);
                        tasks.push(WriteTask::NodeList(&box_node.children, depth + 1));
                    }
                    Some(LeaderPayload::Rule {
                        width,
                        height,
                        depth,
                    }) => {
                        self.u8(wire::leader::RULE);
                        self.optional_scaled(*width);
                        self.optional_scaled(*height);
                        self.optional_scaled(*depth);
                    }
                }
            }
            PageNode::Penalty(value) => {
                self.tagged_i32(wire::node::PENALTY, *value);
            }
            PageNode::Rule {
                width,
                height,
                depth,
            } => {
                self.u8(wire::node::RULE);
                self.optional_scaled(*width);
                self.optional_scaled(*height);
                self.optional_scaled(*depth);
            }
            PageNode::HList(box_node) => {
                self.u8(wire::node::HLIST);
                self.box_fields(box_node);
                tasks.push(WriteTask::NodeList(&box_node.children, depth + 1));
            }
            PageNode::VList(box_node) => {
                self.u8(wire::node::VLIST);
                self.box_fields(box_node);
                tasks.push(WriteTask::NodeList(&box_node.children, depth + 1));
            }
            PageNode::Disc {
                kind,
                pre,
                post,
                replace,
            } => {
                self.u8(wire::node::DISC);
                self.u8(disc_kind_tag(*kind));
                tasks.push(WriteTask::NodeList(replace, depth + 1));
                tasks.push(WriteTask::NodeList(post, depth + 1));
                tasks.push(WriteTask::NodeList(pre, depth + 1));
            }
            PageNode::Mark { class, tokens } => {
                self.u8(wire::node::MARK);
                self.u16(*class);
                self.tokens(tokens);
            }
            PageNode::Insert { class, content } => {
                self.u8(wire::node::INSERT);
                self.u16(*class);
                tasks.push(WriteTask::NodeList(content, depth + 1));
            }
            PageNode::WhatsitAnchor { effect_index } => {
                self.tagged_u32(wire::node::WHATSIT_ANCHOR, *effect_index);
            }
            PageNode::MathOn(width) => {
                self.tagged_i32(wire::node::MATH_ON, width.raw());
            }
            PageNode::MathOff(width) => {
                self.tagged_i32(wire::node::MATH_OFF, width.raw());
            }
            PageNode::Adjust(content) => {
                self.u8(wire::node::ADJUST);
                tasks.push(WriteTask::NodeList(content, depth + 1));
            }
        }
    }

    fn tokens(&mut self, tokens: &[PageToken]) {
        self.collection_len(tokens.len());
        for token in tokens {
            if self.error.is_some() {
                return;
            }
            match token {
                PageToken::Char { ch, cat } => {
                    self.u8(wire::token::CHAR);
                    self.u32(*ch);
                    self.u8(token_catcode_tag(*cat));
                }
                PageToken::ControlSequence(name) => {
                    self.u8(wire::token::CONTROL_SEQUENCE);
                    self.str(name);
                }
                PageToken::Param(slot) => {
                    self.u8(wire::token::PARAM);
                    self.u8(*slot);
                }
                PageToken::ActiveControlSequence(ch) => {
                    self.u8(wire::token::ACTIVE_CONTROL_SEQUENCE);
                    self.u32(*ch);
                }
            }
        }
    }

    fn tagged_i32(&mut self, tag: u8, value: i32) {
        let mut bytes = [0; 5];
        bytes[0] = tag;
        bytes[1..].copy_from_slice(&value.to_le_bytes());
        self.raw(&bytes);
    }

    fn tagged_u32(&mut self, tag: u8, value: u32) {
        let mut bytes = [0; 5];
        bytes[0] = tag;
        bytes[1..].copy_from_slice(&value.to_le_bytes());
        self.raw(&bytes);
    }

    fn box_fields(&mut self, box_node: &BoxNode) {
        self.scaled(box_node.width);
        self.scaled(box_node.height);
        self.scaled(box_node.depth);
        self.scaled(box_node.shift);
        self.i32(box_node.glue_set.numerator());
        self.i32(box_node.glue_set.denominator());
        self.u8(glue_sign_tag(box_node.glue_sign));
        self.u8(glue_order_tag(box_node.glue_order));
    }

    fn glue_spec(&mut self, spec: GlueSpec) {
        self.scaled(spec.width);
        self.scaled(spec.stretch);
        self.u8(glue_order_tag(spec.stretch_order));
        self.scaled(spec.shrink);
        self.u8(glue_order_tag(spec.shrink_order));
    }
}

enum WriteTask<'a> {
    Node(&'a PageNode, usize),
    NodeList(&'a [PageNode], usize),
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
    limits: ArtifactCodecLimits,
    nodes_seen: usize,
    collection_items_seen: usize,
}

impl Reader<'_> {
    fn new(bytes: &[u8], limits: ArtifactCodecLimits) -> Reader<'_> {
        Self::new_at(bytes, limits, 0)
    }

    fn new_at(bytes: &[u8], limits: ArtifactCodecLimits, offset: usize) -> Reader<'_> {
        Reader {
            bytes,
            offset,
            limits,
            nodes_seen: 0,
            collection_items_seen: 0,
        }
    }

    fn header(&mut self) -> Result<(u8, crate::JobInfo, Vec<FontResource>, [i32; 10]), ParseError> {
        self.expect_magic()?;
        let version = self.u8()?;
        if version != VERSION
            && version != PRE_ANNOTATION_VERSION
            && version != PDF_ACCESSIBILITY_VERSION
            && version != FONT_CONSTRUCTION_VERSION
            && version != OPENTYPE_FONT_VERSION
            && version != LEGACY_VERSION
        {
            return Err(ParseError::UnsupportedVersion(version));
        }
        let job = crate::JobInfo {
            mag: self.i32()?,
            banner: self.str()?,
            h_offset: self.scaled()?,
            v_offset: self.scaled()?,
        };
        let fonts = self.fonts(version)?;
        let mut counts = [0; 10];
        for value in &mut counts {
            *value = self.i32()?;
        }
        Ok((version, job, fonts, counts))
    }

    fn expect_magic(&mut self) -> Result<(), ParseError> {
        let magic = self.take(MAGIC.len())?;
        if magic == MAGIC {
            Ok(())
        } else {
            Err(ParseError::InvalidMagic)
        }
    }

    fn finish(&self) -> Result<(), ParseError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(ParseError::TrailingBytes {
                offset: self.offset,
                len: self.bytes.len(),
            })
        }
    }

    fn take(&mut self, len: usize) -> Result<&[u8], ParseError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(ParseError::LengthOverflow)?;
        if end > self.bytes.len() {
            return Err(ParseError::UnexpectedEof);
        }
        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, ParseError> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(bytes))
    }

    fn u16(&mut self) -> Result<u16, ParseError> {
        let mut bytes = [0; 2];
        bytes.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(bytes))
    }

    fn i32(&mut self) -> Result<i32, ParseError> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.take(4)?);
        Ok(i32::from_le_bytes(bytes))
    }

    fn scaled(&mut self) -> Result<Scaled, ParseError> {
        Ok(Scaled::from_raw(self.i32()?))
    }

    fn len(&mut self) -> Result<usize, ParseError> {
        usize::try_from(self.u32()?).map_err(|_| ParseError::LengthOverflow)
    }

    fn collection_len(&mut self, min_item_bytes: usize) -> Result<usize, ParseError> {
        let len = self.len()?;
        if len > self.limits.max_collection_len {
            return Err(ParseError::LimitExceeded {
                kind: CodecLimitKind::CollectionLength,
                actual: len,
                limit: self.limits.max_collection_len,
            });
        }
        self.collection_items_seen = self
            .collection_items_seen
            .checked_add(len)
            .ok_or(ParseError::LengthOverflow)?;
        if self.collection_items_seen > self.limits.max_collection_items {
            return Err(ParseError::LimitExceeded {
                kind: CodecLimitKind::CollectionItems,
                actual: self.collection_items_seen,
                limit: self.limits.max_collection_items,
            });
        }
        let minimum_bytes = len
            .checked_mul(min_item_bytes)
            .ok_or(ParseError::LengthOverflow)?;
        if minimum_bytes > self.bytes.len() - self.offset {
            return Err(ParseError::UnexpectedEof);
        }
        Ok(len)
    }

    fn bytes(&mut self) -> Result<Vec<u8>, ParseError> {
        let len = self.len()?;
        if len > self.limits.max_collection_len {
            return Err(ParseError::LimitExceeded {
                kind: CodecLimitKind::CollectionLength,
                actual: len,
                limit: self.limits.max_collection_len,
            });
        }
        Ok(self.take(len)?.to_vec())
    }

    fn str(&mut self) -> Result<String, ParseError> {
        String::from_utf8(self.bytes()?).map_err(|_| ParseError::InvalidUtf8)
    }

    fn hash(&mut self) -> Result<ContentHash, ParseError> {
        let mut bytes = [0; 32];
        bytes.copy_from_slice(self.take(32)?);
        Ok(ContentHash::new(bytes))
    }

    fn optional_scaled(&mut self) -> Result<Option<Scaled>, ParseError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.scaled()?)),
            tag => Err(ParseError::InvalidTag {
                kind: "optional scaled",
                tag,
            }),
        }
    }

    fn fonts(&mut self, version: u8) -> Result<Vec<FontResource>, ParseError> {
        let len = self.collection_len(if version >= FONT_CONSTRUCTION_VERSION {
            125
        } else if version >= OPENTYPE_FONT_VERSION {
            53
        } else {
            52
        })?;
        let mut fonts = Vec::with_capacity(len);
        for _ in 0..len {
            let font_id = self.u32()?;
            let name = self.str()?;
            let tfm_content_hash = self.hash()?;
            let tfm_checksum = self.u32()?;
            let design_size = self.scaled()?;
            let at_size = self.scaled()?;
            let opentype = if version >= OPENTYPE_FONT_VERSION {
                match self.u8()? {
                    0 => None,
                    1 => {
                        let program_identity =
                            tex_fonts::FontProgramIdentity::from_bytes(self.identity()?);
                        let object_identity =
                            tex_fonts::FontObjectIdentity::from_bytes(self.identity()?);
                        let instance_identity =
                            tex_fonts::FontInstanceIdentity::from_bytes(self.identity()?);
                        let container = match self.u8()? {
                            1 => tex_fonts::FontContainer::OpenType,
                            2 => tex_fonts::FontContainer::TrueType,
                            3 => tex_fonts::FontContainer::Collection,
                            4 => tex_fonts::FontContainer::Woff2,
                            tag => {
                                return Err(ParseError::InvalidTag {
                                    kind: "font container",
                                    tag,
                                });
                            }
                        };
                        Some(crate::OpenTypeFontResource {
                            program_identity,
                            object_identity,
                            instance_identity,
                            container,
                        })
                    }
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "optional OpenType font",
                            tag,
                        });
                    }
                }
            } else {
                None
            };
            let (semantic_identity, construction) = if version >= FONT_CONSTRUCTION_VERSION {
                let semantic_identity = tex_fonts::FontSourceIdentity::from_bytes(self.identity()?);
                let tag = self.u8()?;
                let construction = match tag {
                    wire::font_construction::LOADED => FontResourceConstruction::Loaded,
                    wire::font_construction::COPIED => FontResourceConstruction::Copied {
                        source_font_id: self.u32()?,
                        source_identity: tex_fonts::FontSourceIdentity::from_bytes(
                            self.identity()?,
                        ),
                    },
                    wire::font_construction::LETTERSPACED => {
                        FontResourceConstruction::Letterspaced {
                            source_font_id: self.u32()?,
                            source_identity: tex_fonts::FontSourceIdentity::from_bytes(
                                self.identity()?,
                            ),
                            amount: self.u16()? as i16,
                            no_ligatures: match self.u8()? {
                                0 => false,
                                1 => true,
                                tag => {
                                    return Err(ParseError::InvalidTag {
                                        kind: "letterspace no-ligatures flag",
                                        tag,
                                    });
                                }
                            },
                        }
                    }
                    wire::font_construction::EXPANDED => FontResourceConstruction::Expanded {
                        source_font_id: self.u32()?,
                        source_identity: tex_fonts::FontSourceIdentity::from_bytes(
                            self.identity()?,
                        ),
                        ratio: self.u16()? as i16,
                    },
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "font construction",
                            tag,
                        });
                    }
                };
                (semantic_identity, construction)
            } else {
                (
                    tex_fonts::FontSourceIdentity::from_bytes([0; 32]),
                    FontResourceConstruction::Loaded,
                )
            };
            fonts.push(FontResource {
                font_id,
                name,
                tfm_content_hash,
                tfm_checksum,
                design_size,
                at_size,
                opentype,
                semantic_identity,
                construction,
            });
        }
        Ok(fonts)
    }

    fn identity(&mut self) -> Result<[u8; 32], ParseError> {
        self.take(32)?
            .try_into()
            .map_err(|_| ParseError::UnexpectedEof)
    }

    fn effects(&mut self, version: u8) -> Result<Vec<PageEffect>, ParseError> {
        let len = self.collection_len(2)?;
        let mut effects = Vec::with_capacity(len);
        for _ in 0..len {
            let tag = self.u8()?;
            effects.push(match tag {
                wire::effect::OPEN_OUT => PageEffect::OpenOut {
                    stream: self.u8()?,
                    path: self.str()?,
                },
                wire::effect::CLOSE_OUT => PageEffect::CloseOut { stream: self.u8()? },
                wire::effect::WRITE => PageEffect::Write {
                    sink: self.sink()?,
                    text: self.str()?,
                },
                wire::effect::SPECIAL => PageEffect::Special {
                    class: self.str()?,
                    payload: self.bytes()?,
                },
                wire::effect::PDF_ACCESSIBILITY if version >= PDF_ACCESSIBILITY_VERSION => {
                    PageEffect::PdfAccessibility(match self.u8()? {
                        0 => PdfAccessibilityEffect::InterwordSpaceOn,
                        1 => PdfAccessibilityEffect::InterwordSpaceOff,
                        2 => PdfAccessibilityEffect::FakeSpace,
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF accessibility effect",
                                tag,
                            });
                        }
                    })
                }
                wire::effect::PDF_ANNOTATION if version >= VERSION => {
                    PageEffect::PdfAnnotation(match self.u8()? {
                        0 => PdfAnnotationEffect::Annotation {
                            object: self.u32()?,
                        },
                        1 => PdfAnnotationEffect::LinkStart {
                            object: self.u32()?,
                        },
                        2 => PdfAnnotationEffect::LinkEnd {
                            object: self.u32()?,
                        },
                        3 => PdfAnnotationEffect::RunningLink(match self.u8()? {
                            0 => false,
                            1 => true,
                            tag => {
                                return Err(ParseError::InvalidTag {
                                    kind: "PDF running-link boolean",
                                    tag,
                                });
                            }
                        }),
                        tag => {
                            return Err(ParseError::InvalidTag {
                                kind: "PDF annotation effect",
                                tag,
                            });
                        }
                    })
                }
                tag => {
                    return Err(ParseError::InvalidTag {
                        kind: "effect",
                        tag,
                    });
                }
            });
        }
        Ok(effects)
    }

    fn sink(&mut self) -> Result<EffectSink, ParseError> {
        match self.u8()? {
            wire::sink::TERMINAL => Ok(EffectSink::Terminal),
            wire::sink::LOG => Ok(EffectSink::Log),
            wire::sink::TERMINAL_AND_LOG => Ok(EffectSink::TerminalAndLog),
            wire::sink::STREAM => Ok(EffectSink::Stream(self.u8()?)),
            tag => Err(ParseError::InvalidTag { kind: "sink", tag }),
        }
    }

    fn node(&mut self) -> Result<PageNode, ParseError> {
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

            let mut completed = match self.node_head()? {
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

    fn skip_node(&mut self) -> Result<(), ParseError> {
        let mut frames = Vec::new();
        loop {
            let depth = frames.len() + 1;
            self.begin_node(depth)?;
            if let Some(frame) = self.skip_node_head()? {
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

    fn begin_node(&mut self, depth: usize) -> Result<(), ParseError> {
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

    fn skip_node_head(&mut self) -> Result<Option<SkipFrame>, ParseError> {
        let tag = self.u8()?;
        let children = match tag {
            wire::node::CHAR => {
                self.u32()?;
                self.u32()?;
                self.scaled()?;
                None
            }
            wire::node::LIG => {
                self.u32()?;
                self.u32()?;
                self.scaled()?;
                let count = self.u32()? as usize;
                if count == 0 || count > 63 {
                    return Err(ParseError::Validation(
                        crate::ArtifactValidationError::InvalidLigatureSourceLength { count },
                    ));
                }
                for _ in 0..count {
                    self.u32()?;
                }
                None
            }
            wire::node::KERN => {
                self.scaled()?;
                parse_kern_kind(self.u8()?)?;
                None
            }
            wire::node::GLUE => {
                self.glue_spec()?;
                parse_glue_kind(self.u8()?)?;
                match self.u8()? {
                    wire::leader::NONE => None,
                    wire::leader::HLIST | wire::leader::VLIST => {
                        self.box_fields()?;
                        Some(SkipFrame::List(self.collection_len(5)?))
                    }
                    wire::leader::RULE => {
                        self.optional_scaled()?;
                        self.optional_scaled()?;
                        self.optional_scaled()?;
                        None
                    }
                    tag => {
                        return Err(ParseError::InvalidTag {
                            kind: "leader payload",
                            tag,
                        });
                    }
                }
            }
            wire::node::PENALTY => {
                self.i32()?;
                None
            }
            wire::node::RULE => {
                self.optional_scaled()?;
                self.optional_scaled()?;
                self.optional_scaled()?;
                None
            }
            wire::node::HLIST | wire::node::VLIST => {
                self.box_fields()?;
                Some(SkipFrame::List(self.collection_len(5)?))
            }
            wire::node::WHATSIT_ANCHOR => {
                self.u32()?;
                None
            }
            wire::node::MATH_ON | wire::node::MATH_OFF => {
                self.scaled()?;
                None
            }
            wire::node::DISC => {
                parse_disc_kind(self.u8()?)?;
                Some(SkipFrame::Disc {
                    phase: 0,
                    remaining: self.collection_len(5)?,
                })
            }
            wire::node::MARK => {
                self.u16()?;
                self.tokens()?;
                None
            }
            wire::node::INSERT => {
                self.u16()?;
                Some(SkipFrame::List(self.collection_len(5)?))
            }
            wire::node::ADJUST => Some(SkipFrame::List(self.collection_len(5)?)),
            tag => return Err(ParseError::InvalidTag { kind: "node", tag }),
        };
        if let Some(mut frame) = children
            && frame.ready(self)?
        {
            return Ok(Some(frame));
        }
        Ok(None)
    }

    fn node_head(&mut self) -> Result<ParsedNode, ParseError> {
        let tag = self.u8()?;
        match tag {
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
                            children: Vec::with_capacity(remaining),
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
                    children: Vec::with_capacity(remaining),
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
                    pre: Vec::with_capacity(remaining),
                    post: Vec::new(),
                    replace: Vec::new(),
                    remaining,
                }))
            }
            wire::node::MARK => Ok(ParsedNode::Complete(PageNode::Mark {
                class: self.u16()?,
                tokens: self.tokens()?,
            })),
            wire::node::INSERT => {
                let class = self.u16()?;
                let remaining = self.collection_len(5)?;
                Ok(ParsedNode::Frame(DecodeFrame::Insert {
                    class,
                    content: Vec::with_capacity(remaining),
                    remaining,
                }))
            }
            wire::node::ADJUST => {
                let remaining = self.collection_len(5)?;
                Ok(ParsedNode::Frame(DecodeFrame::Adjust {
                    content: Vec::with_capacity(remaining),
                    remaining,
                }))
            }
            tag => Err(ParseError::InvalidTag { kind: "node", tag }),
        }
    }

    fn tokens(&mut self) -> Result<Vec<PageToken>, ParseError> {
        let len = self.collection_len(2)?;
        let mut tokens = Vec::with_capacity(len);
        for _ in 0..len {
            tokens.push(match self.u8()? {
                wire::token::CHAR => PageToken::Char {
                    ch: self.u32()?,
                    cat: parse_token_catcode(self.u8()?)?,
                },
                wire::token::CONTROL_SEQUENCE => PageToken::ControlSequence(self.str()?),
                wire::token::PARAM => PageToken::Param(self.u8()?),
                wire::token::ACTIVE_CONTROL_SEQUENCE => {
                    PageToken::ActiveControlSequence(self.u32()?)
                }
                tag => {
                    return Err(ParseError::InvalidTag { kind: "token", tag });
                }
            });
        }
        Ok(tokens)
    }

    fn box_fields(&mut self) -> Result<BoxFields, ParseError> {
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

    fn glue_spec(&mut self) -> Result<GlueSpec, ParseError> {
        Ok(GlueSpec {
            width: self.scaled()?,
            stretch: self.scaled()?,
            stretch_order: parse_glue_order(self.u8()?)?,
            shrink: self.scaled()?,
            shrink_order: parse_glue_order(self.u8()?)?,
        })
    }
}

struct BoxFields {
    width: Scaled,
    height: Scaled,
    depth: Scaled,
    shift: Scaled,
    glue_set: GlueSetRatio,
    glue_sign: GlueSign,
    glue_order: GlueOrder,
}

impl BoxFields {
    fn finish(self, children: Vec<PageNode>) -> BoxNode {
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

fn validate_streamed_char(
    fonts: &std::collections::BTreeSet<u32>,
    font_id: u32,
    ch: u32,
) -> Result<(), ParseError> {
    if !fonts.contains(&font_id) {
        return Err(ParseError::Validation(
            crate::ArtifactValidationError::MissingFont { font_id },
        ));
    }
    validate_streamed_scalar(ch)
}

fn validate_streamed_scalar(ch: u32) -> Result<(), ParseError> {
    if ch > u32::from(u8::MAX) {
        return Err(ParseError::Validation(
            crate::ArtifactValidationError::CharacterOutOfRange { ch },
        ));
    }
    Ok(())
}

fn glue_order_tag(order: GlueOrder) -> u8 {
    match order {
        GlueOrder::Normal => 0,
        GlueOrder::Fil => 1,
        GlueOrder::Fill => 2,
        GlueOrder::Filll => 3,
    }
}

fn parse_glue_order(tag: u8) -> Result<GlueOrder, ParseError> {
    match tag {
        0 => Ok(GlueOrder::Normal),
        1 => Ok(GlueOrder::Fil),
        2 => Ok(GlueOrder::Fill),
        3 => Ok(GlueOrder::Filll),
        tag => Err(ParseError::InvalidTag {
            kind: "glue order",
            tag,
        }),
    }
}

fn glue_sign_tag(sign: GlueSign) -> u8 {
    match sign {
        GlueSign::Normal => 0,
        GlueSign::Stretching => 1,
        GlueSign::Shrinking => 2,
    }
}

fn parse_glue_sign(tag: u8) -> Result<GlueSign, ParseError> {
    match tag {
        0 => Ok(GlueSign::Normal),
        1 => Ok(GlueSign::Stretching),
        2 => Ok(GlueSign::Shrinking),
        tag => Err(ParseError::InvalidTag {
            kind: "glue sign",
            tag,
        }),
    }
}

fn kern_kind_tag(kind: KernKind) -> u8 {
    match kind {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Accent => 2,
        KernKind::LeftMargin => 3,
        KernKind::RightMargin => 4,
        KernKind::Auto => 5,
    }
}

fn parse_kern_kind(tag: u8) -> Result<KernKind, ParseError> {
    match tag {
        0 => Ok(KernKind::Explicit),
        1 => Ok(KernKind::Font),
        2 => Ok(KernKind::Accent),
        3 => Ok(KernKind::LeftMargin),
        4 => Ok(KernKind::RightMargin),
        5 => Ok(KernKind::Auto),
        tag => Err(ParseError::InvalidTag {
            kind: "kern kind",
            tag,
        }),
    }
}

fn disc_kind_tag(kind: DiscKind) -> u8 {
    match kind {
        DiscKind::Discretionary => 0,
        DiscKind::ExplicitHyphen => 1,
        DiscKind::AutomaticHyphen => 2,
    }
}

fn parse_disc_kind(tag: u8) -> Result<DiscKind, ParseError> {
    match tag {
        0 => Ok(DiscKind::Discretionary),
        1 => Ok(DiscKind::ExplicitHyphen),
        2 => Ok(DiscKind::AutomaticHyphen),
        tag => Err(ParseError::InvalidTag {
            kind: "disc kind",
            tag,
        }),
    }
}

fn glue_kind_tag(kind: GlueKind) -> u8 {
    match kind {
        GlueKind::Normal => 0,
        GlueKind::BaselineSkip => 1,
        GlueKind::LineSkip => 2,
        GlueKind::LeftSkip => 3,
        GlueKind::RightSkip => 4,
        GlueKind::ParFillSkip => 5,
        GlueKind::Leaders => 6,
        GlueKind::Cleaders => 7,
        GlueKind::Xleaders => 8,
    }
}

fn token_catcode_tag(cat: TokenCatcode) -> u8 {
    match cat {
        TokenCatcode::Escape => 0,
        TokenCatcode::BeginGroup => 1,
        TokenCatcode::EndGroup => 2,
        TokenCatcode::MathShift => 3,
        TokenCatcode::AlignmentTab => 4,
        TokenCatcode::EndLine => 5,
        TokenCatcode::Parameter => 6,
        TokenCatcode::Superscript => 7,
        TokenCatcode::Subscript => 8,
        TokenCatcode::Ignored => 9,
        TokenCatcode::Space => 10,
        TokenCatcode::Letter => 11,
        TokenCatcode::Other => 12,
        TokenCatcode::Active => 13,
        TokenCatcode::Comment => 14,
        TokenCatcode::Invalid => 15,
    }
}

fn parse_token_catcode(tag: u8) -> Result<TokenCatcode, ParseError> {
    match tag {
        0 => Ok(TokenCatcode::Escape),
        1 => Ok(TokenCatcode::BeginGroup),
        2 => Ok(TokenCatcode::EndGroup),
        3 => Ok(TokenCatcode::MathShift),
        4 => Ok(TokenCatcode::AlignmentTab),
        5 => Ok(TokenCatcode::EndLine),
        6 => Ok(TokenCatcode::Parameter),
        7 => Ok(TokenCatcode::Superscript),
        8 => Ok(TokenCatcode::Subscript),
        9 => Ok(TokenCatcode::Ignored),
        10 => Ok(TokenCatcode::Space),
        11 => Ok(TokenCatcode::Letter),
        12 => Ok(TokenCatcode::Other),
        13 => Ok(TokenCatcode::Active),
        14 => Ok(TokenCatcode::Comment),
        15 => Ok(TokenCatcode::Invalid),
        tag => Err(ParseError::InvalidTag {
            kind: "token catcode",
            tag,
        }),
    }
}

fn parse_glue_kind(tag: u8) -> Result<GlueKind, ParseError> {
    match tag {
        0 => Ok(GlueKind::Normal),
        1 => Ok(GlueKind::BaselineSkip),
        2 => Ok(GlueKind::LineSkip),
        3 => Ok(GlueKind::LeftSkip),
        4 => Ok(GlueKind::RightSkip),
        5 => Ok(GlueKind::ParFillSkip),
        6 => Ok(GlueKind::Leaders),
        7 => Ok(GlueKind::Cleaders),
        8 => Ok(GlueKind::Xleaders),
        tag => Err(ParseError::InvalidTag {
            kind: "glue kind",
            tag,
        }),
    }
}

#[cfg(test)]
mod wire_tests {
    use super::wire;

    #[test]
    fn effect_tags_are_unique_and_annotations_use_the_append_only_range() {
        let tags = [
            wire::effect::OPEN_OUT,
            wire::effect::CLOSE_OUT,
            wire::effect::WRITE,
            wire::effect::SPECIAL,
            wire::effect::PDF_ACCESSIBILITY,
            wire::effect::PDF_ANNOTATION,
        ];
        let unique = tags.into_iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), tags.len());
        const { assert!(wire::effect::PDF_ANNOTATION >= 16) };
    }
}
