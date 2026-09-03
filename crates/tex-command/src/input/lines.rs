//! Physical-line splitting and TeX line normalization.

use std::num::NonZeroU64;

use tex_state::SourceId;

use crate::profile::{CharacterCode, CharacterMode};

use super::source::{LineBackingRegistry, RegisteredSource, SourceCursor};

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

    pub(crate) fn rehome_offsets(&mut self, map: super::source::SourceOffsetMap) {
        self.start = map.map(self.start);
        self.end = map.map(self.end);
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
    range_start: u64,
    range_end: u64,
    location: u64,
    /// `SourceId + 1` supplies the niche used by `Option<SourceProvenance>`.
    /// Every `u32` source id fits, including `u32::MAX`.
    source_plus_one: NonZeroU64,
}

impl SourceProvenance {
    const fn packed_source(source: SourceId) -> NonZeroU64 {
        match NonZeroU64::new(source.raw() as u64 + 1) {
            Some(source) => source,
            None => unreachable!(),
        }
    }

    const fn source(self) -> SourceId {
        SourceId::new((self.source_plus_one.get() - 1) as u32)
    }

    pub(crate) const fn from_range(range: SourceRange) -> Self {
        let location = range.terminal_location();
        Self {
            range_start: range.start(),
            range_end: range.end(),
            location: location.byte(),
            source_plus_one: Self::packed_source(range.source()),
        }
    }

    pub(crate) const fn from_range_and_location(
        range: SourceRange,
        location: SourceLocation,
    ) -> Self {
        assert!(range.source().raw() == location.source().raw());
        Self {
            range_start: range.start(),
            range_end: range.end(),
            location: location.byte(),
            source_plus_one: Self::packed_source(range.source()),
        }
    }

    /// Exact raw spelling range.
    #[must_use]
    pub const fn range(self) -> SourceRange {
        SourceRange::new(self.source(), self.range_start, self.range_end)
    }

    /// Canonical post-delivery TeX82 location.
    #[must_use]
    pub const fn location(self) -> SourceLocation {
        SourceLocation::new(self.source(), self.location)
    }
}

const _: () = assert!(core::mem::size_of::<Option<SourceProvenance>>() == 32);

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
    pub(crate) source: SourceId,
    number: u64,
    content: SourceRange,
    terminator: SourceRange,
    terminator_kind: LineTerminator,
}

impl PhysicalLine {
    pub(crate) fn rehome_offsets(&mut self, map: super::source::SourceOffsetMap) {
        self.content.rehome_offsets(map);
        self.terminator.rehome_offsets(map);
    }

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

/// Copy-small lexer cursor for one normalized line.
///
/// `byte_cursor` is authoritative in both modes. `scalar_cursor` is the
/// operational scalar position advanced alongside decoding; no character
/// vector or independently seekable scalar index exists. The reduction head
/// is a coordinate into the line-owned spelling arena, not an owner. Copying
/// this value therefore never clones source bytes or a spelling buffer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceLexCursor {
    pub(crate) byte_cursor: u64,
    pub(crate) scalar_cursor: u64,
    pub(crate) reduced_head: u32,
    pub(crate) lexer_state: super::tokenizer::LexerState,
    pub(crate) endline_delivered: bool,
}

impl SourceLexCursor {
    pub(crate) const EMPTY: Self = Self {
        byte_cursor: 0,
        scalar_cursor: 0,
        reduced_head: 0,
        lexer_state: super::tokenizer::LexerState::NewLine,
        endline_delivered: false,
    };
}

