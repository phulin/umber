//! Canonical token-at-a-time source tokenization.
//!
//! The shared scalar state machine follows TeX.web section 24 (`get_next` and
//! `Scan a control sequence`). Exact-byte profiles retain canonical TeX
//! superscript behavior. `UnicodeExtended` is a separately identified Umber
//! contract over validated Unicode scalars, never a pdfTeX compatibility mode.

use std::ops::Deref;
use std::sync::Arc;

use tex_state::token::{Catcode, TokenWord};

use crate::profile::{CharacterCode, CharacterMode};

use super::lines::{
    SourceCharacter, SourceLocation, SourceProvenance, SourceRange, SourceScalarRange,
};
use super::source::{SourceCursor, SourceRegistration};

/// Character capacity held inside an owned control-sequence spelling.
///
/// The repository's 9,770 fixture control-word occurrences measure p95=15,
/// p99=20, and max=31 characters; the registered primitive vocabulary has a
/// maximum of 17. Twenty-four therefore keeps more than 99% of measured names
/// inline without making pathological-name correctness a fixed-size limit.
pub const CONTROL_SEQUENCE_NAME_INLINE_CAPACITY: usize = 24;

/// An owned semantic control-sequence name with a measured inline fast path.
///
/// Source tokens cross the tokenizer/consumer boundary, so their spellings
/// cannot borrow the live line buffer. Names through
/// [`CONTROL_SEQUENCE_NAME_INLINE_CAPACITY`] occupy the token itself; longer
/// names spill once into an unbounded `Vec`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ControlSequenceName(ControlSequenceNameStorage);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ControlSequenceNameStorage {
    Inline {
        len: u8,
        codes: [CharacterCode; CONTROL_SEQUENCE_NAME_INLINE_CAPACITY],
    },
    Spill(Vec<CharacterCode>),
}

impl ControlSequenceName {
    /// Constructs an empty, allocation-free name.
    #[must_use]
    pub const fn new() -> Self {
        Self(ControlSequenceNameStorage::Inline {
            len: 0,
            codes: [CharacterCode::from_byte(0); CONTROL_SEQUENCE_NAME_INLINE_CAPACITY],
        })
    }

    fn push(&mut self, code: CharacterCode) {
        match &mut self.0 {
            ControlSequenceNameStorage::Inline { len, codes }
                if usize::from(*len) < CONTROL_SEQUENCE_NAME_INLINE_CAPACITY =>
            {
                codes[usize::from(*len)] = code;
                *len += 1;
            }
            ControlSequenceNameStorage::Inline { len, codes } => {
                let mut spill = Vec::with_capacity(CONTROL_SEQUENCE_NAME_INLINE_CAPACITY * 2);
                spill.extend_from_slice(&codes[..usize::from(*len)]);
                spill.push(code);
                self.0 = ControlSequenceNameStorage::Spill(spill);
            }
            ControlSequenceNameStorage::Spill(codes) => codes.push(code),
        }
    }

    /// Calls `consume` with the scalar text used for lookup and interning.
    ///
    /// Inline names encode into a fixed stack buffer. Only names that already
    /// took the pathological spill path allocate a temporary `String` here.
    pub(crate) fn with_text<R>(&self, consume: impl FnOnce(&str) -> R) -> R {
        match &self.0 {
            ControlSequenceNameStorage::Inline { len, codes } => {
                let mut bytes = [0_u8; CONTROL_SEQUENCE_NAME_INLINE_CAPACITY * 4];
                let mut byte_len = 0;
                for code in &codes[..usize::from(*len)] {
                    let mut encoded = [0_u8; 4];
                    let text = crate::profile::token_character(*code).encode_utf8(&mut encoded);
                    bytes[byte_len..byte_len + text.len()].copy_from_slice(text.as_bytes());
                    byte_len += text.len();
                }
                consume(std::str::from_utf8(&bytes[..byte_len]).expect("encoded scalar text"))
            }
            ControlSequenceNameStorage::Spill(codes) => {
                let text: String = codes
                    .iter()
                    .copied()
                    .map(crate::profile::token_character)
                    .collect();
                consume(&text)
            }
        }
    }

    #[cfg(test)]
    fn is_spilled(&self) -> bool {
        matches!(self.0, ControlSequenceNameStorage::Spill(_))
    }
}

impl Default for ControlSequenceName {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for ControlSequenceName {
    type Target = [CharacterCode];

