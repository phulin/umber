//! Snapshot-ready input stack summary shared by the lexer and `Universe`.

use crate::ids::{OriginListId, TokenListId};
use crate::provenance::OriginListRef;
use crate::source_map::RegisteredSource;
use crate::token::{OriginId, Token, TracedTokenWord};
use crate::token_store::TokenListRef;
use crate::world::InputRecordId;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Maximum number of macro arguments TeX permits in one macro body.
pub const MACRO_ARGUMENT_SLOTS: usize = 9;

/// The two non-depth sentinels assigned by TeX's alignment driver.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlignmentScannerPhase {
    /// Preamble scanning: top-level `&` and `\cr` delimit templates.
    Preamble,
    /// Row lookahead and u-template replay: delimiter interception is disabled.
    BetweenEntries,
}

/// Which immutable replay characters a direct span consumer accepts.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LiteralSpanPolicy {
    /// Characters inert to an expanded replacement scanner.
    ExpandedReplacement,
    /// Ordinary text characters accepted by horizontal main control.
    HorizontalText,
}

impl LiteralSpanPolicy {
    #[must_use]
    pub const fn accepts(self, token: Token) -> bool {
        match (self, token) {
            (
                Self::ExpandedReplacement,
                Token::Char {
                    cat:
                        crate::token::Catcode::BeginGroup
                        | crate::token::Catcode::EndGroup
                        | crate::token::Catcode::Parameter
                        | crate::token::Catcode::Active,
                    ..
                },
            ) => false,
            (Self::ExpandedReplacement, Token::Char { .. }) => true,
            (
                Self::HorizontalText,
                Token::Char {
                    cat:
                        crate::token::Catcode::Letter
                        | crate::token::Catcode::Other
                        | crate::token::Catcode::Space,
                    ..
                },
            ) => true,
            (
                Self::ExpandedReplacement | Self::HorizontalText,
                Token::Cs(_) | Token::Param(_) | Token::Frozen(_),
            )
            | (Self::HorizontalText, Token::Char { .. }) => false,
        }
    }
}

impl AlignmentScannerPhase {
    /// Returns TeX's sentinel `align_state` value for this scanner phase.
    #[must_use]
    pub const fn align_state(self) -> i32 {
        match self {
            Self::Preamble => -1_000_000,
            Self::BetweenEntries => 1_000_000,
        }
    }
}

/// A frozen semantic token list paired with the per-instance origins that
/// should be used when replaying it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TracedTokenList {
    token_list: TokenListRef,
    origin_list: OriginListRef,
}

impl TracedTokenList {
    /// Creates a token-list replay pair.
    #[must_use]
    pub fn new(token_list: TokenListRef, origin_list: OriginListRef) -> Self {
        Self {
            token_list,
            origin_list,
        }
    }

    /// Creates a replay pair with no origin-list home.
    #[must_use]
    pub fn synthetic(token_list: TokenListRef) -> Self {
        Self {
            token_list,
            origin_list: OriginListRef::empty(),
        }
    }

    #[must_use]
    pub fn token_list(&self) -> TokenListId {
        self.token_list.id()
    }

    /// Borrows the copy-only token-list identity.
    #[must_use]
    pub const fn token_ref(&self) -> TokenListRef {
        self.token_list
    }

    #[must_use]
    pub fn origin_list(&self) -> OriginListId {
        self.origin_list.id()
    }

    /// Borrows the strong exact-position owner paired with this token list.
    #[must_use]
    pub fn origin_ref(&self) -> &OriginListRef {
        &self.origin_list
    }
}

/// Compact range into one packed macro-argument buffer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroArgumentRange {
    start: u32,
    len: u32,
}

impl MacroArgumentRange {
    #[must_use]
    pub fn new(start: usize, len: usize) -> Self {
        Self {
            start: u32::try_from(start).expect("macro argument offset exceeds u32"),
            len: u32::try_from(len).expect("macro argument length exceeds u32"),
        }
    }

