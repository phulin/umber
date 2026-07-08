use crate::{
    BoxNode, ContentHash, EffectSink, FontResource, GlueKind, GlueOrder, GlueSetRatio, GlueSign,
    GlueSpec, KernKind, PageArtifact, PageEffect, PageNode,
};
use std::fmt;
use tex_arith::Scaled;

const MAGIC: &[u8; 4] = b"UMPG";
const VERSION: u8 = 3;

/// Binary parse failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    InvalidMagic,
    UnsupportedVersion(u8),
    UnexpectedEof,
    TrailingBytes { offset: usize, len: usize },
    InvalidUtf8,
    LengthOverflow,
    InvalidTag { kind: &'static str, tag: u8 },
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
        }
    }
}

impl std::error::Error for ParseError {}

#[must_use]
pub(crate) fn to_bytes(artifact: &PageArtifact) -> Vec<u8> {
    let mut writer = Writer { bytes: Vec::new() };
    writer.bytes.extend_from_slice(MAGIC);
    writer.u8(VERSION);
    writer.i32(artifact.job.mag);
    writer.str(&artifact.job.banner);
    writer.fonts(&artifact.fonts);
    for value in artifact.counts {
        writer.i32(value);
    }
    writer.node(&artifact.root);
    writer.effects(&artifact.effects);
    writer.bytes
}