    fn deref(&self) -> &Self::Target {
        match &self.0 {
            ControlSequenceNameStorage::Inline { len, codes } => &codes[..usize::from(*len)],
            ControlSequenceNameStorage::Spill(codes) => codes,
        }
    }
}

impl FromIterator<CharacterCode> for ControlSequenceName {
    fn from_iter<T: IntoIterator<Item = CharacterCode>>(iter: T) -> Self {
        let mut name = Self::new();
        for code in iter {
            name.push(code);
        }
        name
    }
}

/// TeX's source-line lexical state (`mid_line`, `skip_blanks`, `new_line`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LexerState {
    /// Ordinary material has been seen on the current physical line.
    MidLine,
    /// A control word or emitted space suppresses following spacer tokens.
    SkipBlanks,
    /// No material has been emitted from the current physical line.
    #[default]
    NewLine,
}

/// The namespace and construction rule for a source control sequence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceControlSequenceKind {
    /// An escape followed by one or more current-catcode letters.
    Word,
    /// An escape followed by one nonletter character.
    Symbol,
    /// An active character, whose namespace is distinct from escaped names.
    Active,
    /// TeX's frozen blank-line `\par` spelling in the active character domain.
    Paragraph,
    /// The null control sequence produced by an escape at normalized line end.
    Null,
}

/// One source token spelling before control-sequence interning.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SourceToken {
    /// A character token with the catcode observed at delivery.
    Character {
        code: CharacterCode,
        catcode: Catcode,
        range: SourceRange,
        scalar_range: SourceScalarRange,
    },
    /// A control sequence whose name remains a semantic character sequence.
    ControlSequence {
        name: ControlSequenceName,
        kind: SourceControlSequenceKind,
        range: SourceRange,
        scalar_range: SourceScalarRange,
        location: SourceLocation,
    },
}

impl SourceToken {
    /// Exact half-open physical spelling range.
    #[must_use]
    pub fn range(&self) -> SourceRange {
        match self {
            Self::Character { range, .. } | Self::ControlSequence { range, .. } => *range,
        }
    }

    /// Exact half-open decoded-scalar spelling range within the physical line.
    #[must_use]
    pub fn scalar_range(&self) -> SourceScalarRange {
        match self {
            Self::Character { scalar_range, .. } | Self::ControlSequence { scalar_range, .. } => {
                *scalar_range
            }
        }
    }

    /// Raw span and canonical TeX82 location for this decoded spelling.
    #[must_use]
    pub fn provenance(&self) -> SourceProvenance {
        match self {
            Self::Character { range, .. } => SourceProvenance::from_range(*range),
            Self::ControlSequence {
                range, location, ..
            } => SourceProvenance::from_range_and_location(*range, *location),
        }
    }
}

/// One call-local tokenizer spelling before the owning or compact projection.
///
/// Ordinary, untransformed control words borrow their semantic text directly
/// from the current contiguous source backing. The public tokenizer projection
/// converts that text to its owned [`SourceToken`] representation, while the
/// command-delivery projection consumes the borrow immediately for lookup or
/// interning. A spelling whose semantic characters differ from its raw bytes
/// uses the existing owned representation instead.
enum ScannedSourceToken<'line> {
    Owned(SourceToken),
    BorrowedControlWord {
        name: &'line str,
        range: SourceRange,
        scalar_range: SourceScalarRange,
        location: SourceLocation,
    },
}

impl ScannedSourceToken<'_> {
    fn into_owned(self, mode: CharacterMode) -> SourceToken {
        match self {
            Self::Owned(token) => token,
            Self::BorrowedControlWord {
                name,
                range,
                scalar_range,
                location,
            } => {
                let name = match mode {
                    CharacterMode::EightBitExact => name
                        .bytes()
                        .map(CharacterCode::from_byte)
                        .collect::<ControlSequenceName>(),
                    CharacterMode::UnicodeExtended => name
                        .chars()
                        .map(CharacterCode::from)
                        .collect::<ControlSequenceName>(),
                };
                SourceToken::ControlSequence {
                    name,
                    kind: SourceControlSequenceKind::Word,
                    range,
                    scalar_range,
                    location,
                }
            }
        }
    }

    fn provenance(&self) -> SourceProvenance {
        match self {
            Self::Owned(token) => token.provenance(),
            Self::BorrowedControlWord {
                range, location, ..
            } => SourceProvenance::from_range_and_location(*range, *location),
        }
    }
}