    #[must_use]
    pub const fn start(self) -> usize {
        self.start as usize
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.len as usize
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// By-value checkpoint form of macro arguments carried by a macro-body frame.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct MacroArguments {
    tokens: Arc<[TracedTokenWord]>,
    slots: [Option<MacroArgumentRange>; MACRO_ARGUMENT_SLOTS],
}

impl MacroArguments {
    /// Creates an empty argument-slot frame.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: [None; MACRO_ARGUMENT_SLOTS],
            tokens: Arc::from([]),
        }
    }

    #[must_use]
    pub fn from_parts(
        tokens: impl Into<Arc<[TracedTokenWord]>>,
        slots: [Option<MacroArgumentRange>; MACRO_ARGUMENT_SLOTS],
    ) -> Self {
        let tokens = tokens.into();
        for range in slots.iter().flatten().copied() {
            assert!(range.start().saturating_add(range.len()) <= tokens.len());
        }
        Self { tokens, slots }
    }

    #[must_use]
    pub fn tokens(&self) -> &Arc<[TracedTokenWord]> {
        &self.tokens
    }

    #[must_use]
    pub fn get(&self, slot: u8) -> Option<&[TracedTokenWord]> {
        let index = argument_index(slot);
        let range = self.slots[index]?;
        Some(&self.tokens[range.start()..range.start() + range.len()])
    }

    #[must_use]
    pub const fn ranges(&self) -> &[Option<MacroArgumentRange>; MACRO_ARGUMENT_SLOTS] {
        &self.slots
    }

    /// Returns whether no argument slots are populated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
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

/// Immutable location of a token delivered from macro-body replay.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MacroReplaySite {
    token_list: TokenListRef,
    token_index: usize,
}

impl MacroReplaySite {
    #[must_use]
    pub fn new(token_list: TokenListRef, token_index: usize) -> Self {
        Self {
            token_list,
            token_index,
        }
    }

    #[must_use]
    pub fn token_list(&self) -> TokenListId {
        self.token_list.id()
    }

    #[must_use]
    pub const fn token_index(&self) -> usize {
        self.token_index
    }
}

/// One traced input delivery with expansion-control and replay metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TracedExpansionToken {
    token: Token,
    origin: OriginId,
    suppress_expansion: bool,
    expand_in_ordinary_context: bool,
    macro_replay_site: Option<MacroReplaySite>,
}

impl TracedExpansionToken {
    #[must_use]
    pub fn new(token: TracedTokenWord, suppress_expansion: bool) -> Self {
        Self::from_parts(token, suppress_expansion, false, None)
    }

    #[must_use]
    pub fn from_parts(
        token: TracedTokenWord,
        suppress_expansion: bool,
        expand_in_ordinary_context: bool,
        macro_replay_site: Option<MacroReplaySite>,
    ) -> Self {
        Self {
            token: token.semantic_token(),
            origin: token.origin(),
            suppress_expansion,
            expand_in_ordinary_context,
            macro_replay_site,
        }
    }

    #[must_use]
    pub fn traced_token(&self) -> TracedTokenWord {
        TracedTokenWord::pack(self.token, self.origin)
    }

    #[must_use]
    pub const fn token(&self) -> Token {
        self.token
    }

    #[must_use]
    pub const fn origin(&self) -> OriginId {
        self.origin
    }

    #[must_use]
    pub const fn suppress_expansion(&self) -> bool {
        self.suppress_expansion
    }

    #[must_use]
    pub const fn expand_in_ordinary_context(&self) -> bool {
        self.expand_in_ordinary_context
    }

    #[must_use]
    pub const fn macro_replay_site(&self) -> Option<&MacroReplaySite> {
        self.macro_replay_site.as_ref()
    }
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

/// Identifies one live token-list replay frame independently of its content.
///
/// The marker is intentionally absent from resumable input summaries: callers
/// use it only to delimit a synchronous replay operation on the current stack.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TokenListReplayMarker {
    sequence: u64,
    frame_index: usize,
}

