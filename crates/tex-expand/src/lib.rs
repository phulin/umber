//! TeX expansion engine core loop.
//!
//! This crate owns the gullet's single `get_x_token` interpreter loop. It
//! reads meanings through the aggregate state facade and pushes expansion
//! output back through `tex-lex` token-list replay frames.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::Path;

use tex_lex::{InputSource, InputStack, LexError, MacroArguments, TokenListReplayKind};
use tex_state::glue::GlueSpec;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::provenance::{DiagnosticSite, InsertedOriginKind, SynthesizedOriginKind};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionState, FileContent, InputOpenState, InputReadState, Universe};

pub mod args;
pub mod scan;
pub mod scan_dimen;
pub mod scan_glue;
pub mod scan_int;

mod conditionals;
mod dispatch;
mod primitives;
mod scan_helpers;
#[cfg(test)]
mod tests;
mod values;

pub use dispatch::{dispatch, dispatch_expandable_opcode, dispatch_with_hooks};
pub use scan_helpers::scan_optional_keyword_with_hooks;
pub use values::{meaning_text, scan_the_text_with_hooks, token_text};

/// Installs the expandable TeX82 primitives currently implemented by this
/// crate into the provided state facade.
pub fn install_expandable_primitives(stores: &mut Universe) {
    for (name, primitive) in [
        (
            "expandafter",
            tex_state::meaning::ExpandablePrimitive::ExpandAfter,
        ),
        (
            "noexpand",
            tex_state::meaning::ExpandablePrimitive::NoExpand,
        ),
        ("csname", tex_state::meaning::ExpandablePrimitive::CsName),
        (
            "endcsname",
            tex_state::meaning::ExpandablePrimitive::EndCsName,
        ),
        ("string", tex_state::meaning::ExpandablePrimitive::String),
        ("number", tex_state::meaning::ExpandablePrimitive::Number),
        (
            "romannumeral",
            tex_state::meaning::ExpandablePrimitive::RomanNumeral,
        ),
        ("meaning", tex_state::meaning::ExpandablePrimitive::Meaning),
        ("the", tex_state::meaning::ExpandablePrimitive::The),
        ("input", tex_state::meaning::ExpandablePrimitive::Input),
        (
            "endinput",
            tex_state::meaning::ExpandablePrimitive::EndInput,
        ),
        ("jobname", tex_state::meaning::ExpandablePrimitive::JobName),
        (
            "fontname",
            tex_state::meaning::ExpandablePrimitive::FontName,
        ),
        ("topmark", tex_state::meaning::ExpandablePrimitive::TopMark),
        (
            "firstmark",
            tex_state::meaning::ExpandablePrimitive::FirstMark,
        ),
        ("botmark", tex_state::meaning::ExpandablePrimitive::BotMark),
        (
            "splitfirstmark",
            tex_state::meaning::ExpandablePrimitive::SplitFirstMark,
        ),
        (
            "splitbotmark",
            tex_state::meaning::ExpandablePrimitive::SplitBotMark,
        ),
        ("iftrue", tex_state::meaning::ExpandablePrimitive::IfTrue),
        ("iffalse", tex_state::meaning::ExpandablePrimitive::IfFalse),
        ("if", tex_state::meaning::ExpandablePrimitive::If),
        ("ifcat", tex_state::meaning::ExpandablePrimitive::IfCat),
        ("ifx", tex_state::meaning::ExpandablePrimitive::IfX),
        ("ifnum", tex_state::meaning::ExpandablePrimitive::IfNum),
        ("ifdim", tex_state::meaning::ExpandablePrimitive::IfDim),
        ("ifodd", tex_state::meaning::ExpandablePrimitive::IfOdd),
        ("ifcase", tex_state::meaning::ExpandablePrimitive::IfCase),
        ("ifvmode", tex_state::meaning::ExpandablePrimitive::IfVMode),
        ("ifhmode", tex_state::meaning::ExpandablePrimitive::IfHMode),
        ("ifmmode", tex_state::meaning::ExpandablePrimitive::IfMMode),
        ("ifinner", tex_state::meaning::ExpandablePrimitive::IfInner),
        ("ifvoid", tex_state::meaning::ExpandablePrimitive::IfVoid),
        ("ifhbox", tex_state::meaning::ExpandablePrimitive::IfHBox),
        ("ifvbox", tex_state::meaning::ExpandablePrimitive::IfVBox),
        ("ifeof", tex_state::meaning::ExpandablePrimitive::IfEof),
        ("else", tex_state::meaning::ExpandablePrimitive::Else),
        ("or", tex_state::meaning::ExpandablePrimitive::Or),
        ("fi", tex_state::meaning::ExpandablePrimitive::Fi),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    }
}

