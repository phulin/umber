//! Snapshot-ready input stack summary shared by the lexer and `Universe`.

use crate::ids::{OriginListId, TokenListId};
use crate::source_map::RegisteredSource;
use crate::token::{Token, TracedTokenWord};
use crate::world::InputRecordId;
use std::hash::{Hash, Hasher};

/// Maximum number of macro arguments TeX permits in one macro body.
pub const MACRO_ARGUMENT_SLOTS: usize = 9;

/// A frozen semantic token list paired with the per-instance origins that
/// should be used when replaying it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TracedTokenList {
    token_list: TokenListId,
    origin_list: OriginListId,
}

impl TracedTokenList {
    /// Creates a token-list replay pair.
    #[must_use]
    pub const fn new(token_list: TokenListId, origin_list: OriginListId) -> Self {
        Self {
            token_list,
            origin_list,
        }
    }

    /// Creates a replay pair with no origin-list home.
    #[must_use]
    pub const fn synthetic(token_list: TokenListId) -> Self {
        Self {
            token_list,
            origin_list: OriginListId::EMPTY,
        }
    }

    #[must_use]
    pub const fn token_list(self) -> TokenListId {
        self.token_list
    }

    #[must_use]
    pub const fn origin_list(self) -> OriginListId {
        self.origin_list
    }
}