impl TokenListReplayMarker {
    #[must_use]
    pub const fn new(sequence: u64, frame_index: usize) -> Self {
        Self {
            sequence,
            frame_index,
        }
    }

    #[must_use]
    pub const fn frame_index(self) -> usize {
        self.frame_index
    }
}

/// Why a frozen token list is being replayed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TokenListReplayKind {
    MacroBody,
    MacroArgument,
    NoExpand,
    /// tex.web §307's `backed_up`: tokens `back_input` returned to the input.
    ///
    /// Distinct from [`Self::Inserted`] only in how §314 describes it --
    /// `<to be read again>␣` while unread, `<recently read>␣` once consumed,
    /// against `inserted`'s single `<inserted text>␣`. §327's `ins_error`
    /// exists precisely to retype a backed-up list as `inserted`, so a level
    /// TeX distinguishes in every error report cannot be one kind here.
    BackedUp,
    /// Tokens returned by e-TeX `\unexpanded`; command demand may expand them.
    Unexpanded,
    EveryPar,
    EveryHBox,
    EveryVBox,
    EveryJob,
    EveryCr,
    Mark,
    OutputRoutine,
    Inserted,
    /// `\everyeof` replay whose retirement closes a traced `\scantokens` file.
    ScantokensEveryEof,
    AlignmentUTemplate,
    /// TeX82's v-template frame, retained when exhausted until `do_endv`.
    AlignmentVTemplate,
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
    inverted: bool,
    if_type: u8,
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
            inverted: false,
            if_type: 0,
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
            inverted: false,
            if_type: 0,
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
            inverted: false,
            if_type: 0,
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
            inverted: false,
            if_type: 0,
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

    #[must_use]
    pub const fn inverted(self) -> bool {
        self.inverted
    }

    #[must_use]
    pub const fn with_inverted(mut self, inverted: bool) -> Self {
        self.inverted = inverted;
        self
    }

    #[must_use]
    pub const fn if_type(self) -> u8 {
        self.if_type
    }

    #[must_use]
    pub const fn with_if_type(mut self, if_type: u8) -> Self {
        self.if_type = if_type;
        self
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
#[derive(Clone, Debug, Default)]
pub struct InputSummary {
    semantic_root: InputSemanticRoot,
    last_source_id: Option<SourceId>,
    next_source_id: u32,
}

/// One immutable root for every input field that participates in checkpoint
/// semantics.
///
/// Equality intentionally means allocation identity: this is a cheap cache
/// key, never a hash value. Rebuilt roots are projected canonically and
/// compared by fingerprint at the aggregate `Universe` boundary.
#[derive(Clone, Debug, Default)]
pub(crate) struct InputSemanticRoot(Arc<InputSemanticState>);

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
struct InputSemanticState {
    frames: Vec<InputFrameSummary>,
    last_source_record: Option<InputRecordId>,
    last_source_frame: Option<SourceFrameSummary>,
    unicode_superscript_notation: bool,
    utf8_input_as_bytes: bool,
}

impl PartialEq for InputSemanticRoot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for InputSemanticRoot {}

impl PartialEq for InputSummary {
    fn eq(&self, other: &Self) -> bool {
        self.semantic_root.0 == other.semantic_root.0
    }
}

impl Eq for InputSummary {}

impl Hash for InputSummary {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.semantic_root.0.hash(state);
    }
}