/// Records state reads performed by expansion.
///
/// The default implementation is `NoopRecorder`. Callers that need read sets
/// can supply a concrete recorder type and let monomorphization remove this
/// hook from ordinary builds.
pub trait ReadRecorder {
    fn record_meaning(&mut self, symbol: Symbol, meaning: Meaning);
}

/// Read recorder used when expansion tracing/incremental read sets are off.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopRecorder;

impl ReadRecorder for NoopRecorder {
    #[inline(always)]
    fn record_meaning(&mut self, _symbol: Symbol, _meaning: Meaning) {}
}

/// Why `tex-expand` is replaying a frozen token list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpansionReplayKind {
    MacroBody,
    TheOutput,
    NumberOutput,
    JobName,
    Mark,
    Inserted,
}

impl ExpansionReplayKind {
    #[must_use]
    pub const fn as_lex_kind(self) -> TokenListReplayKind {
        match self {
            Self::MacroBody => TokenListReplayKind::MacroBody,
            Self::TheOutput | Self::NumberOutput | Self::JobName => TokenListReplayKind::Inserted,
            Self::Mark => TokenListReplayKind::Mark,
            Self::Inserted => TokenListReplayKind::Inserted,
        }
    }
}

/// Expandable operation families owned by the gullet epic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpandableOpcode {
    Macro,
    ExpandAfter,
    NoExpand,
    CsName,
    EndCsName,
    String,
    Number,
    RomanNumeral,
    Meaning,
    The,
    Input,
    EndInput,
    JobName,
    FontName,
    Mark,
    If,
    Else,
    Or,
    Fi,
}

/// Current semantic mode as reported by the engine driver.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EngineMode {
    #[default]
    Vertical,
    Horizontal,
    Math,
}

/// Read-only execution facts needed by expansion-time internal quantities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineStateSnapshot {
    pub mode: EngineMode,
    pub is_inner_mode: bool,
    pub space_factor: i32,
    pub prev_depth: Scaled,
    pub prev_graf: i32,
    pub last_penalty: i32,
    pub last_kern: Scaled,
    pub last_skip: GlueSpec,
}

impl Default for EngineStateSnapshot {
    fn default() -> Self {
        Self {
            mode: EngineMode::Vertical,
            is_inner_mode: false,
            space_factor: 1000,
            prev_depth: Scaled::from_raw(0),
            prev_graf: 0,
            last_penalty: 0,
            last_kern: Scaled::from_raw(0),
            last_skip: GlueSpec::ZERO,
        }
    }
}

/// Result of one expansion dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dispatch {
    Continue,
    Deliver(TracedTokenWord),
    DeliverNoExpand(TracedTokenWord),
    Push {
        replay_kind: ExpansionReplayKind,
        token_list: tex_state::ids::TokenListId,
        origin_list: tex_state::ids::OriginListId,
        macro_arguments: MacroArguments,
        macro_invocation: OriginId,
    },
}