/// Frozen macro arguments carried by a macro-body replay frame.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct MacroArguments {
    slots: [Option<TracedTokenList>; MACRO_ARGUMENT_SLOTS],
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
        self.set_traced(slot, TracedTokenList::synthetic(token_list));
    }

    /// Records one traced frozen argument token list in a one-based TeX slot.
    pub fn set_traced(&mut self, slot: u8, token_list: TracedTokenList) {
        let index = argument_index(slot);
        self.slots[index] = Some(token_list);
    }

    /// Reads the frozen argument token list for a one-based TeX slot.
    #[must_use]
    pub fn get(self, slot: u8) -> Option<TokenListId> {
        self.get_traced(slot).map(TracedTokenList::token_list)
    }

    /// Reads the traced frozen argument token list for a one-based TeX slot.
    #[must_use]
    pub fn get_traced(self, slot: u8) -> Option<TracedTokenList> {
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

/// Stable identity for one live conditional frame.
///
/// Expansion keeps this token across recursive operand scans so the result is
/// committed to the same frame that was pushed when the conditional began,
/// even when a nested conditional remains above it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConditionFrameToken(u64);

impl ConditionFrameToken {
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Snapshot-summary state for one open conditional.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConditionFrameSummary {
    context: TracedTokenWord,
    kind: ConditionKind,
    limb: ConditionLimb,
    evaluating: bool,
    current_limb_taken: bool,
    any_limb_taken: bool,
    ifcase_or_count: u32,
    skip_nesting: u32,
}

impl ConditionFrameSummary {
    /// Creates a regular `\if...` frame.
    #[must_use]
    pub const fn new_if(context: TracedTokenWord, current_limb_taken: bool) -> Self {
        Self {
            context,
            kind: ConditionKind::If,
            limb: ConditionLimb::If,
            evaluating: false,
            current_limb_taken,
            any_limb_taken: current_limb_taken,
            ifcase_or_count: 0,
            skip_nesting: 0,
        }
    }

    /// Creates an `\ifcase` frame at its initial limb.
    #[must_use]
    pub const fn new_ifcase(context: TracedTokenWord, current_limb_taken: bool) -> Self {
        Self {
            context,
            kind: ConditionKind::IfCase,
            limb: ConditionLimb::If,
            evaluating: false,
            current_limb_taken,
            any_limb_taken: current_limb_taken,
            ifcase_or_count: 0,
            skip_nesting: 0,
        }
    }

    /// Creates a regular `\if...` frame whose operands are still being
    /// scanned.
    #[must_use]
    pub const fn evaluating_if(context: TracedTokenWord) -> Self {
        Self {
            context,
            kind: ConditionKind::If,
            limb: ConditionLimb::If,
            evaluating: true,
            current_limb_taken: false,
            any_limb_taken: false,
            ifcase_or_count: 0,
            skip_nesting: 0,
        }
    }

    /// Creates an `\ifcase` frame whose selector is still being scanned.
    #[must_use]
    pub const fn evaluating_ifcase(context: TracedTokenWord) -> Self {
        Self {
            context,
            kind: ConditionKind::IfCase,
            limb: ConditionLimb::If,
            evaluating: true,
            current_limb_taken: false,
            any_limb_taken: false,
            ifcase_or_count: 0,
            skip_nesting: 0,
        }
    }

    #[must_use]
    pub const fn kind(self) -> ConditionKind {
        self.kind
    }

    #[must_use]
    pub const fn context(self) -> TracedTokenWord {
        self.context
    }

    #[must_use]
    pub const fn limb(self) -> ConditionLimb {
        self.limb
    }

    #[must_use]
    pub const fn evaluating(self) -> bool {
        self.evaluating
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
        self.evaluating = false;
        self.ifcase_or_count = ifcase_or_count;
        self.current_limb_taken = current_limb_taken;
        self.any_limb_taken = self.any_limb_taken || current_limb_taken;
        self
    }

    /// Moves the frame to its `\else` limb.
    #[must_use]
    pub const fn with_else_limb(mut self, current_limb_taken: bool) -> Self {
        self.limb = ConditionLimb::Else;
        self.evaluating = false;
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
    last_source_id: Option<SourceId>,
    last_source_record: Option<InputRecordId>,
    last_source_frame: Option<SourceFrameSummary>,
    next_source_id: u32,
    unicode_superscript_notation: bool,
}

impl InputSummary {
    #[must_use]
    pub fn new(
        frames: Vec<InputFrameSummary>,
        last_source_id: Option<SourceId>,
        last_source_frame: Option<SourceFrameSummary>,
    ) -> Self {
        Self::new_with_source_records(frames, last_source_id, None, last_source_frame)
    }

    #[must_use]
    pub fn new_with_source_records(
        frames: Vec<InputFrameSummary>,
        last_source_id: Option<SourceId>,
        last_source_record: Option<InputRecordId>,
        last_source_frame: Option<SourceFrameSummary>,
    ) -> Self {
        let next_source_id = frames
            .iter()
            .filter_map(|frame| match frame {
                InputFrameSummary::Source { source_id, .. } => Some(source_id.raw()),
                InputFrameSummary::TokenList { .. } | InputFrameSummary::Condition { .. } => None,
            })
            .chain(last_source_id.map(SourceId::raw))
            .max()
            .map_or(0, |id| {
                id.checked_add(1).expect("source id counter overflowed")
            });
        Self::new_with_resume_state(
            frames,
            last_source_id,
            last_source_record,
            last_source_frame,
            next_source_id,
            true,
        )
    }

    #[must_use]
    pub fn new_with_resume_state(
        frames: Vec<InputFrameSummary>,
        last_source_id: Option<SourceId>,
        last_source_record: Option<InputRecordId>,
        last_source_frame: Option<SourceFrameSummary>,
        next_source_id: u32,
        unicode_superscript_notation: bool,
    ) -> Self {
        Self {
            frames,
            last_source_id,
            last_source_record,
            last_source_frame,
            next_source_id,
            unicode_superscript_notation,
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

    /// The stable id for [`Self::last_source_frame`], when one is retained.
    #[must_use]
    pub const fn last_source_id(&self) -> Option<SourceId> {
        self.last_source_id
    }

    /// The `World` input record for [`Self::last_source_frame`].
    #[must_use]
    pub const fn last_source_record(&self) -> Option<InputRecordId> {
        self.last_source_record
    }

    #[must_use]
    pub const fn next_source_id(&self) -> u32 {
        self.next_source_id
    }

    #[must_use]
    pub const fn unicode_superscript_notation(&self) -> bool {
        self.unicode_superscript_notation
    }
}

/// Snapshot summary for one input frame.
#[derive(Clone, Debug)]
pub enum InputFrameSummary {
    Source {
        source_id: SourceId,
        input_record: Option<InputRecordId>,
        source: SourceFrameSummary,
    },
    TokenList {
        token_list: TokenListId,
        origin_list: OriginListId,
        replay_kind: TokenListReplayKind,
        index: usize,
        macro_arguments: MacroArguments,
        macro_invocation: crate::token::OriginId,
        parent_macro_invocation: crate::token::OriginId,
    },
    Condition {
        token: ConditionFrameToken,
        condition: ConditionFrameSummary,
    },
}

impl PartialEq for InputFrameSummary {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Source {
                    source_id: left_id,
                    input_record: left_record,
                    source: left,
                },
                Self::Source {
                    source_id: right_id,
                    input_record: right_record,
                    source: right,
                },
            ) => left_id == right_id && left_record == right_record && left == right,
            (
                Self::TokenList {
                    token_list: left_token_list,
                    replay_kind: left_replay_kind,
                    index: left_index,
                    macro_arguments: left_arguments,
                    ..
                },
                Self::TokenList {
                    token_list: right_token_list,
                    replay_kind: right_replay_kind,
                    index: right_index,
                    macro_arguments: right_arguments,
                    ..
                },
            ) => {
                left_token_list == right_token_list
                    && left_replay_kind == right_replay_kind
                    && left_index == right_index
                    && macro_arguments_semantic_eq(*left_arguments, *right_arguments)
            }
            (
                Self::Condition {
                    token: _,
                    condition: left,
                },
                Self::Condition {
                    token: _,
                    condition: right,
                },
            ) => left == right,
            _ => false,
        }
    }
}

impl Eq for InputFrameSummary {}

impl Hash for InputFrameSummary {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Source {
                source_id,
                input_record,
                source,
            } => {
                0_u8.hash(state);
                source_id.hash(state);
                input_record.hash(state);
                source.hash(state);
            }
            Self::TokenList {
                token_list,
                replay_kind,
                index,
                macro_arguments,
                ..
            } => {
                1_u8.hash(state);
                token_list.hash(state);
                replay_kind.hash(state);
                index.hash(state);
                hash_macro_arguments_semantic(*macro_arguments, state);
            }
            Self::Condition {
                token: _,
                condition,
            } => {
                2_u8.hash(state);
                condition.hash(state);
            }
        }
    }
}