impl InputSummary {
    /// Detached TeX82 §530 source context for a later output-open retry.
    ///
    /// Deferred `\openout` executes after its originating token list has been
    /// consumed. The current physical source frame is therefore captured at
    /// shipout, while it is still canonical input state.
    #[must_use]
    pub fn output_open_context(&self) -> String {
        let Some(source) = self.semantic_root.0.frames.iter().rev().find_map(|frame| {
            let InputFrameSummary::Source { source, .. } = frame else {
                return None;
            };
            Some(source)
        }) else {
            return String::new();
        };
        let line = source.normalized_line().trim_end_matches('\r');
        let split = source.line_byte_offset().min(line.len());
        let split = (0..=split)
            .rev()
            .find(|offset| line.is_char_boundary(*offset))
            .unwrap_or(0);
        let (read, remaining) = line.split_at(split);
        let label = format!("l.{} ", source.line_number());
        format!(
            "\n{label}{read}\n{}{remaining}",
            " ".repeat(label.chars().count())
        )
    }

    /// tex.web §310's `show_context` display for this replay stack.
    ///
    /// The pseudoprint arithmetic itself is
    /// [`crate::print::render_error_context`]; this is only §312--§314's
    /// projection of the gullet's frames onto it. The result is owned text
    /// because §82's report can be deferred past the point where the stack
    /// that produced it has retired.
    #[must_use]
    pub fn show_context(&self, stores: &crate::Universe) -> String {
        use crate::print::{ErrorContextLevel, token_list_replay_label};

        /// §312's `token_type=backed_up` family, the only levels it omits
        /// once they have been read to the end.
        const fn backed_up(kind: TokenListReplayKind) -> bool {
            matches!(
                kind,
                TokenListReplayKind::BackedUp | TokenListReplayKind::NoExpand
            )
        }

        fn shown_tokens(stores: &crate::Universe, tokens: &[Token]) -> String {
            let mut text = String::new();
            for &token in tokens {
                crate::token_show::append_token_show_text(stores, token, &mut text);
            }
            text
        }

        // §312's `base_ptr=input_ptr` test. Condition frames are not input
        // levels in tex.web at all, so they never make the level beneath them
        // stop counting as the current one.
        let mut current = true;
        let mut levels = Vec::new();
        for frame in self.frames().iter().rev() {
            let position = usize::from(!current);
            match frame {
                InputFrameSummary::Condition { .. } => continue,
                InputFrameSummary::TokenList {
                    token_list,
                    replay_kind,
                    index,
                    ..
                } => {
                    let tokens = stores.tokens(token_list.id());
                    let split = (*index).min(tokens.len());
                    let exhausted = split >= tokens.len();
                    if position != 0 && exhausted && backed_up(*replay_kind) {
                        // §312 omits a fully-read `backed_up` list that is not
                        // the current level.
                        continue;
                    }
                    levels.push(ErrorContextLevel::new(
                        token_list_replay_label(*replay_kind, exhausted),
                        shown_tokens(stores, &tokens[..split]),
                        shown_tokens(stores, &tokens[split..]),
                    ));
                }
                InputFrameSummary::TransientTokenList {
                    tokens,
                    replay_kind,
                    ..
                } => {
                    let exhausted = tokens.is_empty();
                    if position != 0 && exhausted && backed_up(*replay_kind) {
                        continue;
                    }
                    let tokens = tokens
                        .iter()
                        .map(|word| word.semantic_token())
                        .collect::<Vec<_>>();
                    levels.push(ErrorContextLevel::new(
                        token_list_replay_label(*replay_kind, exhausted),
                        "",
                        shown_tokens(stores, &tokens),
                    ));
                }
                InputFrameSummary::Source { source, .. } => {
                    let line = source.normalized_line().trim_end_matches('\r');
                    let split = source.line_byte_offset().min(line.len());
                    let split = (0..=split)
                        .rev()
                        .find(|offset| line.is_char_boundary(*offset))
                        .unwrap_or(0);
                    let (before, after) = line.split_at(split);
                    levels.push(ErrorContextLevel::new(
                        format!("l.{} ", source.line_number()),
                        before,
                        after,
                    ));
                }
            }
            current = false;
        }
        crate::print::render_error_context(
            &levels,
            stores.error_context_widths(),
            stores.int_param(crate::env::banks::IntParam::new(54)),
        )
    }