/// One recoverable invalid-character observation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InvalidSourceCharacter {
    code: CharacterCode,
    range: SourceRange,
    scalar_range: SourceScalarRange,
}

impl InvalidSourceCharacter {
    /// Exact semantic character that carried catcode 15.
    #[must_use]
    pub const fn code(self) -> CharacterCode {
        self.code
    }

    /// Complete physical spelling, including superscript notation.
    #[must_use]
    pub const fn range(self) -> SourceRange {
        self.range
    }

    /// Complete decoded-scalar spelling range within the physical line.
    #[must_use]
    pub const fn scalar_range(self) -> SourceScalarRange {
        self.scalar_range
    }
}

/// One externally observable step of source tokenization.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SourceTokenizationStep {
    /// One complete semantic source token.
    Token(SourceToken),
    /// A consumed catcode-15 character requiring canonical recovery.
    InvalidCharacter(InvalidSourceCharacter),
    /// The registered source has no remaining physical input.
    End,
}

/// One production source step after control-sequence creation or lookup.
///
/// The owned source-token API retains names for tokenizer consumers. The
/// command machine instead resolves that transient name at the tokenizer
/// boundary and carries only the packed semantic identity into raw delivery.
pub(crate) enum CompactSourceTokenizationStep {
    Token(CompactSourceToken),
    InvalidCharacter,
    NeedLine,
    End,
}

pub(crate) enum CursorSourceTokenizationStep {
    Token(SourceToken),
    InvalidCharacter(InvalidSourceCharacter),
    NeedLine,
    End,
}

pub(crate) struct CompactSourceToken {
    pub(crate) word: TokenWord,
    pub(crate) provenance: SourceProvenance,
}

enum SourceStep<T> {
    Token(T),
    InvalidCharacter(InvalidSourceCharacter),
    NeedLine,
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SuperscriptPolicy {
    ExactByte,
    UnicodeExtended,
}

/// The live engine reads the source tokenizer makes as it advances.
///
/// Both are TeX82 state the tokenizer neither owns nor caches: §207's
/// category codes, queried once per classified character, and §363's
/// `firm_up_the_line`, queried once per loaded physical line. They travel in
/// one trait because they are one borrow of live state, not because the two
/// reads are otherwise related -- a tokenizer that took them as two closures
/// would need the same state borrowed mutably twice at once.
pub trait SourceStepQueries {
    /// TeX82 §207's `cat_code(c)`.
    fn catcode(&mut self, code: CharacterCode) -> Catcode;

    /// TeX82 §363's `firm_up_the_line`, offered for each loaded line.
    ///
    /// `line` is the normalized line as `limit` bounds it. `Some` is §363's
    /// `last>first` branch, where a line typed at the terminal is moved down
    /// over the file's; `None` covers both `pausing<=0` and a bare carriage
    /// return, which §363 treats identically -- the line stands as it is.
    ///
    /// §363 is reached from §362 (a continuing file's refill) and from §538
    /// (a newly opened file's first line), which are the same two moments the
    /// tokenizer loads a line, so offering it here is what makes `\pausing`
    /// apply to every line rather than to the first one a caller observes.
    fn firm_up_the_line(&mut self, line: &str) -> Option<SourceRegistration> {
        let _ = line;
        None
    }
}

pub(crate) trait CompactSourceStepQueries: SourceStepQueries {
    fn compact_source_token(&mut self, token: &SourceToken) -> TokenWord;

    /// Resolves one untransformed multi-character control word while its
    /// spelling still borrows the current source line.
    fn compact_control_word(&mut self, name: &str) -> TokenWord;
}

/// Category codes alone: TeX82 §363's replacement can never fire.
pub struct CatcodeQueries<F>(pub F);

impl<F: FnMut(CharacterCode) -> Catcode> SourceStepQueries for CatcodeQueries<F> {
    fn catcode(&mut self, code: CharacterCode) -> Catcode {
        (self.0)(code)
    }
}

#[derive(Clone, Copy)]
struct SourceStepControls {
    force_eof: bool,
    mode: CharacterMode,
    superscript: SuperscriptPolicy,
}

impl SourceCursor {
    /// Delivers one exact-byte tokenization step.
    pub(crate) fn next_exact_byte_step(
        &mut self,
        force_eof: bool,
        queries: &mut dyn SourceStepQueries,
    ) -> CursorSourceTokenizationStep {
        match self.next_source_step(
            SourceStepControls {
                force_eof,
                mode: CharacterMode::EightBitExact,
                superscript: SuperscriptPolicy::ExactByte,
            },
            queries,
            &mut |_, token| token.into_owned(CharacterMode::EightBitExact),
        ) {
            SourceStep::Token(token) => CursorSourceTokenizationStep::Token(token),
            SourceStep::InvalidCharacter(invalid) => {
                CursorSourceTokenizationStep::InvalidCharacter(invalid)
            }
            SourceStep::NeedLine => CursorSourceTokenizationStep::NeedLine,
            SourceStep::End => CursorSourceTokenizationStep::End,
        }
    }