fn macro_arguments_semantic_eq(left: MacroArguments, right: MacroArguments) -> bool {
    (1..=MACRO_ARGUMENT_SLOTS as u8).all(|slot| left.get(slot) == right.get(slot))
}

fn hash_macro_arguments_semantic<H: Hasher>(arguments: MacroArguments, state: &mut H) {
    for slot in 1..=MACRO_ARGUMENT_SLOTS as u8 {
        arguments.get(slot).hash(state);
    }
}

/// Snapshot summary for one source frame.
///
/// `source_id` and the durable `World` input-record reopen key belong to the
/// surrounding `InputFrameSummary`; this value also retains the opaque source
/// registration capability needed to reject a recycled `SourceId` on resume.
#[derive(Clone, Debug)]
pub struct SourceFrameSummary {
    buffer_offset: usize,
    next_source_offset: usize,
    line_number: usize,
    column: usize,
    lexer_state: LexerState,
    normalized_line: String,
    line_byte_offset: usize,
    physical_content_end: usize,
    terminator_start: usize,
    terminator_end: usize,
    normalized_end_anchor: usize,
    synthetic_endline_start: Option<usize>,
    pending: Vec<TracedTokenWord>,
    end_after_current_line: bool,
    registration: Option<RegisteredSource>,
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
        pending: Vec<TracedTokenWord>,
        end_after_current_line: bool,
    ) -> Self {
        let line_byte_offset = normalized_line
            .char_indices()
            .nth(line_char_offset)
            .map_or(normalized_line.len(), |(offset, _)| offset);
        let normalized_end_anchor = buffer_offset + normalized_line.len();
        Self::new_with_physical_metadata(
            buffer_offset,
            next_source_offset,
            line_number,
            column,
            lexer_state,
            normalized_line,
            line_byte_offset,
            normalized_end_anchor,
            normalized_end_anchor,
            normalized_end_anchor,
            normalized_end_anchor,
            None,
            pending,
            end_after_current_line,
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new_with_physical_metadata(
        buffer_offset: usize,
        next_source_offset: usize,
        line_number: usize,
        column: usize,
        lexer_state: LexerState,
        normalized_line: String,
        line_byte_offset: usize,
        physical_content_end: usize,
        terminator_start: usize,
        terminator_end: usize,
        normalized_end_anchor: usize,
        synthetic_endline_start: Option<usize>,
        pending: Vec<TracedTokenWord>,
        end_after_current_line: bool,
    ) -> Self {
        Self {
            buffer_offset,
            next_source_offset,
            line_number,
            column,
            lexer_state,
            normalized_line,
            line_byte_offset,
            physical_content_end,
            terminator_start,
            terminator_end,
            normalized_end_anchor,
            synthetic_endline_start,
            pending,
            end_after_current_line,
            registration: None,
        }
    }

    /// Attaches the live aggregate source registration used by this frame.
    #[must_use]
    pub const fn with_registration(mut self, registration: Option<RegisteredSource>) -> Self {
        self.registration = registration;
        self
    }

    /// Returns the aggregate source registration retained for resume.
    #[must_use]
    pub const fn registration(&self) -> Option<RegisteredSource> {
        self.registration
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
        self.normalized_line[..self.line_byte_offset]
            .chars()
            .count()
    }

    #[must_use]
    pub fn line_byte_offset(&self) -> usize {
        self.line_byte_offset
    }

    #[must_use]
    pub fn physical_content_end(&self) -> usize {
        self.physical_content_end
    }

    #[must_use]
    pub fn terminator_start(&self) -> usize {
        self.terminator_start
    }

    #[must_use]
    pub fn terminator_end(&self) -> usize {
        self.terminator_end
    }

    #[must_use]
    pub fn normalized_end_anchor(&self) -> usize {
        self.normalized_end_anchor
    }

    #[must_use]
    pub const fn synthetic_endline_start(&self) -> Option<usize> {
        self.synthetic_endline_start
    }

    #[must_use]
    pub fn pending(&self) -> &[TracedTokenWord] {
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
        self.line_byte_offset <= self.normalized_line.len()
            && self.normalized_line.is_char_boundary(self.line_byte_offset)
            && self.buffer_offset <= self.normalized_end_anchor
            && self.normalized_end_anchor <= self.physical_content_end
            && self.physical_content_end <= self.terminator_start
            && self.terminator_start <= self.terminator_end
            && self.terminator_end <= self.next_source_offset
            && self.synthetic_endline_start.is_none_or(|offset| {
                offset <= self.normalized_line.len()
                    && self.normalized_line.is_char_boundary(offset)
            })
    }
}

impl PartialEq for SourceFrameSummary {
    fn eq(&self, other: &Self) -> bool {
        self.buffer_offset == other.buffer_offset
            && self.next_source_offset == other.next_source_offset
            && self.line_number == other.line_number
            && self.column == other.column
            && self.lexer_state == other.lexer_state
            && self.normalized_line == other.normalized_line
            && self.line_byte_offset == other.line_byte_offset
            && self.physical_content_end == other.physical_content_end
            && self.terminator_start == other.terminator_start
            && self.terminator_end == other.terminator_end
            && self.normalized_end_anchor == other.normalized_end_anchor
            && self.synthetic_endline_start == other.synthetic_endline_start
            && self.end_after_current_line == other.end_after_current_line
            && traced_pending_tokens_eq(&self.pending, &other.pending)
    }
}

impl Eq for SourceFrameSummary {}

impl Hash for SourceFrameSummary {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.buffer_offset.hash(state);
        self.next_source_offset.hash(state);
        self.line_number.hash(state);
        self.column.hash(state);
        self.lexer_state.hash(state);
        self.normalized_line.hash(state);
        self.line_byte_offset.hash(state);
        self.physical_content_end.hash(state);
        self.terminator_start.hash(state);
        self.terminator_end.hash(state);
        self.normalized_end_anchor.hash(state);
        self.synthetic_endline_start.hash(state);
        self.pending.len().hash(state);
        for token in &self.pending {
            semantic_token(*token).hash(state);
        }
        self.end_after_current_line.hash(state);
    }
}

fn traced_pending_tokens_eq(left: &[TracedTokenWord], right: &[TracedTokenWord]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| semantic_token(*left) == semantic_token(*right))
}

fn semantic_token(token: TracedTokenWord) -> Token {
    token
        .token()
        .expect("source-frame pending tokens must be valid traced tokens")
}