    /// Exact future input semantics with revision-relative byte coordinates
    /// excluded. The editor checkpoint restore separately proves the mapped
    /// root suffix; this comparison retains line/token content and lexer state
    /// so buffered tokens from a semantic edit cannot spuriously converge.
    pub fn exact_future_state_matches(&self, other: &Self) -> bool {
        let left = &self.semantic_root.0;
        let right = &other.semantic_root.0;
        left.unicode_superscript_notation == right.unicode_superscript_notation
            && left.frames.len() == right.frames.len()
            && left
                .frames
                .iter()
                .zip(&right.frames)
                .all(|(left, right)| input_frame_future_eq(left, right))
            && match (&left.last_source_frame, &right.last_source_frame) {
                (Some(left), Some(right)) => source_frame_future_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        let frames = self
            .semantic_root
            .0
            .frames
            .capacity()
            .saturating_mul(std::mem::size_of::<InputFrameSummary>());
        let transient_words = self
            .semantic_root
            .0
            .frames
            .iter()
            .filter_map(|frame| match frame {
                InputFrameSummary::TransientTokenList { tokens, .. } => Some(tokens.len()),
                InputFrameSummary::Source { .. }
                | InputFrameSummary::TokenList { .. }
                | InputFrameSummary::Condition { .. } => None,
            })
            .sum::<usize>()
            .saturating_mul(std::mem::size_of::<TracedTokenWord>());
        let macro_argument_words = self
            .semantic_root
            .0
            .frames
            .iter()
            .filter_map(|frame| match frame {
                InputFrameSummary::TokenList {
                    macro_arguments, ..
                } => Some(macro_arguments.tokens().len()),
                InputFrameSummary::Source { .. }
                | InputFrameSummary::TransientTokenList { .. }
                | InputFrameSummary::Condition { .. } => None,
            })
            .sum::<usize>()
            .saturating_mul(std::mem::size_of::<TracedTokenWord>());
        std::mem::size_of::<InputSemanticState>()
            .saturating_add(frames)
            .saturating_add(transient_words)
            .saturating_add(macro_argument_words)
    }

    pub(crate) fn semantic_root(&self) -> InputSemanticRoot {
        self.semantic_root.clone()
    }

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
                InputFrameSummary::TokenList { .. }
                | InputFrameSummary::TransientTokenList { .. }
                | InputFrameSummary::Condition { .. } => None,
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
            semantic_root: InputSemanticRoot(Arc::new(InputSemanticState {
                frames,
                last_source_record,
                last_source_frame,
                unicode_superscript_notation,
                utf8_input_as_bytes: false,
            })),
            last_source_id,
            next_source_id,
        }
    }

    #[must_use]
    pub fn frames(&self) -> &[InputFrameSummary] {
        &self.semantic_root.0.frames
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.semantic_root.0.frames.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.semantic_root.0.frames.len()
    }

    /// The most recently popped source frame, retained so a snapshot taken
    /// after source exhaustion can still report the final source coordinates.
    #[must_use]
    pub fn last_source_frame(&self) -> Option<&SourceFrameSummary> {
        self.semantic_root.0.last_source_frame.as_ref()
    }

    /// The stable id for [`Self::last_source_frame`], when one is retained.
    #[must_use]
    pub const fn last_source_id(&self) -> Option<SourceId> {
        self.last_source_id
    }

    /// The `World` input record for [`Self::last_source_frame`].
    #[must_use]
    pub fn last_source_record(&self) -> Option<InputRecordId> {
        self.semantic_root.0.last_source_record
    }

    #[must_use]
    pub const fn next_source_id(&self) -> u32 {
        self.next_source_id
    }

    #[must_use]
    pub fn unicode_superscript_notation(&self) -> bool {
        self.semantic_root.0.unicode_superscript_notation
    }

    #[must_use]
    pub fn utf8_input_as_bytes(&self) -> bool {
        self.semantic_root.0.utf8_input_as_bytes
    }

    /// Selects classic 8-bit TeX tokenization of physical UTF-8 input bytes.
    #[must_use]
    pub fn with_utf8_input_as_bytes(mut self, enabled: bool) -> Self {
        Arc::make_mut(&mut self.semantic_root.0).utf8_input_as_bytes = enabled;
        self
    }

    /// Conservative complete-physical-line position for the root editor source.
    #[must_use]
    pub fn conservative_root_position(&self) -> usize {
        self.frames()
            .iter()
            .find_map(|frame| match frame {
                InputFrameSummary::Source { source, .. } => Some(source.next_source_offset()),
                InputFrameSummary::TokenList { .. }
                | InputFrameSummary::TransientTokenList { .. }
                | InputFrameSummary::Condition { .. } => None,
            })
            .or_else(|| {
                self.last_source_frame()
                    .map(SourceFrameSummary::next_source_offset)
            })
            .unwrap_or(0)
    }

    pub(crate) fn rebind_root_layout(
        &self,
        bytes: &[u8],
        mapped_position: usize,
    ) -> Option<(Self, SourceId)> {
        let mut state = (*self.semantic_root.0).clone();
        let root = state.frames.iter_mut().find_map(|frame| match frame {
            InputFrameSummary::Source {
                source_id,
                input_record,
                source,
            } => Some((source_id, input_record, source)),
            InputFrameSummary::TokenList { .. }
            | InputFrameSummary::TransientTokenList { .. }
            | InputFrameSummary::Condition { .. } => None,
        })?;
        let source_id = *root.0;
        let registration = root.2.registration();
        *root.1 = None;
        *root.2 = if root.2.next_source_offset() == mapped_position {
            root.2.clone()
        } else {
            let line_number = bytes[..mapped_position]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count()
                .saturating_add(1);
            SourceFrameSummary::new_with_physical_metadata(
                mapped_position,
                mapped_position,
                line_number,
                0,
                LexerState::NewLine,
                "",
                0,
                mapped_position,
                mapped_position,
                mapped_position,
                mapped_position,
                None,
                Vec::new(),
                false,
            )
            .with_registration(registration)
        };
        Some((
            Self {
                semantic_root: InputSemanticRoot(Arc::new(state)),
                last_source_id: self.last_source_id,
                next_source_id: self.next_source_id,
            },
            source_id,
        ))
    }
}