    /// Delivers one separately identified Unicode-extension tokenization step.
    pub(crate) fn next_unicode_step(
        &mut self,
        force_eof: bool,
        queries: &mut dyn SourceStepQueries,
    ) -> CursorSourceTokenizationStep {
        match self.next_source_step(
            SourceStepControls {
                force_eof,
                mode: CharacterMode::UnicodeExtended,
                superscript: SuperscriptPolicy::UnicodeExtended,
            },
            queries,
            &mut |_, token| token.into_owned(CharacterMode::UnicodeExtended),
        ) {
            SourceStep::Token(token) => CursorSourceTokenizationStep::Token(token),
            SourceStep::InvalidCharacter(invalid) => {
                CursorSourceTokenizationStep::InvalidCharacter(invalid)
            }
            SourceStep::NeedLine => CursorSourceTokenizationStep::NeedLine,
            SourceStep::End => CursorSourceTokenizationStep::End,
        }
    }

    pub(crate) fn next_compact_exact_byte_step(
        &mut self,
        force_eof: bool,
        queries: &mut dyn CompactSourceStepQueries,
    ) -> CompactSourceTokenizationStep {
        match self.next_source_step(
            SourceStepControls {
                force_eof,
                mode: CharacterMode::EightBitExact,
                superscript: SuperscriptPolicy::ExactByte,
            },
            queries,
            &mut |queries, token| {
                let provenance = token.provenance();
                let word = match token {
                    ScannedSourceToken::Owned(token) => queries.compact_source_token(&token),
                    ScannedSourceToken::BorrowedControlWord { name, .. } => {
                        queries.compact_control_word(name)
                    }
                };
                CompactSourceToken { word, provenance }
            },
        ) {
            SourceStep::Token(token) => CompactSourceTokenizationStep::Token(token),
            SourceStep::InvalidCharacter(_) => CompactSourceTokenizationStep::InvalidCharacter,
            SourceStep::NeedLine => CompactSourceTokenizationStep::NeedLine,
            SourceStep::End => CompactSourceTokenizationStep::End,
        }
    }

    pub(crate) fn next_compact_unicode_step(
        &mut self,
        force_eof: bool,
        queries: &mut dyn CompactSourceStepQueries,
    ) -> CompactSourceTokenizationStep {
        match self.next_source_step(
            SourceStepControls {
                force_eof,
                mode: CharacterMode::UnicodeExtended,
                superscript: SuperscriptPolicy::UnicodeExtended,
            },
            queries,
            &mut |queries, token| {
                let provenance = token.provenance();
                let word = match token {
                    ScannedSourceToken::Owned(token) => queries.compact_source_token(&token),
                    ScannedSourceToken::BorrowedControlWord { name, .. } => {
                        queries.compact_control_word(name)
                    }
                };
                CompactSourceToken { word, provenance }
            },
        ) {
            SourceStep::Token(token) => CompactSourceTokenizationStep::Token(token),
            SourceStep::InvalidCharacter(_) => CompactSourceTokenizationStep::InvalidCharacter,
            SourceStep::NeedLine => CompactSourceTokenizationStep::NeedLine,
            SourceStep::End => CompactSourceTokenizationStep::End,
        }
    }

