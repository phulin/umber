//! Physical-line splitting and TeX line normalization.

use tex_state::SourceId;

use crate::profile::{CharacterCode, CharacterMode};

use super::source::SourceCursor;

/// A half-open range in one immutable registered source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceRange {
    source: SourceId,
    start: u64,
    end: u64,
}

impl SourceRange {
    pub(crate) const fn new(source: SourceId, start: u64, end: u64) -> Self {
        Self { source, start, end }
    }

    /// Source whose immutable bytes are addressed.
    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    /// Inclusive physical byte offset.
    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Exclusive physical byte offset.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.end
    }

    /// Whether this range is a zero-width physical anchor.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Physical source column of the final byte this spelling consumed.
    ///
    /// A reduced `^^` spelling is located at its final physical byte even
    /// though its provenance span covers every byte consumed to produce the
    /// decoded character. Synthetic spellings retain their zero-width physical
    /// anchor.
    ///
    /// TeX82 observes this as `loc - start - 1`, but that equality holds only
    /// while `buffer` still mirrors the source line: tex.web §355 reduces an
    /// expanded code inside a control-sequence name *in place* and shifts the
    /// remainder of the line down by two or three bytes, after which every
    /// `buffer` index on that line is smaller than the source column it came
    /// from. This location is the source column, never the `buffer` index.
    #[must_use]
    pub const fn terminal_location(self) -> SourceLocation {
        SourceLocation {
            source: self.source,
            byte: if self.is_empty() {
                self.start
            } else {
                self.end - 1
            },
        }
    }
}

/// TeX82's canonical physical source location for one delivered spelling.
///
/// This is intentionally separate from [`SourceRange`]: the latter retains
/// every raw source byte that formed a decoded token, while this value is the
/// single source column of the final byte the spelling consumed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceLocation {
    source: SourceId,
    byte: u64,
}

impl SourceLocation {
    /// Constructs one exact physical source location.
    #[must_use]
    pub const fn new(source: SourceId, byte: u64) -> Self {
        Self { source, byte }
    }

    /// Source whose immutable bytes contain this location.
    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    /// Zero-based physical byte offset.
    #[must_use]
    pub const fn byte(self) -> u64 {
        self.byte
    }
}

/// Complete direct-source provenance for one decoded spelling.
///
/// The span keeps raw-source attribution exact; the location keeps TeX82's
/// cursor observation exact. It is carried through backups as ordinary input
/// state and therefore remains stable across executor snapshots.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceProvenance {
    range: SourceRange,
    location: SourceLocation,
}

impl SourceProvenance {
    pub(crate) const fn from_range(range: SourceRange) -> Self {
        Self {
            range,
            location: range.terminal_location(),
        }
    }

    /// Exact raw spelling range.
    #[must_use]
    pub const fn range(self) -> SourceRange {
        self.range
    }

    /// Canonical post-delivery TeX82 location.
    #[must_use]
    pub const fn location(self) -> SourceLocation {
        self.location
    }
}

/// A half-open decoded-scalar range within one normalized physical line.
///
/// Synthetic `endlinechar` occupies one scalar position even though its
/// physical [`SourceRange`] is empty.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceScalarRange {
    start: u64,
    end: u64,
}

impl SourceScalarRange {
    pub(crate) const fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// Inclusive decoded-scalar offset within the normalized line.
    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Exclusive decoded-scalar offset within the normalized line.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.end
    }
}

/// Exact spelling of one physical line terminator.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LineTerminator {
    /// The final physical line reaches end of backing.
    Missing,
    /// A single line-feed byte.
    Lf,
    /// A single carriage-return byte.
    Cr,
    /// A carriage return followed by line feed.
    CrLf,
}

/// Physical metadata retained independently of normalized character delivery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PhysicalLine {
    source: SourceId,
    number: u64,
    content: SourceRange,
    terminator: SourceRange,
    terminator_kind: LineTerminator,
}

impl PhysicalLine {
    /// One-based physical line number within this registered source.
    #[must_use]
    pub const fn number(self) -> u64 {
        self.number
    }

    /// Original content bytes, before trailing-space stripping.
    #[must_use]
    pub const fn content_range(self) -> SourceRange {
        self.content
    }

    /// Original terminator bytes, never included in normalized content.
    #[must_use]
    pub const fn terminator_range(self) -> SourceRange {
        self.terminator
    }

    /// Exact terminator spelling.
    #[must_use]
    pub const fn terminator(self) -> LineTerminator {
        self.terminator_kind
    }
}

/// One character read from a normalized physical line.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceCharacter {
    pub(crate) code: CharacterCode,
    pub(crate) range: SourceRange,
    pub(crate) scalar_offset: u64,
    pub(crate) synthetic: bool,
}

impl SourceCharacter {
    /// Semantic character in the job's fixed byte or Unicode domain.
    #[must_use]
    pub const fn code(self) -> CharacterCode {
        self.code
    }

    /// Exact physical byte range, or a zero-width synthetic anchor.
    #[must_use]
    pub const fn range(self) -> SourceRange {
        self.range
    }

    /// Zero-based scalar delivery offset within the normalized line.
    #[must_use]
    pub const fn scalar_offset(self) -> u64 {
        self.scalar_offset
    }

    /// Whether this is a synthetic `endlinechar`, absent from backing bytes.
    #[must_use]
    pub const fn is_synthetic(self) -> bool {
        self.synthetic
    }
}

