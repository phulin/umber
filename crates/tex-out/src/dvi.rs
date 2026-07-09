use std::fmt;

use tex_arith::Scaled;

use fonts::DefinedFont;
use movement::MovementStack;

use crate::PageArtifact;

#[cfg(test)]
mod tests;

pub mod disasm;
mod extent;
mod fonts;
mod framing;
mod glue;
mod leaders;
mod movement;
mod opcodes;
mod traversal;

/// DVI emission failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DviError {
    NoPages,
    EmptyFontName { font_id: u32 },
    FieldTooLong { field: &'static str, len: usize },
    MissingFont { font_id: u32 },
    MissingEffect { effect_index: u32 },
    CharacterOutOfRange { ch: u32 },
    InconsistentJobInfo,
    TooManyPages { pages: usize },
    SpecialTooLong { len: usize },
    OffsetOverflow { offset: usize },
    PositionOverflow,
}

impl fmt::Display for DviError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPages => f.write_str("cannot write DVI without page artifacts"),
            Self::EmptyFontName { font_id } => {
                write!(f, "font resource {font_id} has an empty DVI font name")
            }
            Self::FieldTooLong { field, len } => {
                write!(f, "DVI {field} length {len} exceeds 255 bytes")
            }
            Self::MissingFont { font_id } => {
                write!(f, "page node references missing font resource {font_id}")
            }
            Self::MissingEffect { effect_index } => {
                write!(f, "page node references missing effect {effect_index}")
            }
            Self::CharacterOutOfRange { ch } => {
                write!(f, "DVI TeX82 character code {ch} is outside 0..=255")
            }
            Self::InconsistentJobInfo => {
                f.write_str("page artifacts disagree on job banner or magnification")
            }
            Self::TooManyPages { pages } => write!(f, "DVI page count {pages} exceeds 65535"),
            Self::SpecialTooLong { len } => {
                write!(
                    f,
                    "DVI special payload length {len} exceeds signed 32-bit range"
                )
            }
            Self::OffsetOverflow { offset } => {
                write!(
                    f,
                    "DVI byte offset {offset} exceeds signed 32-bit pointer range"
                )
            }
            Self::PositionOverflow => f.write_str("DVI page position arithmetic overflowed"),
        }
    }
}

impl std::error::Error for DviError {}

/// Writes a complete DVI file from committed page artifacts.
///
/// The writer is intentionally downstream-only: all DVI preamble data, page
/// counters, dimensions, and font resources come from the artifact stream.
pub fn write_dvi(pages: &[PageArtifact]) -> Result<Vec<u8>, DviError> {
    DviWriter::new(pages)?.finish()
}

struct DviWriter<'a> {
    pages: &'a [PageArtifact],
    bytes: Vec<u8>,
    fonts: Vec<DefinedFont<'a>>,
    previous_bop: i32,
    max_height_depth: i32,
    max_width: i32,
    max_stack_depth: u16,
    right_stack: MovementStack,
    down_stack: MovementStack,
    dvi_h: Scaled,
    dvi_v: Scaled,
    cur_h: Scaled,
    cur_v: Scaled,
    dvi_f: Option<u32>,
    cur_s: i32,
}

impl<'a> DviWriter<'a> {
    fn new(pages: &'a [PageArtifact]) -> Result<Self, DviError> {
        let Some(first) = pages.first() else {
            return Err(DviError::NoPages);
        };
        for page in pages {
            if page.job != first.job {
                return Err(DviError::InconsistentJobInfo);
            }
        }
        let page_count = u16::try_from(pages.len())
            .map_err(|_| DviError::TooManyPages { pages: pages.len() })?;
        let mut writer = Self {
            pages,
            bytes: Vec::new(),
            fonts: Vec::new(),
            previous_bop: -1,
            max_height_depth: 0,
            max_width: 0,
            max_stack_depth: 0,
            right_stack: MovementStack::default(),
            down_stack: MovementStack::default(),
            dvi_h: Scaled::from_raw(0),
            dvi_v: Scaled::from_raw(0),
            cur_h: Scaled::from_raw(0),
            cur_v: Scaled::from_raw(0),
            dvi_f: None,
            cur_s: -1,
        };
        writer.preamble(&first.job.banner, first.job.mag)?;
        debug_assert_eq!(page_count as usize, pages.len());
        Ok(writer)
    }

    fn finish(mut self) -> Result<Vec<u8>, DviError> {
        for page in self.pages {
            self.page(page)?;
        }
        self.postamble()?;
        Ok(self.bytes)
    }
}