    fn next_source_step<T, Q: SourceStepQueries + ?Sized>(
        &mut self,
        controls: SourceStepControls,
        queries: &mut Q,
        emit: &mut impl for<'line> FnMut(&mut Q, ScannedSourceToken<'line>) -> T,
    ) -> SourceStep<T> {
        let SourceStepControls {
            force_eof,
            mode,
            superscript,
        } = controls;
        debug_assert_eq!(self.backing.mode, mode);

        loop {
            if self.line.is_none() {
                return SourceStep::NeedLine;
            }
            // The replacement §363 installs has backing of its own, so the
            // current line's bytes must be taken after it has run, not once
            // for the whole level.
            let bytes = Arc::clone(&self.current_backing().bytes);
            let mut catcode = |code: CharacterCode| queries.catcode(code);

            let Some(character) =
                self.next_reduced_character(&bytes, mode, superscript, false, &mut catcode)
            else {
                if force_eof || (self.end_after_line && !self.pending_acquired_line) {
                    // TeX82 §362 reaches `end_file_reading` with the final
                    // line's `buffer`, `loc`, and `limit` still installed. In
                    // e-TeX, §24.362 may first put `\everyeof` above this
                    // still-live source, so error context from that token list
                    // must still be able to pseudoprint the exhausted line.
                    return SourceStep::End;
                }
                self.finish_line();
                return SourceStep::NeedLine;
            };
            let scalar_range = self.spelling_scalar_range(character);
            let observed = catcode(character.code());

            // A reduced active character is itself a control-sequence name.
            // Once delivered, TeX82 §355's buffer spelling is the reduced
            // character that §316 pseudoprints, just as for an escaped
            // single-character control sequence. Keep invalid-character
            // recovery on the unreduced physical spelling: §346 reports it
            // immediately, before a control sequence has been formed.
            if observed == Catcode::Active
                && character
                    .range()
                    .end()
                    .saturating_sub(character.range().start())
                    > 1
                && let Some(line) = self.line.as_mut()
            {
                line.reduced_spellings
                    .retain(|spelling| spelling.range.start() != character.range().start());
                line.reduced_spellings
                    .push(super::lines::ReducedSourceSpelling {
                        range: character.range(),
                        code: character.code(),
                    });
            }

            match observed {
                Catcode::Ignored => continue,
                Catcode::Invalid => {
                    return SourceStep::InvalidCharacter(InvalidSourceCharacter {
                        code: character.code(),
                        range: character.range(),
                        scalar_range,
                    });
                }
                Catcode::Comment => {
                    self.skip_rest_of_line();
                    continue;
                }
                Catcode::Escape => {
                    let token = self.scan_control_sequence(
                        character,
                        &bytes,
                        mode,
                        superscript,
                        &mut catcode,
                    );
                    return SourceStep::Token(emit(queries, token));
                }
                Catcode::Active => {
                    self.lexer_state = LexerState::MidLine;
                    let token = SourceToken::ControlSequence {
                        name: std::iter::once(character.code()).collect(),
                        kind: SourceControlSequenceKind::Active,
                        range: character.range(),
                        scalar_range,
                        location: character.range().terminal_location(),
                    };
                    return SourceStep::Token(emit(queries, ScannedSourceToken::Owned(token)));
                }
                Catcode::Space => match self.lexer_state {
                    LexerState::MidLine => {
                        self.lexer_state = LexerState::SkipBlanks;
                        let token = SourceToken::Character {
                            code: semantic_ascii(mode, b' '),
                            catcode: Catcode::Space,
                            range: character.range(),
                            scalar_range,
                        };
                        return SourceStep::Token(emit(queries, ScannedSourceToken::Owned(token)));
                    }
                    LexerState::SkipBlanks | LexerState::NewLine => continue,
                },
                Catcode::EndLine => {
                    // The synthetic character appended while §362 normalizes
                    // a physical line drives §348/§350/§351's line-ending
                    // state machine. A source character that merely has
                    // category 5 is instead a raw `car_ret` command: §1126
                    // must be allowed to route it through `align_error`.
                    // Keeping those cases separate prevents a current
                    // catcode assignment from turning an ordinary buffered
                    // character into a physical line boundary.
                    if !character.is_synthetic() {
                        self.lexer_state = LexerState::MidLine;
                        let token = SourceToken::Character {
                            code: character.code(),
                            catcode: Catcode::EndLine,
                            range: character.range(),
                            scalar_range,
                        };
                        return SourceStep::Token(emit(queries, ScannedSourceToken::Owned(token)));
                    }
                    let range = self.line_end_anchor();
                    let state = self.lexer_state;
                    self.skip_rest_of_line();
                    self.lexer_state = LexerState::NewLine;
                    match state {
                        LexerState::MidLine => {
                            let token = SourceToken::Character {
                                code: semantic_ascii(mode, b' '),
                                catcode: Catcode::Space,
                                range,
                                scalar_range,
                            };
                            return SourceStep::Token(emit(
                                queries,
                                ScannedSourceToken::Owned(token),
                            ));
                        }
                        LexerState::SkipBlanks => continue,
                        LexerState::NewLine => {
                            let token = SourceToken::ControlSequence {
                                name: b"par"
                                    .iter()
                                    .copied()
                                    .map(|byte| semantic_ascii(mode, byte))
                                    .collect(),
                                kind: SourceControlSequenceKind::Paragraph,
                                range,
                                scalar_range,
                                location: range.terminal_location(),
                            };
                            return SourceStep::Token(emit(
                                queries,
                                ScannedSourceToken::Owned(token),
                            ));
                        }
                    }
                }
                _ => {
                    self.lexer_state = LexerState::MidLine;
                    let token = SourceToken::Character {
                        code: character.code(),
                        catcode: observed,
                        range: character.range(),
                        scalar_range,
                    };
                    return SourceStep::Token(emit(queries, ScannedSourceToken::Owned(token)));
                }
            }
        }
    }

