//! Snapshot-ready input stack summary shared by the lexer and `Universe`.

use crate::ids::TokenListId;
use crate::token::Token;

/// Maximum number of macro arguments TeX permits in one macro body.
pub const MACRO_ARGUMENT_SLOTS: usize = 9;

/// Frozen macro arguments carried by a macro-body replay frame.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct MacroArguments {
    slots: [Option<TokenListId>; MACRO_ARGUMENT_SLOTS],
}

impl MacroArguments {
    /// Creates an empty argument-slot frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [None; MACRO_ARGUMENT_SLOTS],
        }
    }

    /// Records one frozen argument token list in a one-based TeX slot.
    pub fn set(&mut self, slot: u8, token_list: TokenListId) {
        let index = argument_index(slot);
        self.slots[index] = Some(token_list);
    }

    /// Reads the frozen argument token list for a one-based TeX slot.
    #[must_use]
    pub fn get(self, slot: u8) -> Option<TokenListId> {
        let index = argument_index(slot);
        self.slots[index]
    }

    /// Returns whether no argument slots are populated.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.slots.iter().all(Option::is_none)
    }
}

fn argument_index(slot: u8) -> usize {
    assert!(
        (1..=MACRO_ARGUMENT_SLOTS as u8).contains(&slot),
        "macro argument slot must be in 1..=9"
    );
    usize::from(slot - 1)
}

/// The semantic lexer state from TeX's `state` variable.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LexerState {
    /// State N: at the beginning of a line.
    #[default]
    NewLine,
    /// State M: in the middle of a line.
    MidLine,
    /// State S: skipping blanks after a space/control word.
    SkippingBlanks,
}

/// Stable identifier for a source frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceId(u32);

impl SourceId {
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Why a frozen token list is being replayed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TokenListReplayKind {
    MacroBody,
    MacroArgument,
    NoExpand,
    EveryPar,
    EveryCr,
    Mark,
    OutputRoutine,
    Inserted,
}

/// The family of TeX conditional represented by an open condition frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConditionKind {
    /// A regular two-limb `\if...` conditional.
    If,
    /// An `\ifcase` conditional whose active limb is selected by `\or` count.
    IfCase,
}

/// The conditional limb currently being evaluated or skipped.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConditionLimb {
    If,
    Or,
    Else,
}

/// Snapshot-summary state for one open conditional.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConditionFrameSummary {
    kind: ConditionKind,
    limb: ConditionLimb,
    current_limb_taken: bool,
    any_limb_taken: bool,
    ifcase_or_count: u32,
    skip_nesting: u32,
}

impl ConditionFrameSummary {
    /// Creates a regular `\if...` frame.
    #[must_use]
    pub const fn new_if(current_limb_taken: bool) -> Self {
        Self {
            kind: ConditionKind::If,
            limb: ConditionLimb::If,
            current_limb_taken,
            any_limb_taken: current_limb_taken,
            ifcase_or_count: 0,
            skip_nesting: 0,
        }
    }

    /// Creates an `\ifcase` frame at its initial limb.
    #[must_use]
    pub const fn new_ifcase(current_limb_taken: bool) -> Self {
        Self {
            kind: ConditionKind::IfCase,
            limb: ConditionLimb::If,
            current_limb_taken,
            any_limb_taken: current_limb_taken,
            ifcase_or_count: 0,
            skip_nesting: 0,
        }
    }

    #[must_use]
    pub const fn kind(self) -> ConditionKind {
        self.kind
    }

    #[must_use]
    pub const fn limb(self) -> ConditionLimb {
        self.limb
    }

    #[must_use]
    pub const fn current_limb_taken(self) -> bool {
        self.current_limb_taken
    }

    #[must_use]
    pub const fn any_limb_taken(self) -> bool {
        self.any_limb_taken
    }

    #[must_use]
    pub const fn ifcase_or_count(self) -> u32 {
        self.ifcase_or_count
    }

    #[must_use]
    pub const fn skip_nesting(self) -> u32 {
        self.skip_nesting
    }

    /// Moves the frame to an `\or` limb and records how many `\or` tokens
    /// have been crossed in the current `\ifcase`.
    #[must_use]
    pub const fn with_or_limb(mut self, ifcase_or_count: u32, current_limb_taken: bool) -> Self {
        self.limb = ConditionLimb::Or;
        self.ifcase_or_count = ifcase_or_count;
        self.current_limb_taken = current_limb_taken;
        self.any_limb_taken = self.any_limb_taken || current_limb_taken;
        self
    }