/// Variable owner state for one normalized line.
///
/// Geometry and the spelling arena remain owned once. Speculative token probes
/// copy only [`SourceLexCursor`]; checkpoint history likewise retains cursor
/// values rather than cloning this owner.
#[derive(Debug)]
pub(crate) struct SourceLineState {
    pub(crate) physical: PhysicalLine,
    pub(crate) retained_end: u64,
    pub(crate) endline: Option<CharacterCode>,
    pub(crate) cursor: SourceLexCursor,
    /// TeX82 §355 reductions already applied to the mutable input buffer.
    ///
    /// Source bytes remain immutable for provenance, so §316's pseudoprint
    /// projects these replacements over them to recover TeX's live buffer.
    reduced_spellings: ReducedSourceSpellings,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ReducedSourceSpelling {
    pub(crate) range: SourceRange,
    pub(crate) code: CharacterCode,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ReducedSourceSpellingNode {
    spelling: ReducedSourceSpelling,
    previous: u32,
}

#[derive(Debug, Default)]
struct ReducedSourceSpellings {
    nodes: Vec<ReducedSourceSpellingNode>,
}

impl ReducedSourceSpellings {
    fn commit(&mut self, cursor: &mut SourceLexCursor, spelling: ReducedSourceSpelling) {
        let previous = cursor.reduced_head;
        let previous = previous
            .checked_sub(1)
            .and_then(|index| self.nodes.get(index as usize))
            .filter(|node| node.spelling.range.start() == spelling.range.start())
            .map_or(previous, |node| node.previous);
        let index = u32::try_from(self.nodes.len()).expect("source reduction arena fits u32");
        self.nodes
            .push(ReducedSourceSpellingNode { spelling, previous });
        cursor.reduced_head = index.saturating_add(1);
    }

    fn active_len(&self, head: u32) -> usize {
        let mut len = 0_usize;
        let mut next = head;
        while let Some(index) = next.checked_sub(1) {
            let node = self
                .nodes
                .get(index as usize)
                .expect("live source reduction head is admitted");
            len = len.saturating_add(1);
            next = node.previous;
        }
        len
    }

    fn active(&self, head: u32) -> ActiveReducedSourceSpellings<'_> {
        ActiveReducedSourceSpellings {
            owner: self,
            head,
            emitted: 0,
            len: self.active_len(head),
        }
    }
}

pub(crate) struct ActiveReducedSourceSpellings<'a> {
    owner: &'a ReducedSourceSpellings,
    head: u32,
    emitted: usize,
    len: usize,
}

impl Iterator for ActiveReducedSourceSpellings<'_> {
    type Item = ReducedSourceSpelling;

    fn next(&mut self) -> Option<Self::Item> {
        if self.emitted == self.len {
            return None;
        }
        let steps = self.len.saturating_sub(self.emitted).saturating_sub(1);
        let mut next = self.head;
        for _ in 0..steps {
            let index = next.checked_sub(1)?;
            next = self.owner.nodes.get(index as usize)?.previous;
        }
        let index = next.checked_sub(1)?;
        self.emitted += 1;
        self.owner
            .nodes
            .get(index as usize)
            .map(|node| node.spelling)
    }
}

impl ExactSizeIterator for ActiveReducedSourceSpellings<'_> {
    fn len(&self) -> usize {
        self.len.saturating_sub(self.emitted)
    }
}

impl PartialEq for SourceLineState {
    fn eq(&self, other: &Self) -> bool {
        self.physical == other.physical
            && self.retained_end == other.retained_end
            && self.endline == other.endline
            && self.cursor.byte_cursor == other.cursor.byte_cursor
            && self.cursor.scalar_cursor == other.cursor.scalar_cursor
            && self.cursor.lexer_state == other.cursor.lexer_state
            && self.cursor.endline_delivered == other.cursor.endline_delivered
            && self
                .active_reduced_spellings()
                .eq(other.active_reduced_spellings())
    }
}

impl Eq for SourceLineState {}

impl std::hash::Hash for SourceLineState {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.physical.hash(state);
        self.retained_end.hash(state);
        self.endline.hash(state);
        self.cursor.byte_cursor.hash(state);
        self.cursor.scalar_cursor.hash(state);
        self.cursor.lexer_state.hash(state);
        self.cursor.endline_delivered.hash(state);
        for spelling in self.active_reduced_spellings() {
            spelling.hash(state);
        }
    }
}

impl SourceLineState {
    pub(crate) fn rehome_offsets(&mut self, map: super::source::SourceOffsetMap) {
        self.physical.rehome_offsets(map);
        self.retained_end = map.map(self.retained_end);
        self.cursor.byte_cursor = map.map(self.cursor.byte_cursor);
        for node in &mut self.reduced_spellings.nodes {
            node.spelling.range.rehome_offsets(map);
        }
    }

