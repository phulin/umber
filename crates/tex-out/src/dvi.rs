use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;

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
pub(crate) mod glue;
mod leaders;
mod movement;
mod opcodes;
mod plan;
mod traversal;

pub use plan::{DviPagePlan, DviPagePlanBuilder};

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
    Sink { message: String },
    InconsistentFontResource { font_id: u32 },
    Artifact { message: String },
    Poisoned,
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
            Self::Sink { message } => write!(f, "failed to write DVI output: {message}"),
            Self::InconsistentFontResource { font_id } => write!(
                f,
                "DVI pages define incompatible resources for font number {font_id}"
            ),
            Self::Artifact { message } => write!(f, "invalid page artifact: {message}"),
            Self::Poisoned => f.write_str("DVI stream cannot continue after an earlier failure"),
        }
    }
}

impl std::error::Error for DviError {}

impl From<crate::ParseError> for DviError {
    fn from(value: crate::ParseError) -> Self {
        Self::Artifact {
            message: value.to_string(),
        }
    }
}

/// Writes a complete DVI file from committed page artifacts.
///
/// The writer is intentionally downstream-only: all DVI preamble data, page
/// counters, dimensions, and font resources come from the artifact stream.
pub fn write_dvi(pages: &[PageArtifact]) -> Result<Vec<u8>, DviError> {
    let mut writer = DviStreamWriter::new(Vec::new());
    for page in pages {
        writer.write_page(page)?;
    }
    writer.finish()
}

/// Incremental DVI emitter that retains at most one encoded page buffer.
pub struct DviStreamWriter<W: Write> {
    writer: DviWriter<W>,
    failed: bool,
}

impl<W: Write> DviStreamWriter<W> {
    #[must_use]
    pub fn new(sink: W) -> Self {
        Self {
            writer: DviWriter::new(sink),
            failed: false,
        }
    }

    pub fn write_page(&mut self, page: &PageArtifact) -> Result<(), DviError> {
        if self.failed {
            return Err(DviError::Poisoned);
        }
        let result = self.write_page_inner(page);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    /// Appends a page whose traversal has already been compiled into DVI body
    /// bytes before the shipout commit boundary.
    pub fn write_page_plan(&mut self, plan: &DviPagePlan) -> Result<(), DviError> {
        if self.failed {
            return Err(DviError::Poisoned);
        }
        let result = self.write_page_plan_inner(plan);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn write_page_inner(&mut self, page: &PageArtifact) -> Result<(), DviError> {
        if self.writer.page_count == u16::MAX {
            return Err(DviError::TooManyPages {
                pages: usize::from(self.writer.page_count) + 1,
            });
        }
        match (&self.writer.job_banner, self.writer.job_mag) {
            (None, None) => {
                self.writer.preamble(&page.job.banner, page.job.mag)?;
                self.writer.job_banner = Some(page.job.banner.clone());
                self.writer.job_mag = Some(page.job.mag);
                self.writer.flush_buffer()?;
            }
            (Some(banner), Some(mag)) if banner == &page.job.banner && mag == page.job.mag => {}
            _ => return Err(DviError::InconsistentJobInfo),
        }
        self.writer.page(page)?;
        self.writer.page_count += 1;
        self.writer.flush_buffer()
    }

    fn write_page_plan_inner(&mut self, plan: &DviPagePlan) -> Result<(), DviError> {
        if self.writer.page_count == u16::MAX {
            return Err(DviError::TooManyPages {
                pages: usize::from(self.writer.page_count) + 1,
            });
        }
        match (&self.writer.job_banner, self.writer.job_mag) {
            (None, None) => {
                self.writer.preamble(plan.banner(), plan.mag())?;
                self.writer.job_banner = Some(plan.banner().to_owned());
                self.writer.job_mag = Some(plan.mag());
                self.writer.flush_buffer()?;
            }
            (Some(banner), Some(mag)) if banner == plan.banner() && mag == plan.mag() => {}
            _ => return Err(DviError::InconsistentJobInfo),
        }
        self.writer.page_plan(plan)?;
        self.writer.page_count += 1;
        self.writer.flush_buffer()
    }

    pub fn finish(mut self) -> Result<W, DviError> {
        if self.failed {
            return Err(DviError::Poisoned);
        }
        if self.writer.page_count == 0 {
            return Err(DviError::NoPages);
        }
        self.writer.postamble()?;
        self.writer.flush_buffer()?;
        Ok(self.writer.sink)
    }
}

struct DviWriter<W: Write> {
    sink: W,
    bytes: Vec<u8>,
    committed_offset: usize,
    fonts: BTreeMap<fonts::FontKey, DefinedFont>,
    fonts_by_number: BTreeMap<u32, fonts::FontKey>,
    page_fonts: BTreeMap<u32, crate::FontResource>,
    job_banner: Option<String>,
    job_mag: Option<i32>,
    page_count: u16,
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
    font_definition_sites: Option<Vec<plan::FontDefinitionSite>>,
}

impl<W: Write> DviWriter<W> {
    fn new(sink: W) -> Self {
        Self {
            sink,
            bytes: Vec::new(),
            committed_offset: 0,
            fonts: BTreeMap::new(),
            fonts_by_number: BTreeMap::new(),
            page_fonts: BTreeMap::new(),
            job_banner: None,
            job_mag: None,
            page_count: 0,
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
            font_definition_sites: None,
        }
    }

    fn flush_buffer(&mut self) -> Result<(), DviError> {
        self.sink
            .write_all(&self.bytes)
            .map_err(|error| DviError::Sink {
                message: error.to_string(),
            })?;
        self.committed_offset = self
            .committed_offset
            .checked_add(self.bytes.len())
            .ok_or(DviError::OffsetOverflow { offset: usize::MAX })?;
        self.bytes.clear();
        Ok(())
    }
}