pub(crate) fn from_bytes(bytes: &[u8]) -> Result<PageArtifact, ParseError> {
    let mut reader = Reader { bytes, offset: 0 };
    reader.expect_magic()?;
    let version = reader.u8()?;
    if version != VERSION {
        return Err(ParseError::UnsupportedVersion(version));
    }
    let mag = reader.i32()?;
    let banner = reader.str()?;
    let fonts = reader.fonts()?;
    let mut counts = [0; 10];
    for value in &mut counts {
        *value = reader.i32()?;
    }
    let root = reader.node()?;
    let effects = reader.effects()?;
    reader.finish()?;
    Ok(PageArtifact {
        job: crate::JobInfo { mag, banner },
        fonts,
        counts,
        root,
        effects,
    })
}

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn scaled(&mut self, value: Scaled) {
        self.i32(value.raw());
    }

    fn len(&mut self, len: usize) {
        self.u32(u32::try_from(len).expect("page artifact length exceeds u32"));
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.bytes.extend_from_slice(bytes);
    }

    fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn hash(&mut self, value: ContentHash) {
        self.bytes.extend_from_slice(&value.bytes());
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
        self.len(fonts.len());
        for font in fonts {
            self.u32(font.font_id);
            self.str(&font.name);
            self.hash(font.tfm_content_hash);
            self.u32(font.tfm_checksum);
            self.scaled(font.design_size);
            self.scaled(font.at_size);
        }
    }

    fn effects(&mut self, effects: &[PageEffect]) {
        self.len(effects.len());
        for effect in effects {
            match effect {
                PageEffect::OpenOut { stream, path } => {
                    self.u8(0);
                    self.u8(*stream);
                    self.str(path);
                }
                PageEffect::CloseOut { stream } => {
                    self.u8(1);
                    self.u8(*stream);
                }
                PageEffect::Write { sink, text } => {
                    self.u8(2);
                    self.sink(*sink);
                    self.str(text);
                }
                PageEffect::Special { class, payload } => {
                    self.u8(3);
                    self.str(class);
                    self.bytes(payload);
                }
            }
        }
    }

    fn sink(&mut self, sink: EffectSink) {
        match sink {
            EffectSink::Terminal => self.u8(0),
            EffectSink::Log => self.u8(1),
            EffectSink::TerminalAndLog => self.u8(2),
            EffectSink::Stream(stream) => {
                self.u8(3);
                self.u8(stream);
            }
        }
    }

    fn node(&mut self, node: &PageNode) {
        match node {
            PageNode::Char { font_id, ch, width } => {
                self.u8(0);
                self.u32(*font_id);
                self.u32(*ch);
                self.scaled(*width);
            }
            PageNode::Lig {
                font_id,
                ch,
                left,
                right,
                width,
            } => {
                self.u8(1);
                self.u32(*font_id);
                self.u32(*ch);
                self.u32(*left);
                self.u32(*right);
                self.scaled(*width);
            }
            PageNode::Kern { amount, kind } => {
                self.u8(2);
                self.scaled(*amount);
                self.u8(kern_kind_tag(*kind));
            }
            PageNode::Glue { spec, kind } => {
                self.u8(3);
                self.glue_spec(*spec);
                self.u8(glue_kind_tag(*kind));
            }
            PageNode::Penalty(value) => {
                self.u8(4);
                self.i32(*value);
            }
            PageNode::Rule {
                width,
                height,
                depth,
            } => {
                self.u8(5);
                self.optional_scaled(*width);
                self.optional_scaled(*height);
                self.optional_scaled(*depth);
            }
            PageNode::HList(box_node) => {
                self.u8(6);
                self.box_node(box_node);
            }
            PageNode::VList(box_node) => {
                self.u8(7);
                self.box_node(box_node);
            }
            PageNode::Unset => self.u8(8),
            PageNode::WhatsitAnchor { effect_index } => {
                self.u8(9);
                self.u32(*effect_index);
            }
            PageNode::MathOn => self.u8(10),
            PageNode::MathOff => self.u8(11),
        }
    }

    fn box_node(&mut self, box_node: &BoxNode) {
        self.scaled(box_node.width);
        self.scaled(box_node.height);
        self.scaled(box_node.depth);
        self.scaled(box_node.shift);
        self.i32(box_node.glue_set.raw);
        self.u8(glue_sign_tag(box_node.glue_sign));
        self.u8(glue_order_tag(box_node.glue_order));
        self.len(box_node.children.len());
        for child in &box_node.children {
            self.node(child);
        }
    }

    fn glue_spec(&mut self, spec: GlueSpec) {
        self.scaled(spec.width);
        self.scaled(spec.stretch);
        self.u8(glue_order_tag(spec.stretch_order));
        self.scaled(spec.shrink);
        self.u8(glue_order_tag(spec.shrink_order));
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Reader<'_> {
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

    fn bytes(&mut self) -> Result<Vec<u8>, ParseError> {
        let len = self.len()?;
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

    fn fonts(&mut self) -> Result<Vec<FontResource>, ParseError> {
        let len = self.len()?;
        let mut fonts = Vec::with_capacity(len);
        for _ in 0..len {
            fonts.push(FontResource {
                font_id: self.u32()?,
                name: self.str()?,
                tfm_content_hash: self.hash()?,
                tfm_checksum: self.u32()?,
                design_size: self.scaled()?,
                at_size: self.scaled()?,
            });
        }
        Ok(fonts)
    }

    fn effects(&mut self) -> Result<Vec<PageEffect>, ParseError> {
        let len = self.len()?;
        let mut effects = Vec::with_capacity(len);
        for _ in 0..len {
            let tag = self.u8()?;
            effects.push(match tag {
                0 => PageEffect::OpenOut {
                    stream: self.u8()?,
                    path: self.str()?,
                },
                1 => PageEffect::CloseOut { stream: self.u8()? },
                2 => PageEffect::Write {
                    sink: self.sink()?,
                    text: self.str()?,
                },
                3 => PageEffect::Special {
                    class: self.str()?,
                    payload: self.bytes()?,
                },
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
            0 => Ok(EffectSink::Terminal),
            1 => Ok(EffectSink::Log),
            2 => Ok(EffectSink::TerminalAndLog),
            3 => Ok(EffectSink::Stream(self.u8()?)),
            tag => Err(ParseError::InvalidTag { kind: "sink", tag }),
        }
    }

    fn node(&mut self) -> Result<PageNode, ParseError> {
        let tag = self.u8()?;
        match tag {
            0 => Ok(PageNode::Char {
                font_id: self.u32()?,
                ch: self.u32()?,
                width: self.scaled()?,
            }),
            1 => Ok(PageNode::Lig {
                font_id: self.u32()?,
                ch: self.u32()?,
                left: self.u32()?,
                right: self.u32()?,
                width: self.scaled()?,
            }),
            2 => Ok(PageNode::Kern {
                amount: self.scaled()?,
                kind: parse_kern_kind(self.u8()?)?,
            }),
            3 => Ok(PageNode::Glue {
                spec: self.glue_spec()?,
                kind: parse_glue_kind(self.u8()?)?,
            }),
            4 => Ok(PageNode::Penalty(self.i32()?)),
            5 => Ok(PageNode::Rule {
                width: self.optional_scaled()?,
                height: self.optional_scaled()?,
                depth: self.optional_scaled()?,
            }),
            6 => Ok(PageNode::HList(self.box_node()?)),
            7 => Ok(PageNode::VList(self.box_node()?)),
            8 => Ok(PageNode::Unset),
            9 => Ok(PageNode::WhatsitAnchor {
                effect_index: self.u32()?,
            }),
            10 => Ok(PageNode::MathOn),
            11 => Ok(PageNode::MathOff),
            tag => Err(ParseError::InvalidTag { kind: "node", tag }),
        }
    }

    fn box_node(&mut self) -> Result<BoxNode, ParseError> {
        let width = self.scaled()?;
        let height = self.scaled()?;
        let depth = self.scaled()?;
        let shift = self.scaled()?;
        let glue_set = GlueSetRatio { raw: self.i32()? };
        let glue_sign = parse_glue_sign(self.u8()?)?;
        let glue_order = parse_glue_order(self.u8()?)?;
        let len = self.len()?;
        let mut children = Vec::with_capacity(len);
        for _ in 0..len {
            children.push(self.node()?);
        }
        Ok(BoxNode {
            width,
            height,
            depth,
            shift,
            glue_set,
            glue_sign,
            glue_order,
            children,
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
    }
}

fn parse_kern_kind(tag: u8) -> Result<KernKind, ParseError> {
    match tag {
        0 => Ok(KernKind::Explicit),
        1 => Ok(KernKind::Font),
        2 => Ok(KernKind::Accent),
        tag => Err(ParseError::InvalidTag {
            kind: "kern kind",
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