/// Errors raised by `get_x_token`.
#[derive(Debug)]
pub enum ExpandError {
    Captured {
        error: Box<ExpandError>,
        site: DiagnosticSite,
    },
    Lex(LexError),
    MacroCall(args::MacroCallError),
    UnimplementedExpandable {
        opcode: ExpandableOpcode,
        context: TracedTokenWord,
    },
    MissingTokenAfterPrimitive {
        opcode: ExpandableOpcode,
        context: TracedTokenWord,
    },
    MissingEndCsName {
        context: TracedTokenWord,
    },
    MissingInputName {
        context: TracedTokenWord,
    },
    NonCharacterInInputName {
        context: TracedTokenWord,
    },
    InputOpen {
        name: String,
        message: String,
        context: TracedTokenWord,
    },
    UndefinedControlSequence {
        name: String,
        context: TracedTokenWord,
    },
    ScanInt(Box<scan_int::ScanIntError>),
    ScanDimen(Box<scan_dimen::ScanDimenError>),
    UnsupportedTheTarget {
        context: TracedTokenWord,
    },
    MissingFontIdentifier {
        context: TracedTokenWord,
    },
    MathFamilyOutOfRange {
        value: i32,
        context: TracedTokenWord,
    },
    FontDimenOutOfRange {
        font_name: String,
        number: i32,
        available: u16,
        context: TracedTokenWord,
    },
    InvalidConditionalRelation {
        context: TracedTokenWord,
    },
    IncompleteIf {
        context: TracedTokenWord,
    },
    ExtraConditionalControl {
        name: &'static str,
        context: TracedTokenWord,
    },
    ForbiddenOuterTokenInSkippedConditional {
        name: String,
        context: TracedTokenWord,
    },
}

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Captured { error, .. } => write!(f, "{error}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::MacroCall(err) => write!(f, "{err}"),
            Self::UnimplementedExpandable { opcode, .. } => {
                write!(f, "expandable opcode {opcode:?} is not implemented yet")
            }
            Self::MissingTokenAfterPrimitive { opcode, .. } => {
                write!(f, "missing token after expandable primitive {opcode:?}")
            }
            Self::MissingEndCsName { .. } => write!(f, "missing \\endcsname for \\csname"),
            Self::MissingInputName { .. } => write!(f, "missing file name after \\input"),
            Self::NonCharacterInInputName { context } => {
                write!(
                    f,
                    "non-character token {:?} while scanning \\input file name",
                    semantic_token(*context)
                )
            }
            Self::InputOpen { name, message, .. } => {
                write!(f, "failed to open input {name:?}: {message}")
            }
            Self::UndefinedControlSequence { name, .. } => {
                write!(f, "Undefined control sequence \\{name}")
            }
            Self::ScanInt(err) => write!(f, "{err}"),
            Self::ScanDimen(err) => write!(f, "{err}"),
            Self::UnsupportedTheTarget { context } => {
                write!(
                    f,
                    "unsupported token {:?} after \\the",
                    semantic_token(*context)
                )
            }
            Self::MissingFontIdentifier { context } => write!(
                f,
                "missing font identifier at token {:?}",
                semantic_token(*context)
            ),
            Self::MathFamilyOutOfRange { .. } => f.write_str("Bad number"),
            Self::FontDimenOutOfRange {
                font_name,
                available,
                ..
            } => write!(
                f,
                "Font \\{font_name} has only {available} fontdimen parameters"
            ),
            Self::InvalidConditionalRelation { context } => {
                write!(
                    f,
                    "invalid conditional relation token {:?}",
                    semantic_token(*context)
                )
            }
            Self::IncompleteIf { .. } => {
                write!(f, "Incomplete \\if; all text was ignored after line")
            }
            Self::ExtraConditionalControl { name, .. } => write!(f, "Extra \\{name}"),
            Self::ForbiddenOuterTokenInSkippedConditional { name, .. } => {
                write!(
                    f,
                    "Forbidden control sequence found while scanning conditional text: {name}"
                )
            }
        }
    }
}

impl std::error::Error for ExpandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Captured { error, .. } => Some(error),
            Self::Lex(err) => Some(err),
            Self::MacroCall(err) => Some(err),
            Self::ScanInt(err) => Some(err),
            Self::ScanDimen(err) => Some(err),
            Self::UnimplementedExpandable { .. }
            | Self::MissingTokenAfterPrimitive { .. }
            | Self::MissingEndCsName { .. }
            | Self::MissingInputName { .. }
            | Self::NonCharacterInInputName { .. }
            | Self::InputOpen { .. }
            | Self::UndefinedControlSequence { .. }
            | Self::UnsupportedTheTarget { .. }
            | Self::MissingFontIdentifier { .. }
            | Self::MathFamilyOutOfRange { .. }
            | Self::FontDimenOutOfRange { .. }
            | Self::InvalidConditionalRelation { .. }
            | Self::IncompleteIf { .. }
            | Self::ExtraConditionalControl { .. }
            | Self::ForbiddenOuterTokenInSkippedConditional { .. } => None,
        }
    }
}