/// Snapshot-safe canonical cursor for one normalized line.
///
/// `byte_cursor` is authoritative in both modes. `scalar_cursor` is the
/// operational scalar position advanced alongside decoding; no character
/// vector or independently seekable scalar index exists.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceLineState {
    pub(crate) physical: PhysicalLine,
    pub(crate) retained_end: u64,
    pub(crate) byte_cursor: u64,
    pub(crate) scalar_cursor: u64,
    pub(crate) endline: Option<CharacterCode>,
    pub(crate) endline_delivered: bool,
}

impl SourceLineState {
    pub(crate) fn next_character(
        &mut self,
        mode: CharacterMode,
        bytes: &[u8],
    ) -> Option<SourceCharacter> {
        if self.byte_cursor < self.retained_end {
            let start = self.byte_cursor;
            let (code, width) = match mode {
                CharacterMode::EightBitExact => {
                    let index = usize::try_from(start).ok()?;
                    (CharacterCode::from_byte(*bytes.get(index)?), 1_u64)
                }
                CharacterMode::UnicodeExtended => {
                    let index = usize::try_from(start).ok()?;
                    let text = std::str::from_utf8(bytes.get(index..)?)
                        .expect("Unicode backing was validated at registration");
                    let scalar = text
                        .chars()
                        .next()
                        .expect("cursor before retained end has a scalar");
                    (
                        CharacterCode::from(scalar),
                        u64::try_from(scalar.len_utf8()).expect("UTF-8 width fits u64"),
                    )
                }
            };
            self.byte_cursor += width;
            let scalar_offset = self.scalar_cursor;
            self.scalar_cursor += 1;
            return Some(SourceCharacter {
                code,
                range: SourceRange::new(self.physical.source, start, self.byte_cursor),
                scalar_offset,
                synthetic: false,
            });
        }
        if self.endline_delivered {
            return None;
        }
        self.endline_delivered = true;
        let code = self.endline?;
        let scalar_offset = self.scalar_cursor;
        self.scalar_cursor += 1;
        Some(SourceCharacter {
            code,
            range: SourceRange::new(self.physical.source, self.retained_end, self.retained_end),
            scalar_offset,
            synthetic: true,
        })
    }
}

impl SourceCursor {
    /// Returns the physical terminator of the currently loaded line, if it
    /// has one. The tokenizer uses this only for the generated blank-line
    /// `\par` spelling; ordinary synthetic `endlinechar` remains anchored at
    /// its normalized content end.
    pub(crate) fn current_terminator_range(&self) -> SourceRange {
        self.line.as_ref().map_or_else(
            || {
                SourceRange::new(
                    self.backing.id,
                    self.next_physical_offset,
                    self.next_physical_offset,
                )
            },
            |line| line.physical.terminator,
        )
    }

    pub(crate) fn load_next_line(&mut self, endlinechar: i32) -> Option<&mut SourceLineState> {
        if self.line.is_some() {
            return self.line.as_mut();
        }
        let len = u64::try_from(self.backing.bytes.len()).expect("registration checked length");
        if self.next_physical_offset >= len {
            return None;
        }

        let start = self.next_physical_offset;
        let start_index = usize::try_from(start).expect("backing offset fits usize");
        let tail = &self.backing.bytes[start_index..];
        let relative_terminator = tail.iter().position(|byte| matches!(*byte, b'\r' | b'\n'));
        let (content_end, terminator_end, terminator_kind) = match relative_terminator {
            None => (len, len, LineTerminator::Missing),
            Some(relative) => {
                let offset = start + u64::try_from(relative).expect("offset fits u64");
                let index = usize::try_from(offset).expect("backing offset fits usize");
                if self.backing.bytes[index] == b'\r'
                    && self.backing.bytes.get(index + 1) == Some(&b'\n')
                {
                    (offset, offset + 2, LineTerminator::CrLf)
                } else if self.backing.bytes[index] == b'\r' {
                    (offset, offset + 1, LineTerminator::Cr)
                } else {
                    (offset, offset + 1, LineTerminator::Lf)
                }
            }
        };
        self.next_physical_offset = terminator_end;
        self.end_after_line = terminator_end == len;

        let mut retained_end = content_end;
        while retained_end > start {
            let index = usize::try_from(retained_end - 1).expect("backing offset fits usize");
            if self.backing.bytes[index] != b' ' {
                break;
            }
            retained_end -= 1;
        }
        let endline = endline_character(self.backing.mode, endlinechar);
        let line = PhysicalLine {
            source: self.backing.id,
            number: self.next_line_number,
            content: SourceRange::new(self.backing.id, start, content_end),
            terminator: SourceRange::new(self.backing.id, content_end, terminator_end),
            terminator_kind,
        };
        self.next_line_number += 1;
        self.line = Some(SourceLineState {
            physical: line,
            retained_end,
            byte_cursor: start,
            scalar_cursor: 0,
            endline,
            endline_delivered: false,
        });
        self.lexer_state = super::tokenizer::LexerState::NewLine;
        self.line.as_mut()
    }

    pub(crate) fn finish_line(&mut self) {
        self.line = None;
    }
}

fn endline_character(mode: CharacterMode, endlinechar: i32) -> Option<CharacterCode> {
    match mode {
        CharacterMode::EightBitExact => {
            u8::try_from(endlinechar).ok().map(CharacterCode::from_byte)
        }
        CharacterMode::UnicodeExtended => u32::try_from(endlinechar)
            .ok()
            .and_then(|scalar| CharacterCode::from_unicode_scalar(scalar).ok()),
    }
}

#[cfg(test)]
mod tests;