    fn scan_control_sequence<'line>(
        &mut self,
        escape: SourceCharacter,
        bytes: &'line [u8],
        mode: CharacterMode,
        superscript: SuperscriptPolicy,
        catcode: &mut impl FnMut(CharacterCode) -> Catcode,
    ) -> ScannedSourceToken<'line> {
        let Some(first) = self.next_reduced_character(bytes, mode, superscript, true, catcode)
        else {
            return ScannedSourceToken::Owned(SourceToken::ControlSequence {
                name: ControlSequenceName::new(),
                kind: SourceControlSequenceKind::Null,
                range: escape.range(),
                scalar_range: self.spelling_scalar_range(escape),
                location: escape.range().terminal_location(),
            });
        };
        let first_catcode = catcode(first.code());
        self.lexer_state = if matches!(first_catcode, Catcode::Letter | Catcode::Space) {
            LexerState::SkipBlanks
        } else {
            LexerState::MidLine
        };

        let name_start = first.range().start();
        let mut owned_name = if character_matches_raw_text(first, bytes, mode) {
            None
        } else {
            Some(std::iter::once(first.code()).collect::<ControlSequenceName>())
        };
        let mut end = first.range().end();
        let mut location = first.range().terminal_location();
        let mut name_len = 1_usize;
        let kind = if first_catcode == Catcode::Letter {
            loop {
                let saved = self.line.clone().expect("control sequence has a line");
                let Some(next) =
                    self.next_reduced_character(bytes, mode, superscript, true, catcode)
                else {
                    break;
                };
                if catcode(next.code()) != Catcode::Letter {
                    self.line = Some(saved);
                    break;
                }
                if let Some(name) = &mut owned_name {
                    name.push(next.code());
                } else if !character_matches_raw_text(next, bytes, mode) {
                    let prefix_end = usize::try_from(next.range().start())
                        .expect("source offset fits backing index");
                    let prefix_start =
                        usize::try_from(name_start).expect("source offset fits backing index");
                    let mut name =
                        control_sequence_name_from_raw(&bytes[prefix_start..prefix_end], mode);
                    name.push(next.code());
                    owned_name = Some(name);
                }
                name_len += 1;
                end = next.range().end();
                location = next.range().terminal_location();
            }
            SourceControlSequenceKind::Word
        } else {
            SourceControlSequenceKind::Symbol
        };

        let range = SourceRange::new(escape.range().source(), escape.range().start(), end);
        let scalar_range = SourceScalarRange::new(
            escape.scalar_offset(),
            self.line
                .as_ref()
                .map_or(first.scalar_offset() + 1, |line| line.scalar_cursor),
        );
        if kind == SourceControlSequenceKind::Word
            && let Some(name) = owned_name
        {
            return ScannedSourceToken::Owned(SourceToken::ControlSequence {
                name,
                kind,
                range,
                scalar_range,
                location,
            });
        }
        if kind == SourceControlSequenceKind::Word && name_len > 1 {
            let start = usize::try_from(name_start).expect("source offset fits backing index");
            let end = usize::try_from(end).expect("source offset fits backing index");
            let name = std::str::from_utf8(&bytes[start..end])
                .expect("unchanged source control word is UTF-8 text");
            return ScannedSourceToken::BorrowedControlWord {
                name,
                range,
                scalar_range,
                location,
            };
        }
        ScannedSourceToken::Owned(SourceToken::ControlSequence {
            name: owned_name.unwrap_or_else(|| std::iter::once(first.code()).collect()),
            kind,
            range,
            scalar_range,
            location,
        })
    }

    fn next_reduced_character(
        &mut self,
        bytes: &[u8],
        mode: CharacterMode,
        superscript: SuperscriptPolicy,
        persist_reduction: bool,
        catcode: &mut impl FnMut(CharacterCode) -> Catcode,
    ) -> Option<SourceCharacter> {
        let line = self.line.as_mut()?;
        let first = line.next_character(mode, bytes)?;
        reduce_superscript_notation(
            line,
            bytes,
            first,
            mode,
            superscript,
            persist_reduction,
            catcode,
        )
    }

    fn spelling_scalar_range(&self, character: SourceCharacter) -> SourceScalarRange {
        SourceScalarRange::new(
            character.scalar_offset(),
            self.line
                .as_ref()
                .map_or(character.scalar_offset() + 1, |line| line.scalar_cursor),
        )
    }

    /// tex.web's `loc:=limit+1`, shared by every case that abandons the rest
    /// of a line: comments (§345's `Cases where character is ignored` sibling
    /// in §344) and all three `car_ret` cases (§348, §350, §351).
    ///
    /// The line stays loaded and merely becomes exhausted, because tex.web
    /// moves to the next line only on the following `get_next`, at §343's
    /// `loc>limit` branch. That branch is where §362 observes `force_eof` and
    /// retires the file, so finishing the line eagerly here would skip
    /// `\endinput`.
    fn skip_rest_of_line(&mut self) {
        if let Some(line) = &mut self.line {
            line.byte_cursor = line.retained_end;
            line.endline_delivered = true;
        }
    }
}