impl ExpandError {
    #[must_use]
    pub fn primary_origin(&self) -> Option<OriginId> {
        match self {
            Self::Captured { site, .. } => site.primary_origin(),
            Self::UnimplementedExpandable { context, .. }
            | Self::MissingTokenAfterPrimitive { context, .. }
            | Self::MissingEndCsName { context }
            | Self::MissingInputName { context }
            | Self::InputOpen { context, .. }
            | Self::UndefinedControlSequence { context, .. }
            | Self::ExtraConditionalControl { context, .. }
            | Self::ForbiddenOuterTokenInSkippedConditional { context, .. } => {
                Some(context.origin())
            }
            Self::NonCharacterInInputName { context }
            | Self::UnsupportedTheTarget { context }
            | Self::MissingFontIdentifier { context }
            | Self::MathFamilyOutOfRange { context, .. }
            | Self::FontDimenOutOfRange { context, .. }
            | Self::InvalidConditionalRelation { context }
            | Self::IncompleteIf { context } => Some(context.origin()),
            Self::ScanInt(err) => err.primary_origin(),
            Self::ScanDimen(err) => err.primary_origin(),
            Self::MacroCall(err) => err.primary_origin(),
            Self::Lex(err) => err.diagnostic_site().primary_origin(),
        }
    }

    #[must_use]
    pub fn diagnostic_site(&self) -> DiagnosticSite {
        match self {
            Self::Captured { site, .. } => site.clone(),
            Self::Lex(err) => err.diagnostic_site().clone(),
            _ => DiagnosticSite::new(self.primary_origin(), [], []),
        }
    }

    #[cold]
    #[inline(never)]
    fn capture<S: InputSource>(self, input: &InputStack<S>) -> Self {
        if matches!(self, Self::Captured { .. }) {
            return self;
        }
        let site = input.diagnostic_site(self.primary_origin(), []);
        if site.expansion_trace().is_empty() {
            self
        } else {
            Self::Captured {
                error: Box::new(self),
                site,
            }
        }
    }
}

/// Driver hooks for expandable primitives that need outside-world state.
///
/// `tex-expand` never opens files itself. A driver or test harness supplies
/// sources through this trait; the eventual `World` implementation is expected
/// to record and snapshot those reads.
pub trait ExpansionHooks<S> {
    fn open_input<C: InputReadState>(&mut self, input: &mut C, name: &str) -> Result<S, String>;

    fn open_font<C: InputReadState>(
        &mut self,
        input: &mut C,
        path: &Path,
    ) -> Result<FileContent, String> {
        input.read_input_file(path).map_err(|err| err.to_string())
    }

    fn job_name(&self) -> &str {
        "texput"
    }

    fn mode(&self) -> EngineMode {
        EngineMode::Vertical
    }

    fn is_inner_mode(&self) -> bool {
        false
    }

    fn space_factor(&self) -> i32 {
        1000
    }

    fn prev_depth(&self) -> Scaled {
        Scaled::from_raw(0)
    }

    fn prev_graf(&self) -> i32 {
        0
    }

    fn last_penalty(&self) -> i32 {
        0
    }

    fn last_kern(&self) -> Scaled {
        Scaled::from_raw(0)
    }

    fn last_skip(&self) -> GlueSpec {
        GlueSpec::ZERO
    }

    fn input_stream_eof(&self, stores: &impl ExpansionState, stream: u8) -> bool {
        if stream >= tex_state::world::STREAM_SLOT_COUNT as u8 {
            return true;
        }
        stores.input_stream_eof(tex_state::StreamSlot::new(stream))
    }

    fn set_engine_state(&mut self, _state: EngineStateSnapshot) {}
}