fn input_frame_future_eq(left: &InputFrameSummary, right: &InputFrameSummary) -> bool {
    match (left, right) {
        (
            InputFrameSummary::Source { source: left, .. },
            InputFrameSummary::Source { source: right, .. },
        ) => source_frame_future_eq(left, right),
        (
            InputFrameSummary::TokenList {
                token_list: left_list,
                replay_kind: left_kind,
                index: left_index,
                macro_arguments: left_arguments,
                ..
            },
            InputFrameSummary::TokenList {
                token_list: right_list,
                replay_kind: right_kind,
                index: right_index,
                macro_arguments: right_arguments,
                ..
            },
        ) => {
            left_list == right_list
                && left_kind == right_kind
                && left_index == right_index
                && macro_arguments_semantic_eq(left_arguments, right_arguments)
        }
        (
            InputFrameSummary::TransientTokenList {
                tokens: left_tokens,
                replay_kind: left_kind,
                ..
            },
            InputFrameSummary::TransientTokenList {
                tokens: right_tokens,
                replay_kind: right_kind,
                ..
            },
        ) => left_kind == right_kind && traced_tokens_semantic_eq(left_tokens, right_tokens),
        (
            InputFrameSummary::Condition {
                condition: left, ..
            },
            InputFrameSummary::Condition {
                condition: right, ..
            },
        ) => left == right,
        _ => false,
    }
}