    /// Moves the frame to its `\else` limb.
    #[must_use]
    pub const fn with_else_limb(mut self, current_limb_taken: bool) -> Self {
        self.limb = ConditionLimb::Else;
        self.current_limb_taken = current_limb_taken;
        self.any_limb_taken = self.any_limb_taken || current_limb_taken;
        self
    }

    /// Records nested conditional depth observed while scanning/skipping.
    #[must_use]
    pub const fn with_skip_nesting(mut self, skip_nesting: u32) -> Self {
        self.skip_nesting = skip_nesting;
        self
    }
}

/// Snapshot summary for the input stack.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct InputSummary {
    frames: Vec<InputFrameSummary>,
    last_source_frame: Option<SourceFrameSummary>,
}

impl InputSummary {
    #[must_use]
    pub fn new(
        frames: Vec<InputFrameSummary>,
        last_source_frame: Option<SourceFrameSummary>,
    ) -> Self {
        Self {
            frames,
            last_source_frame,
        }
    }

    #[must_use]
    pub fn frames(&self) -> &[InputFrameSummary] {
        &self.frames
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// The most recently popped source frame, retained so a snapshot taken
    /// after source exhaustion can still report the final source coordinates.
    #[must_use]
    pub fn last_source_frame(&self) -> Option<&SourceFrameSummary> {
        self.last_source_frame.as_ref()
    }
}

/// Snapshot summary for one input frame.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum InputFrameSummary {
    Source {
        source_id: SourceId,
        source: SourceFrameSummary,
    },
    TokenList {
        token_list: TokenListId,
        replay_kind: TokenListReplayKind,
        index: usize,
        macro_arguments: MacroArguments,
    },
    Condition(ConditionFrameSummary),
}

/// Snapshot summary for one source frame.
///
/// `source_id` belongs to the surrounding `InputFrameSummary`; the durable
/// reopen key is intentionally not stored here because `World` input records
/// own file/content identity and reopen sources by content hash before this
/// summary's offsets and normalized-line state are applied.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceFrameSummary {
    buffer_offset: usize,
    next_source_offset: usize,
    line_number: usize,
    column: usize,
    lexer_state: LexerState,
    normalized_line: String,
    line_char_offset: usize,
    line_byte_offset: usize,
    pending: Vec<Token>,
    end_after_current_line: bool,
}

impl SourceFrameSummary {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        buffer_offset: usize,
        next_source_offset: usize,
        line_number: usize,
        column: usize,
        lexer_state: LexerState,
        normalized_line: String,
        line_char_offset: usize,
        pending: Vec<Token>,
        end_after_current_line: bool,
    ) -> Self {
        let line: Vec<_> = normalized_line.chars().collect();
        Self {
            buffer_offset,
            next_source_offset,
            line_number,
            column,
            lexer_state,
            normalized_line,
            line_char_offset,
            line_byte_offset: byte_offset_for_char_offset(&line, line_char_offset),
            pending,
            end_after_current_line,
        }
    }

    #[must_use]
    pub fn buffer_offset(&self) -> usize {
        self.buffer_offset
    }

    #[must_use]
    pub fn next_source_offset(&self) -> usize {
        self.next_source_offset
    }

    #[must_use]
    pub fn line_number(&self) -> usize {
        self.line_number
    }

    #[must_use]
    pub fn column(&self) -> usize {
        self.column
    }

    #[must_use]
    pub fn lexer_state(&self) -> LexerState {
        self.lexer_state
    }

    #[must_use]
    pub fn normalized_line(&self) -> &str {
        &self.normalized_line
    }

    #[must_use]
    pub fn line_char_offset(&self) -> usize {
        self.line_char_offset
    }

    #[must_use]
    pub fn line_byte_offset(&self) -> usize {
        self.line_byte_offset
    }

    #[must_use]
    pub fn pending(&self) -> &[Token] {
        &self.pending
    }

    #[must_use]
    pub fn end_after_current_line(&self) -> bool {
        self.end_after_current_line
    }

    /// Returns whether this frame summary contains all lexer-owned state
    /// needed after a source has been reopened by the snapshot owner.
    #[must_use]
    pub fn is_resume_complete(&self) -> bool {
        self.line_char_offset <= self.normalized_line.chars().count()
            && self.line_byte_offset <= self.normalized_line.len()
            && self.normalized_line.is_char_boundary(self.line_byte_offset)
    }
}

fn byte_offset_for_char_offset(line: &[char], char_offset: usize) -> usize {
    line.iter().take(char_offset).map(|ch| ch.len_utf8()).sum()
}