    pub(crate) fn active_reduced_spellings(&self) -> ActiveReducedSourceSpellings<'_> {
        self.reduced_spellings.active(self.cursor.reduced_head)
    }

    pub(crate) fn commit_reduced_spelling(&mut self, spelling: ReducedSourceSpelling) {
        self.reduced_spellings.commit(&mut self.cursor, spelling);
    }

    #[cfg(test)]
    pub(crate) fn reduced_spelling_storage_len(&self) -> usize {
        self.reduced_spellings.nodes.len()
    }

    pub(crate) fn next_character(
        &mut self,
        mode: CharacterMode,
        bytes: &[u8],
    ) -> Option<SourceCharacter> {
        let mut cursor = self.cursor;
        let character = self.next_character_from(&mut cursor, mode, bytes);
        self.cursor = cursor;
        character
    }

    pub(crate) fn next_character_from(
        &self,
        cursor: &mut SourceLexCursor,
        mode: CharacterMode,
        bytes: &[u8],
    ) -> Option<SourceCharacter> {
        if cursor.byte_cursor < self.retained_end {
            let start = cursor.byte_cursor;
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
            cursor.byte_cursor += width;
            let scalar_offset = cursor.scalar_cursor;
            cursor.scalar_cursor += 1;
            return Some(SourceCharacter {
                code,
                range: SourceRange::new(self.physical.source, start, cursor.byte_cursor),
                scalar_offset,
                synthetic: false,
            });
        }
        if cursor.endline_delivered {
            return None;
        }
        cursor.endline_delivered = true;
        let code = self.endline?;
        let scalar_offset = cursor.scalar_cursor;
        cursor.scalar_cursor += 1;
        Some(SourceCharacter {
            code,
            range: SourceRange::new(self.physical.source, self.retained_end, self.retained_end),
            scalar_offset,
            synthetic: true,
        })
    }
}