fn character_matches_raw_text(
    character: SourceCharacter,
    bytes: &[u8],
    mode: CharacterMode,
) -> bool {
    if character.is_synthetic() {
        return false;
    }
    let start =
        usize::try_from(character.range().start()).expect("source offset fits backing index");
    let end = usize::try_from(character.range().end()).expect("source offset fits backing index");
    let raw = &bytes[start..end];
    match mode {
        CharacterMode::EightBitExact => character
            .code()
            .to_byte()
            .is_ok_and(|byte| byte.is_ascii() && raw == [byte]),
        CharacterMode::UnicodeExtended => {
            let mut encoded = [0_u8; 4];
            let text = crate::profile::token_character(character.code()).encode_utf8(&mut encoded);
            raw == text.as_bytes()
        }
    }
}

fn control_sequence_name_from_raw(bytes: &[u8], mode: CharacterMode) -> ControlSequenceName {
    match mode {
        CharacterMode::EightBitExact => bytes
            .iter()
            .copied()
            .map(CharacterCode::from_byte)
            .collect(),
        CharacterMode::UnicodeExtended => std::str::from_utf8(bytes)
            .expect("Unicode backing was validated at registration")
            .chars()
            .map(CharacterCode::from)
            .collect(),
    }
}

fn reduce_superscript_notation(
    line: &mut super::lines::SourceLineState,
    bytes: &[u8],
    first: SourceCharacter,
    mode: CharacterMode,
    policy: SuperscriptPolicy,
    persist_reduction: bool,
    catcode: &mut impl FnMut(CharacterCode) -> Catcode,
) -> Option<SourceCharacter> {
    let start = first.range().start();
    let mut current = first;

    loop {
        if catcode(current.code()) != Catcode::Superscript {
            return Some(SourceCharacter {
                code: current.code(),
                range: SourceRange::new(current.range().source(), start, current.range().end()),
                scalar_offset: first.scalar_offset(),
                synthetic: current.is_synthetic(),
            });
        }
        let mut trial = line.clone();
        let Some(second) = trial.next_character(mode, bytes) else {
            return Some(current);
        };
        if second.code() != current.code() {
            return Some(current);
        }
        let Some(third) = trial.next_character(mode, bytes) else {
            return Some(current);
        };

        let reduced = match policy {
            SuperscriptPolicy::ExactByte => reduce_exact_superscript(trial, bytes, third),
            SuperscriptPolicy::UnicodeExtended => {
                reduce_unicode_superscript(trial, bytes, current.code(), third)
            }
        };
        let Some((code, consumed)) = reduced else {
            return Some(current);
        };
        *line = consumed;
        current = SourceCharacter {
            code,
            range: SourceRange::new(first.range().source(), start, line.byte_cursor),
            scalar_offset: first.scalar_offset(),
            synthetic: false,
        };
        // TeX82 §355 overwrites the first superscript byte with the reduced
        // character and shifts the remaining buffer left. Immutable source
        // storage cannot do that, so retain the equivalent replacement for
        // §316's later pseudoprint of the live buffer. A recursive reduction
        // supersedes the shorter replacement recorded by its first pass.
        if persist_reduction {
            line.reduced_spellings
                .retain(|spelling| spelling.range.start() != start);
            line.reduced_spellings
                .push(super::lines::ReducedSourceSpelling {
                    range: current.range(),
                    code,
                });
        }
        // TeX's `reswitch` behavior applies again if the replacement currently
        // has superscript catcode.
    }
}