pub trait ExpandNext<S, St: ExpansionState, R, H> {
    fn next_expanded_token(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<Option<TracedTokenWord>, ExpandError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoInputExpandNext;

impl<S, St, R, H> ExpandNext<S, St, R, H> for NoInputExpandNext
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    fn next_expanded_token(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<Option<TracedTokenWord>, ExpandError> {
        get_x_token_without_input_open(input, stores, recorder, hooks)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DriverExpandNext;

impl<S, St, R, H> ExpandNext<S, St, R, H> for DriverExpandNext
where
    S: InputSource,
    St: ExpansionState + InputOpenState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    fn next_expanded_token(
        &mut self,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<Option<TracedTokenWord>, ExpandError> {
        get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopExpansionHooks;

impl<S> ExpansionHooks<S> for NoopExpansionHooks {
    fn open_input<C: InputReadState>(&mut self, _input: &mut C, _name: &str) -> Result<S, String> {
        Err("no input source hook is installed".to_owned())
    }
}

impl From<LexError> for ExpandError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<args::MacroCallError> for ExpandError {
    fn from(value: args::MacroCallError) -> Self {
        Self::MacroCall(value)
    }
}

impl From<scan_int::ScanIntError> for ExpandError {
    fn from(value: scan_int::ScanIntError) -> Self {
        Self::ScanInt(Box::new(value))
    }
}

impl From<scan_dimen::ScanDimenError> for ExpandError {
    fn from(value: scan_dimen::ScanDimenError) -> Self {
        Self::ScanDimen(Box::new(value))
    }
}

/// Pulls the next fully expanded token.
pub fn get_x_token<S>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
{
    get_x_token_with_recorder_and_hooks(input, stores, &mut NoopRecorder, &mut NoopExpansionHooks)
}

/// Pulls the next fully expanded token while recording meaning reads.
pub fn get_x_token_with_recorder<S, R>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
{
    get_x_token_with_recorder_and_hooks(input, stores, recorder, &mut NoopExpansionHooks)
}

/// Pulls the next fully expanded token using driver-provided expansion hooks.
pub fn get_x_token_with_hooks<S, H>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    hooks: &mut H,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    get_x_token_with_recorder_and_hooks(input, stores, &mut NoopRecorder, hooks)
}

/// Pulls the next fully expanded token while recording reads and using hooks.
pub fn get_x_token_with_recorder_and_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    match get_x_token_with_recorder_and_hooks_inner(input, stores, recorder, hooks) {
        Ok(token) => Ok(token),
        Err(error) => Err(error.capture(input)),
    }
}

fn get_x_token_with_recorder_and_hooks_inner<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    loop {
        let Some(read) = input.next_traced_expansion_token(stores)? else {
            return Ok(None);
        };
        let token = read.token();
        let traced = read.traced_token();

        if token.is_frozen_end_template() {
            return Ok(Some(TracedTokenWord::pack(
                Token::frozen_endv(),
                read.origin(),
            )));
        }

        if read.suppress_expansion() {
            if intercept_alignment_token(input, stores, traced) {
                continue;
            }
            return Ok(Some(traced));
        }

        let symbol = match expandable_symbol(stores, traced) {
            Some(symbol) => symbol,
            None => {
                if intercept_alignment_token(input, stores, traced) {
                    continue;
                }
                return Ok(Some(traced));
            }
        };

        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);

        match dispatch_with_hooks(
            token,
            read.origin(),
            input,
            stores,
            recorder,
            hooks,
            meaning,
        )? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => {
                if intercept_alignment_token(input, stores, token) {
                    continue;
                }
                return Ok(Some(token));
            }
            push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
        }
    }
}

pub(crate) fn intercept_alignment_token<S>(
    input: &mut InputStack<S>,
    stores: &impl ExpansionState,
    traced: TracedTokenWord,
) -> bool {
    let token = semantic_token(traced);
    let meaning = match token {
        Token::Cs(symbol) => Some(stores.meaning(symbol)),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => stores
            .active_character_symbol(ch)
            .map(|symbol| stores.meaning(symbol)),
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => None,
    };
    let terminator = if matches!(
        token,
        Token::Char {
            cat: Catcode::AlignmentTab,
            ..
        }
    ) || matches!(
        meaning,
        Some(Meaning::CharToken {
            cat: Catcode::AlignmentTab,
            ..
        })
    ) {
        Some(tex_lex::AlignmentTerminator::Tab)
    } else {
        match meaning {
            Some(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr,
            )) => Some(tex_lex::AlignmentTerminator::Cr),
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Span)) => {
                Some(tex_lex::AlignmentTerminator::Span)
            }
            _ => None,
        }
    };
    input.intercept_alignment_token(traced, terminator, stores.execution_group_depth())
}