fn source_frame_future_eq(left: &SourceFrameSummary, right: &SourceFrameSummary) -> bool {
    left.line_number == right.line_number
        && left.column == right.column
        && left.lexer_state == right.lexer_state
        && left.normalized_line == right.normalized_line
        && left.line_byte_offset == right.line_byte_offset
        && left.synthetic_endline_start == right.synthetic_endline_start
        && left.end_after_current_line == right.end_after_current_line
        && left.scantokens == right.scantokens
        && traced_pending_tokens_eq(&left.pending, &right.pending)
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
        token_list: TokenListRef,
        origin_list: OriginListRef,
        replay_kind: TokenListReplayKind,
        index: usize,
        macro_arguments: MacroArguments,
        macro_invocation: crate::token::OriginId,
        parent_macro_invocation: crate::token::OriginId,
    },
    /// Execution-local token replay with inline per-token provenance.
    ///
    /// The words are the complete unconsumed suffix. Transient replay has no
    /// durable token-list or origin-list identity.
    TransientTokenList {
        tokens: Arc<[TracedTokenWord]>,
        replay_kind: TokenListReplayKind,
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
                    source_id: _,
                    input_record: left_record,
                    source: left,
                },
                Self::Source {
                    source_id: _,
                    input_record: right_record,
                    source: right,
                },
            ) => left_record == right_record && left == right,
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
                    && macro_arguments_semantic_eq(left_arguments, right_arguments)
            }
            (
                Self::TransientTokenList {
                    tokens: left_tokens,
                    replay_kind: left_replay_kind,
                    ..
                },
                Self::TransientTokenList {
                    tokens: right_tokens,
                    replay_kind: right_replay_kind,
                    ..
                },
            ) => {
                left_replay_kind == right_replay_kind
                    && traced_tokens_semantic_eq(left_tokens, right_tokens)
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
                source_id: _,
                input_record,
                source,
            } => {
                0_u8.hash(state);
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
                hash_macro_arguments_semantic(macro_arguments, state);
            }
            Self::TransientTokenList {
                tokens,
                replay_kind,
                ..
            } => {
                2_u8.hash(state);
                replay_kind.hash(state);
                for word in tokens.iter().copied() {
                    word.token().hash(state);
                }
            }
            Self::Condition {
                token: _,
                condition,
            } => {
                3_u8.hash(state);
                condition.hash(state);
            }
        }
    }
}

fn traced_tokens_semantic_eq(left: &[TracedTokenWord], right: &[TracedTokenWord]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(&left, &right)| left.token() == right.token())
}

fn macro_arguments_semantic_eq(left: &MacroArguments, right: &MacroArguments) -> bool {
    (1..=MACRO_ARGUMENT_SLOTS as u8).all(|slot| match (left.get(slot), right.get(slot)) {
        (Some(left), Some(right)) => traced_tokens_semantic_eq(left, right),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    })
}