impl SourceCursor {
    pub(crate) fn load_next_line(&mut self, endlinechar: i32) -> Option<&mut SourceLineState> {
        if self.line.is_some() {
            return self.line.as_mut();
        }
        // A physical line is always loaded from the file itself; only §363
        // may substitute backing, and only for the line already loaded.
        self.line_backing = None;
        self.line_backing_registered = false;
        self.line_backing_capability = None;
        let len = u64::try_from(self.backing.bytes.len()).expect("registration checked length");
        let acquired = std::mem::take(&mut self.pending_acquired_line);
        if self.next_physical_offset >= len && !acquired {
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
            endline,
            cursor: SourceLexCursor {
                byte_cursor: start,
                scalar_cursor: 0,
                reduced_head: 0,
                lexer_state: super::tokenizer::LexerState::NewLine,
                endline_delivered: false,
            },
            reduced_spellings: ReducedSourceSpellings::default(),
        });
        self.line.as_mut()
    }

    /// Runs TeX82 §363's `firm_up_the_line` over the line just loaded.
    ///
    /// §363 is the whole of `\pausing`: `limit:=last; if pausing>0 then if
    /// interaction>nonstop_mode then begin ... print the buffered line;
    /// first:=limit; prompt_input("=>"); if last>first then move the typed
    /// line down into the buffer and reset limit; end`. Every one of those
    /// decisions belongs to the caller's live engine state, so the tokenizer
    /// only supplies the line as `limit` bounds it and installs whatever
    /// replacement comes back. A replacement that cannot be given an identity
    /// is no replacement: §363's `last>first` test simply fails and the
    /// file's line stands.
    pub(crate) fn firm_up_the_line<Q: super::tokenizer::SourceStepQueries + ?Sized>(
        &mut self,
        endlinechar: i32,
        queries: &mut Q,
        lines: &mut LineBackingRegistry<'_>,
    ) {
        let Some(line) = self.line.as_ref() else {
            return;
        };
        let backing = self.current_backing();
        let start = usize::try_from(line.physical.content.start).expect("offset fits usize");
        let end = usize::try_from(line.retained_end).expect("offset fits usize");
        let Some(text) = backing.bytes.get(start..end) else {
            return;
        };
        // Unicode source backing is validated at registration, while exact-
        // byte input may contain arbitrary TeX bytes. Keep the ordinary valid
        // slice borrowed; `from_utf8_lossy` materializes only the exact-byte
        // case that genuinely needs replacement characters for the terminal
        // display boundary.
        let text = String::from_utf8_lossy(text);
        let Some(replacement) = queries.firm_up_the_line(&text) else {
            return;
        };
        let Some(backing) = lines.register(replacement) else {
            return;
        };
        self.replace_current_line(backing, endlinechar);
    }

    /// Replaces the loaded line's characters, per TeX82 §363.
    ///
    /// `firm_up_the_line` moves a typed line down into `buffer` at `start`
    /// and sets `limit:=start+last-first`; the input level, its line number,
    /// and `first`/`loc` bookkeeping are untouched, and §362 goes on to store
    /// `\endlinechar` at the new `limit`. The replacement is therefore one
    /// complete line with no terminator of its own, tokenized from column
    /// zero in `state=new_line` -- exactly what §363 leaves behind.
    ///
    /// Trailing blanks were already removed by §31's `input_ln`; stripping
    /// them again here is idempotent and keeps `retained_end` the single
    /// definition of `limit` for every line, however it was acquired.
    pub(crate) fn replace_current_line(&mut self, backing: RegisteredSource, endlinechar: i32) {
        let Some(line) = self.line.as_ref() else {
            return;
        };
        let end = u64::try_from(backing.bytes.len()).expect("registration checked length");
        let mut retained_end = end;
        while retained_end > 0 {
            let index = usize::try_from(retained_end - 1).expect("backing offset fits usize");
            if backing.bytes[index] != b' ' {
                break;
            }
            retained_end -= 1;
        }
        let endline = endline_character(backing.mode, endlinechar);
        let physical = PhysicalLine {
            source: backing.id,
            number: line.physical.number,
            content: SourceRange::new(backing.id, 0, end),
            terminator: SourceRange::new(backing.id, end, end),
            terminator_kind: LineTerminator::Missing,
        };
        self.line = Some(SourceLineState {
            physical,
            retained_end,
            endline,
            cursor: SourceLexCursor {
                byte_cursor: 0,
                scalar_cursor: 0,
                reduced_head: 0,
                lexer_state: super::tokenizer::LexerState::NewLine,
                endline_delivered: false,
            },
            reduced_spellings: ReducedSourceSpellings::default(),
        });
        self.line_backing = Some(backing);
        self.line_backing_registered = false;
        self.line_backing_capability = None;
    }

    pub(crate) fn finish_line(&mut self) {
        self.line = None;
        self.line_backing = None;
        self.line_backing_registered = false;
        self.line_backing_capability = None;
    }

    /// Installs e-TeX §53a's context-only sentinel record at pseudo EOF.
    ///
    /// `pseudo_input` has advanced `line` past the generated text when
    /// §24.362 inserts `\everyeof`, but it does not tokenize another
    /// `\endlinechar`. The empty record is therefore visible to §313 without
    /// delivering a `\par` command.
    pub(crate) fn install_scantokens_eof_context_line(&mut self) {
        let end = u64::try_from(self.backing.bytes.len()).expect("registration checked length");
        let physical = PhysicalLine {
            source: self.backing.id,
            number: self.next_line_number,
            content: SourceRange::new(self.backing.id, end, end),
            terminator: SourceRange::new(self.backing.id, end, end),
            terminator_kind: LineTerminator::Missing,
        };
        self.next_line_number += 1;
        self.line = Some(SourceLineState {
            physical,
            retained_end: end,
            endline: None,
            cursor: SourceLexCursor {
                byte_cursor: end,
                scalar_cursor: 0,
                reduced_head: 0,
                lexer_state: super::tokenizer::LexerState::NewLine,
                endline_delivered: true,
            },
            reduced_spellings: ReducedSourceSpellings::default(),
        });
        self.line_backing = None;
        self.line_backing_registered = false;
        self.line_backing_capability = None;
        self.end_after_line = true;
    }
}

/// Start offset of the character that ends at `end`, or `end` itself when the
/// normalized line is empty.
pub(crate) fn final_character_start_for_tokenizer(
    bytes: &[u8],
    mode: CharacterMode,
    line_start: u64,
    end: u64,
) -> u64 {
    if end <= line_start {
        return end;
    }
    let mut start = end - 1;
    if mode == CharacterMode::UnicodeExtended {
        while start > line_start
            && usize::try_from(start)
                .ok()
                .and_then(|index| bytes.get(index))
                .is_some_and(|byte| byte & 0b1100_0000 == 0b1000_0000)
        {
            start -= 1;
        }
    }
    start
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