fn reduce_exact_superscript(
    trial: super::lines::SourceLineState,
    bytes: &[u8],
    third: SourceCharacter,
) -> Option<(CharacterCode, super::lines::SourceLineState)> {
    let third_byte = third.code().to_byte().ok()?;
    if third_byte >= 128 {
        return None;
    }
    let (result, consumed) = if let Some(high) = lower_hex_value(third_byte) {
        let mut hexadecimal = trial.clone();
        match hexadecimal.next_character(CharacterMode::EightBitExact, bytes) {
            Some(fourth) => match fourth.code().to_byte().ok().and_then(lower_hex_value) {
                Some(low) => (16 * high + low, hexadecimal),
                None => (toggle_ascii(third_byte), trial),
            },
            None => (toggle_ascii(third_byte), trial),
        }
    } else {
        (toggle_ascii(third_byte), trial)
    };
    Some((CharacterCode::from_byte(result), consumed))
}

fn reduce_unicode_superscript(
    trial: super::lines::SourceLineState,
    bytes: &[u8],
    superscript: CharacterCode,
    third: SourceCharacter,
) -> Option<(CharacterCode, super::lines::SourceLineState)> {
    let mut after_fourth = trial.clone();
    if third.code() == superscript
        && after_fourth
            .next_character(CharacterMode::UnicodeExtended, bytes)
            .is_some_and(|fourth| fourth.code() == superscript)
        && let Some((value, consumed)) = take_unicode_hex(after_fourth, bytes, 4)
        && let Ok(code) = CharacterCode::from_unicode_scalar(value)
    {
        return Some((code, consumed));
    }

    if let Some(high) = ascii_hex_value(third.code()) {
        let mut consumed = trial.clone();
        if let Some(fourth) = consumed.next_character(CharacterMode::UnicodeExtended, bytes)
            && let Some(low) = ascii_hex_value(fourth.code())
        {
            return Some((
                CharacterCode::from_unicode_scalar(16 * high + low)
                    .expect("two hexadecimal digits form a scalar"),
                consumed,
            ));
        }
    }

    let scalar = third.code().to_unicode_scalar().ok()?;
    let toggled = if scalar < 64 {
        scalar.checked_add(64)?
    } else {
        scalar.checked_sub(64)?
    };
    CharacterCode::from_unicode_scalar(toggled)
        .ok()
        .map(|code| (code, trial))
}

fn take_unicode_hex(
    mut line: super::lines::SourceLineState,
    bytes: &[u8],
    count: usize,
) -> Option<(u32, super::lines::SourceLineState)> {
    let mut value = 0_u32;
    for _ in 0..count {
        let character = line.next_character(CharacterMode::UnicodeExtended, bytes)?;
        value = value
            .checked_mul(16)?
            .checked_add(ascii_hex_value(character.code())?)?;
    }
    Some((value, line))
}

fn semantic_ascii(mode: CharacterMode, byte: u8) -> CharacterCode {
    match mode {
        CharacterMode::EightBitExact => CharacterCode::from_byte(byte),
        CharacterMode::UnicodeExtended => CharacterCode::from_unicode_scalar(u32::from(byte))
            .expect("ASCII is a valid Unicode scalar"),
    }
}

fn ascii_hex_value(code: CharacterCode) -> Option<u32> {
    let scalar = code.to_unicode_scalar().ok()?;
    match scalar {
        value @ 0x30..=0x39 => Some(value - 0x30),
        value @ 0x41..=0x46 => Some(value - 0x41 + 10),
        value @ 0x61..=0x66 => Some(value - 0x61 + 10),
        _ => None,
    }
}

const fn lower_hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

const fn toggle_ascii(byte: u8) -> u8 {
    if byte < 64 { byte + 64 } else { byte - 64 }
}

#[cfg(test)]
mod tests;