pub(crate) fn get_x_token_without_input_open<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    loop {
        let Some(read) = input.next_traced_expansion_token(stores)? else {
            return Ok(None);
        };
        let token = read.token();
        let traced = read.traced_token();

        if read.suppress_expansion() {
            if intercept_alignment_token(input, stores, traced) {
                continue;
            }
            return Ok(Some(traced));
        }

        let symbol = match expandable_symbol(stores, traced) {
            Some(symbol) => symbol,
            None => {
                if intercept_alignment_token(input, stores, traced) {
                    continue;
                }
                return Ok(Some(traced));
            }
        };

        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);

        match dispatch::dispatch_without_input_open(
            token,
            read.origin(),
            input,
            stores,
            recorder,
            hooks,
            meaning,
        )? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => {
                if intercept_alignment_token(input, stores, token) {
                    continue;
                }
                return Ok(Some(token));
            }
            push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
        }
    }
}

pub(crate) fn dispatch_one_raw_token_with_hooks<S, R, H>(
    token: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let semantic = semantic_token(token);
    let symbol = match expandable_symbol(stores, token) {
        Some(symbol) => symbol,
        None => return Ok(Dispatch::Deliver(token)),
    };

    let meaning = stores.meaning(symbol);
    recorder.record_meaning(symbol, meaning);
    dispatch::dispatch_without_input_open(
        semantic,
        token.origin(),
        input,
        stores,
        recorder,
        hooks,
        meaning,
    )
}

pub(crate) fn expandable_symbol(
    stores: &mut impl ExpansionState,
    token: TracedTokenWord,
) -> Option<Symbol> {
    match semantic_token(token) {
        Token::Cs(symbol) => Some(symbol),
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => Some(stores.intern_active_character(ch)),
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => None,
    }
}

pub(crate) fn push_dispatch_result<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    dispatch: Dispatch,
) {
    match dispatch {
        Dispatch::Deliver(token) => {
            push_inserted_token(input, stores, token, InsertedOriginKind::ExpandAfter);
        }
        Dispatch::DeliverNoExpand(token) => push_noexpand_token(input, stores, token),
        Dispatch::Continue => {}
        push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
    }
}

pub(crate) fn apply_dispatch_push<S>(input: &mut InputStack<S>, dispatch: Dispatch) {
    let Dispatch::Push {
        replay_kind,
        token_list,
        origin_list,
        macro_arguments,
        macro_invocation,
    } = dispatch
    else {
        return;
    };

    if replay_kind == ExpansionReplayKind::MacroBody {
        input.push_macro_body_with_origins_and_invocation(
            token_list,
            origin_list,
            macro_arguments,
            macro_invocation,
        );
    } else {
        input.push_token_list_with_origins(token_list, origin_list, replay_kind.as_lex_kind());
    }
}

pub(crate) fn push_inserted_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    token: TracedTokenWord,
    kind: InsertedOriginKind,
) {
    let semantic = semantic_token(token);
    let token_list = stores.intern_token_list(&[semantic]);
    let origin_list = inserted_origin_list(stores, &[token], kind);
    input.push_token_list_with_origins(token_list, origin_list, TokenListReplayKind::Inserted);
}

pub(crate) fn push_noexpand_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    token: TracedTokenWord,
) {
    let semantic = semantic_token(token);
    let token_list = stores.intern_token_list(&[semantic]);
    let origin_list = inserted_origin_list(stores, &[token], InsertedOriginKind::NoExpand);
    input.push_token_list_with_origins(token_list, origin_list, TokenListReplayKind::NoExpand);
}

pub(crate) fn inserted_origin_list(
    stores: &mut impl ExpansionState,
    tokens: &[TracedTokenWord],
    kind: InsertedOriginKind,
) -> tex_state::ids::OriginListId {
    let mut origins = stores.origin_list_builder();
    for &token in tokens {
        origins.push(stores.inserted_origin(kind, semantic_token(token), token.origin()));
    }
    stores.finish_origin_list(&mut origins)
}

pub(crate) fn synthesized_origin_list(
    stores: &mut impl ExpansionState,
    len: usize,
    parent: OriginId,
    kind: SynthesizedOriginKind,
) -> tex_state::ids::OriginListId {
    let origin = stores.synthesized_origin(kind, parent);
    stores.allocate_repeated_origin_list(origin, len)
}

pub fn semantic_token(token: TracedTokenWord) -> Token {
    token
        .token()
        .expect("expansion must only receive valid traced tokens")
}
