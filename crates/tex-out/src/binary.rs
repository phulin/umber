use crate::{
    BoxNode, DiscKind, EffectSink, FontResource, FontResourceConstruction, GlueKind, GlueOrder,
    GlueSetRatio, GlueSign, GlueSpec, KernKind, LeaderPayload, MarginKernSide, MathGlyphSelection,
    MathOutputEvent, PageArtifact, PageEffect, PageNode, PageToken, PdfAccessibilityEffect,
    PdfAnnotationEffect, PdfDestinationEffect, PdfDestinationIdentifier, PdfDestinationKind,
    PdfLiteralMode, PdfThreadEffect, TokenCatcode, UnvalidatedPageArtifact,
};
use std::fmt;
use tex_arith::Scaled;

const MAGIC: &[u8; 4] = b"UMPG";
const VERSION: u8 = 24;
mod nodes;
mod owned_decode;
mod owned_encode;
mod primitive;
mod streaming_scan;
mod streaming_writer;
mod wire;

use primitive::{Reader, Writer};
pub(crate) use streaming_scan::{
    V10NodeListReader, V10NodeListSlice, V10PageDecoder, V10StreamLeader, V10StreamNode,
};
pub use streaming_writer::{V10ArtifactBuilder, V10DiscWriter, V10NodeListWriter, V10TokenWriter};
use wire::{
    disc_kind_tag, glue_kind_tag, glue_order_tag, glue_sign_tag, kern_kind_tag, parse_disc_kind,
    parse_glue_kind, parse_glue_order, parse_glue_sign, parse_kern_kind, parse_token_catcode,
    token_catcode_tag,
};

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
    writer.scaled(artifact.job.page_origin_x);
    writer.scaled(artifact.job.page_origin_y);
    writer.scaled(artifact.job.page_width);
    writer.scaled(artifact.job.page_height);
    writer.fonts(&artifact.fonts);
    for value in artifact.counts {
        writer.i32(value);
    }
    writer.node(&artifact.root);
    writer.effects(&artifact.effects);
    writer.math_events(&artifact.math_events);
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
    reader.expect_header()?;
    let mag = reader.i32()?;
    let banner = reader.str()?;
    let h_offset = reader.scaled()?;
    let v_offset = reader.scaled()?;
    let (page_origin_x, page_origin_y, page_width, page_height) = (
        reader.scaled()?,
        reader.scaled()?,
        reader.scaled()?,
        reader.scaled()?,
    );
    let fonts = reader.fonts()?;
    let mut counts = [0; 10];
    for value in &mut counts {
        *value = reader.i32()?;
    }
    let root = reader.node()?;
    let effects = reader.effects()?;
    let math_events = reader.math_events()?;
    reader.finish()?;
    Ok(UnvalidatedPageArtifact {
        job: crate::JobInfo {
            mag,
            banner,
            h_offset,
            v_offset,
            page_origin_x,
            page_origin_y,
            page_width,
            page_height,
        },
        fonts,
        counts,
        root,
        effects,
        math_events,
    })
}