fn hash_macro_arguments_semantic<H: Hasher>(arguments: &MacroArguments, state: &mut H) {
    for slot in 1..=MACRO_ARGUMENT_SLOTS as u8 {
        match arguments.get(slot) {
            Some(words) => {
                true.hash(state);
                words.len().hash(state);
                for &word in words {
                    word.token().hash(state);
                }
            }
            None => false.hash(state),
        }
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
    normalized_line: Arc<str>,
    line_byte_offset: usize,
    physical_content_end: usize,
    /// Provenance-only coordinate base for the current physical line. This is
    /// fragment-relative for editor layouts and document-relative otherwise.
    origin_line_start: u64,
    terminator_start: usize,
    terminator_end: usize,
    normalized_end_anchor: usize,
    synthetic_endline_start: Option<usize>,
    pending: Arc<[TracedTokenWord]>,
    end_after_current_line: bool,
    registration: Option<RegisteredSource>,
    scantokens: bool,
    byte_oriented: bool,
    bytes_as_chars: bool,
    byte_projection: bool,
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
        normalized_line: impl Into<Arc<str>>,
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
            normalized_line: normalized_line.into(),
            line_byte_offset,
            physical_content_end,
            origin_line_start: u64::try_from(buffer_offset).unwrap_or(u64::MAX),
            terminator_start,
            terminator_end,
            normalized_end_anchor,
            synthetic_endline_start,
            pending: pending.into(),
            end_after_current_line,
            registration: None,
            scantokens: false,
            byte_oriented: false,
            bytes_as_chars: false,
            byte_projection: false,
        }
    }

    #[must_use]
    pub const fn is_scantokens(&self) -> bool {
        self.scantokens
    }

    #[must_use]
    pub const fn with_scantokens(mut self, scantokens: bool) -> Self {
        self.scantokens = scantokens;
        self
    }

    /// Allows a resumable cursor to lie between bytes of one UTF-8 scalar.
    #[must_use]
    pub const fn with_byte_oriented(mut self, byte_oriented: bool) -> Self {
        self.byte_oriented = byte_oriented;
        self
    }

    /// Marks a physical line whose Unicode scalars are a one-for-one view of
    /// source bytes rather than UTF-8 decoding.
    #[must_use]
    pub const fn with_bytes_as_chars(mut self, bytes_as_chars: bool) -> Self {
        self.bytes_as_chars = bytes_as_chars;
        self
    }

    /// Marks a valid-Unicode editor buffer whose scalars are a lossless
    /// one-byte-to-one-character projection of an original 8-bit file.
    #[must_use]
    pub const fn with_byte_projection(mut self, byte_projection: bool) -> Self {
        self.byte_projection = byte_projection;
        self
    }

    #[must_use]
    pub const fn byte_projection(&self) -> bool {
        self.byte_projection
    }

    /// Attaches the live aggregate source registration used by this frame.
    #[must_use]
    pub const fn with_registration(mut self, registration: Option<RegisteredSource>) -> Self {
        self.registration = registration;
        self
    }

    /// Attaches the provenance coordinate base selected at physical-line refill.
    #[must_use]
    pub const fn with_origin_line_start(mut self, origin_line_start: u64) -> Self {
        self.origin_line_start = origin_line_start;
        self
    }

    /// Returns the provenance coordinate base for the current physical line.
    #[must_use]
    pub const fn origin_line_start(&self) -> u64 {
        self.origin_line_start
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
        if self.bytes_as_chars {
            return self.line_byte_offset;
        }
        self.normalized_line[..self.line_byte_offset]
            .chars()
            .count()
    }

    #[must_use]
    pub fn line_byte_offset(&self) -> usize {
        self.line_byte_offset
    }

    #[must_use]
    pub const fn bytes_as_chars(&self) -> bool {
        self.bytes_as_chars
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
        let cursor_len = if self.bytes_as_chars {
            self.normalized_line.chars().count()
        } else {
            self.normalized_line.len()
        };
        self.line_byte_offset <= cursor_len
            && (self.bytes_as_chars
                || self.byte_oriented
                || self.normalized_line.is_char_boundary(self.line_byte_offset))
            && self.buffer_offset <= self.normalized_end_anchor
            && self.normalized_end_anchor <= self.physical_content_end
            && self.physical_content_end <= self.terminator_start
            && self.terminator_start <= self.terminator_end
            && self.terminator_end <= self.next_source_offset
            && self.synthetic_endline_start.is_none_or(|offset| {
                offset <= cursor_len
                    && (self.bytes_as_chars || self.normalized_line.is_char_boundary(offset))
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
            && self.scantokens == other.scantokens
            && self.byte_oriented == other.byte_oriented
            && self.bytes_as_chars == other.bytes_as_chars
            && self.byte_projection == other.byte_projection
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
        for token in self.pending.iter() {
            semantic_token(*token).hash(state);
        }
        self.end_after_current_line.hash(state);
        self.scantokens.hash(state);
        self.byte_oriented.hash(state);
        self.bytes_as_chars.hash(state);
        self.byte_projection.hash(state);
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

#[cfg(test)]
mod tests;
