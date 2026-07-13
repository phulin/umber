//! TeX expansion engine core loop.
//!
//! This crate owns the gullet's single `get_x_token` interpreter loop. It
//! reads meanings through the aggregate state facade and pushes expansion
//! output back through `tex-lex` token-list replay frames.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::Path;

use tex_lex::{
    InputSource, InputStack, LexError, MacroArguments, TokenListReplayKind, TracedExpansionToken,
};
use tex_state::glue::GlueSpec;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
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
pub use values::{
    append_token_show_text, append_token_string_text, meaning_text, scan_the_text_with_hooks,
    token_text,
};

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

/// Installs expandable primitives that exist only in e-TeX extended mode.
pub fn install_etex_expandable_primitives(stores: &mut Universe) {
    stores.set_int_param_global(tex_state::env::banks::IntParam::ETEX_EXTENDED_MODE, 1);
    for (name, primitive) in [
        (
            "unexpanded",
            tex_state::meaning::ExpandablePrimitive::Unexpanded,
        ),
        (
            "detokenize",
            tex_state::meaning::ExpandablePrimitive::Detokenize,
        ),
        ("unless", tex_state::meaning::ExpandablePrimitive::Unless),
        (
            "scantokens",
            tex_state::meaning::ExpandablePrimitive::Scantokens,
        ),
        (
            "eTeXrevision",
            tex_state::meaning::ExpandablePrimitive::ETeXRevision,
        ),
        (
            "ifdefined",
            tex_state::meaning::ExpandablePrimitive::IfDefined,
        ),
        (
            "ifcsname",
            tex_state::meaning::ExpandablePrimitive::IfCsName,
        ),
        (
            "iffontchar",
            tex_state::meaning::ExpandablePrimitive::IfFontChar,
        ),
        (
            "topmarks",
            tex_state::meaning::ExpandablePrimitive::TopMarks,
        ),
        (
            "firstmarks",
            tex_state::meaning::ExpandablePrimitive::FirstMarks,
        ),
        (
            "botmarks",
            tex_state::meaning::ExpandablePrimitive::BotMarks,
        ),
        (
            "splitfirstmarks",
            tex_state::meaning::ExpandablePrimitive::SplitFirstMarks,
        ),
        (
            "splitbotmarks",
            tex_state::meaning::ExpandablePrimitive::SplitBotMarks,
        ),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    }
    for (name, value) in [
        (
            "eTeXversion",
            tex_state::meaning::InternalInteger::ETeXVersion,
        ),
        (
            "currentgrouplevel",
            tex_state::meaning::InternalInteger::CurrentGroupLevel,
        ),
        (
            "currentgrouptype",
            tex_state::meaning::InternalInteger::CurrentGroupType,
        ),
        (
            "currentiflevel",
            tex_state::meaning::InternalInteger::CurrentIfLevel,
        ),
        (
            "currentiftype",
            tex_state::meaning::InternalInteger::CurrentIfType,
        ),
        (
            "currentifbranch",
            tex_state::meaning::InternalInteger::CurrentIfBranch,
        ),
        (
            "lastnodetype",
            tex_state::meaning::InternalInteger::LastNodeType,
        ),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::InternalInteger(value));
    }
}

/// Records state reads performed by expansion.
///
/// The default implementation is `NoopRecorder`. Callers that need read sets
/// can supply a concrete recorder type and let monomorphization remove this
/// hook from ordinary builds.
pub trait ReadRecorder {
    fn record_meaning(&mut self, symbol: Symbol, _meaning: Meaning) {
        self.record_dependency(ReadDependency::Meaning(symbol.raw()));
    }

    fn record_dependency(&mut self, _dependency: ReadDependency) {}
}

/// Typed semantic keys read by expansion and scanners.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReadDependency {
    Meaning(u32),
    Cell {
        bank: ReadBank,
        index: u32,
    },
    Code {
        table: ReadCodeTable,
        scalar: u32,
    },
    CodeGeneration(ReadCodeTable),
    Font {
        field: ReadFontField,
        font: u32,
        index: u16,
    },
    PageDimension(u8),
    PageInteger(u8),
    PageMark(u8),
    PageMarkClass {
        mark: u8,
        class: u16,
    },
    InputLine,
    InputStream(u8),
    Engine(ReadEngineField),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReadBank {
    Count,
    Dimen,
    Skip,
    Muskip,
    Toks,
    Box,
    IntParam,
    DimenParam,
    GlueParam,
    TokParam,
    CurrentFont,
    MathFamilyFont,
    LastBadness,
    Magnification,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReadEngineField {
    Mode,
    InnerMode,
    GroupLevel,
    GroupType,
    ConditionLevel,
    ConditionType,
    ConditionBranch,
    ConditionStack,
    LastNodeType,
    ParShape,
    PenaltyArrays,
    InteractionMode,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReadCodeTable {
    Catcode,
    Lccode,
    Uccode,
    Sfcode,
    Mathcode,
    Delcode,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReadFontField {
    Identifier,
    Name,
    Parameter,
    ParameterCount,
    HyphenChar,
    SkewChar,
    Metrics,
}

/// Deterministic concrete recorder for memoization and speculation clients.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReadSetRecorder {
    dependencies: std::collections::BTreeSet<ReadDependency>,
}

impl ReadSetRecorder {
    #[must_use]
    pub fn dependencies(&self) -> impl ExactSizeIterator<Item = ReadDependency> + '_ {
        self.dependencies.iter().copied()
    }
}

impl ReadRecorder for ReadSetRecorder {
    fn record_dependency(&mut self, dependency: ReadDependency) {
        self.dependencies.insert(dependency);
    }
}

pub(crate) fn record_code_dependency(
    recorder: &mut impl ReadRecorder,
    table: ReadCodeTable,
    ch: char,
) {
    recorder.record_dependency(ReadDependency::CodeGeneration(table));
    recorder.record_dependency(ReadDependency::Code {
        table,
        scalar: ch as u32,
    });
}

/// Read recorder used when expansion tracing/incremental read sets are off.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopRecorder;

impl ReadRecorder for NoopRecorder {
    #[inline(always)]
    fn record_meaning(&mut self, _symbol: Symbol, _meaning: Meaning) {}

    #[inline(always)]
    fn record_dependency(&mut self, _dependency: ReadDependency) {}
}

/// Why `tex-expand` is replaying a frozen token list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpansionReplayKind {
    MacroBody,
    TheOutput,
    NumberOutput,
    JobName,
    Mark,
    Unexpanded,
    Inserted,
}

impl ExpansionReplayKind {
    #[must_use]
    pub const fn as_lex_kind(self) -> TokenListReplayKind {
        match self {
            Self::MacroBody => TokenListReplayKind::MacroBody,
            Self::TheOutput | Self::NumberOutput | Self::JobName => TokenListReplayKind::Inserted,
            Self::Mark => TokenListReplayKind::Mark,
            Self::Unexpanded => TokenListReplayKind::NoExpand,
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
    Unexpanded,
    Detokenize,
    Unless,
    Scantokens,
    ETeXVersion,
    ETeXRevision,
    IfDefined,
    IfCsName,
    IfFontChar,
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
    pub par_shape_len: i32,
    pub last_penalty: i32,
    pub last_kern: Scaled,
    pub last_skip: GlueSpec,
    pub last_node_type: i32,
}

impl Default for EngineStateSnapshot {
    fn default() -> Self {
        Self {
            mode: EngineMode::Vertical,
            is_inner_mode: false,
            space_factor: 1000,
            prev_depth: Scaled::from_raw(0),
            prev_graf: 0,
            par_shape_len: 0,
            last_penalty: 0,
            last_kern: Scaled::from_raw(0),
            last_skip: GlueSpec::ZERO,
            last_node_type: -1,
        }
    }
}

/// Result of one expansion dispatch.
// Keeping dispatch copyable avoids ownership machinery in the expansion loop;
// generation-tagged replay handles make the uncommon Push variant larger.
#[allow(clippy::large_enum_variant)]
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
    ScanGlue(Box<scan_glue::ScanGlueError>),
    ScanGeneralText(Box<scan::ScanToksError>),
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
    ForbiddenOuterTokenInAlignment {
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
            Self::ScanGlue(err) => write!(f, "{err}"),
            Self::ScanGeneralText(err) => write!(f, "{err}"),
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
            Self::ForbiddenOuterTokenInAlignment { .. } => {
                f.write_str("Forbidden control sequence found while scanning an alignment")
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
            Self::ScanGlue(err) => Some(err),
            Self::ScanGeneralText(err) => Some(err),
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
            Self::ForbiddenOuterTokenInAlignment { .. } => None,
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
            Self::ForbiddenOuterTokenInAlignment { context } => Some(context.origin()),
            Self::NonCharacterInInputName { context }
            | Self::UnsupportedTheTarget { context }
            | Self::MissingFontIdentifier { context }
            | Self::MathFamilyOutOfRange { context, .. }
            | Self::FontDimenOutOfRange { context, .. }
            | Self::InvalidConditionalRelation { context }
            | Self::IncompleteIf { context } => Some(context.origin()),
            Self::ScanInt(err) => err.primary_origin(),
            Self::ScanDimen(err) => err.primary_origin(),
            Self::ScanGlue(err) => err.primary_origin(),
            Self::ScanGeneralText(err) => err.primary_origin(),
            Self::MacroCall(err) => err.primary_origin(),
            Self::Lex(err) => err.diagnostic_site().primary_origin(),
        }
    }

    #[must_use]
    pub fn diagnostic_site(&self) -> DiagnosticSite {
        match self {
            Self::Captured { site, .. } => site.clone(),
            Self::Lex(err) => err.diagnostic_site().clone(),
            _ => DiagnosticSite::new(self.primary_origin(), [], None),
        }
    }

    #[cold]
    #[inline(never)]
    fn capture<S: InputSource>(self, input: &InputStack<S>) -> Self {
        if matches!(self, Self::Captured { .. }) {
            return self;
        }
        let site = input.diagnostic_site(self.primary_origin(), []);
        if site.expansion_head().is_none() {
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

    fn par_shape_len(&self) -> i32 {
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

    fn last_node_type(&self) -> i32 {
        -1
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

    fn dispatch_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<Dispatch, ExpandError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>;

    fn dispatch_inverted_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<Dispatch, ExpandError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>;

    fn dispatch_raw_token_after(
        &mut self,
        saved: TracedTokenWord,
        target: TracedTokenWord,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<(), ExpandError>
    where
        S: InputSource,
        R: ReadRecorder,
        H: ExpansionHooks<S>,
    {
        let dispatch = self.dispatch_raw_token(target, input, stores, recorder, hooks)?;
        push_dispatch_result(input, stores, dispatch);
        push_inserted_token(input, stores, saved, InsertedOriginKind::ExpandAfter);
        Ok(())
    }
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

    fn dispatch_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<Dispatch, ExpandError> {
        dispatch_one_raw_token_with_hooks(token, input, stores, recorder, hooks)
    }

    fn dispatch_inverted_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<Dispatch, ExpandError> {
        let Some(symbol) = expandable_symbol(stores, token) else {
            return Ok(Dispatch::Deliver(token));
        };
        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);
        dispatch::dispatch_without_input_open_inverted(
            semantic_token(token),
            token.origin(),
            input,
            stores,
            recorder,
            hooks,
            meaning,
        )
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

    fn dispatch_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<Dispatch, ExpandError> {
        let Some(symbol) = expandable_symbol(stores, token) else {
            return Ok(Dispatch::Deliver(token));
        };
        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);
        dispatch::dispatch_with_hooks(
            semantic_token(token),
            token.origin(),
            input,
            stores,
            recorder,
            hooks,
            meaning,
        )
    }

    fn dispatch_inverted_raw_token(
        &mut self,
        token: TracedTokenWord,
        input: &mut InputStack<S>,
        stores: &mut St,
        recorder: &mut R,
        hooks: &mut H,
    ) -> Result<Dispatch, ExpandError> {
        let Some(symbol) = expandable_symbol(stores, token) else {
            return Ok(Dispatch::Deliver(token));
        };
        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);
        dispatch::dispatch_with_hooks_inverted(
            semantic_token(token),
            token.origin(),
            input,
            stores,
            recorder,
            hooks,
            meaning,
        )
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

impl From<scan_glue::ScanGlueError> for ExpandError {
    fn from(value: scan_glue::ScanGlueError) -> Self {
        Self::ScanGlue(Box::new(value))
    }
}

impl From<scan::ScanToksError> for ExpandError {
    fn from(value: scan::ScanToksError) -> Self {
        Self::ScanGeneralText(Box::new(value))
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
    match get_x_token_with_recorder_and_hooks_inner(input, stores, recorder, hooks, false, None) {
        Ok(token) => Ok(token),
        Err(error) => Err(error.capture(input)),
    }
}

/// Pulls the next expanded token while leaving e-TeX protected macros
/// unexpanded. This is the `get_x_or_protected` operation used by alignments.
pub fn get_x_or_protected_with_recorder_and_hooks<S, R, H>(
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
    match get_x_token_with_recorder_and_hooks_inner(input, stores, recorder, hooks, true, None) {
        Ok(token) => Ok(token),
        Err(error) => Err(error.capture(input)),
    }
}

/// A token whose alignment-sensitive `get_next` work has already completed.
///
/// This is the synchronous, stack-local equivalent of TeX82's `cur_cmd`,
/// `cur_chr`, and `cur_cs` state between `get_next` and `x_token`. It must not
/// be stored in the input stack or any checkpoint-visible engine state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreparedExpansionToken(TracedExpansionToken);

impl PreparedExpansionToken {
    #[must_use]
    pub(crate) const fn traced_token(self) -> TracedTokenWord {
        self.0.traced_token()
    }

    #[must_use]
    pub(crate) const fn suppress_expansion(self) -> bool {
        self.0.suppress_expansion()
    }
}

/// TeX82's `x_token`: expand a token already obtained under `get_next`
/// semantics, while sharing the ordinary `get_x_token` interpreter.
pub(crate) fn get_x_or_protected_from_prepared_with_recorder_and_hooks<S, R, H>(
    prepared: PreparedExpansionToken,
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
    match get_x_token_with_recorder_and_hooks_inner(
        input,
        stores,
        recorder,
        hooks,
        true,
        Some(prepared),
    ) {
        Ok(token) => Ok(token),
        Err(error) => Err(error.capture(input)),
    }
}

fn get_x_token_with_recorder_and_hooks_inner<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
    hooks: &mut H,
    protect_macros: bool,
    first: Option<PreparedExpansionToken>,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut first = first;
    loop {
        let (read, alignment_prepared) = if let Some(prepared) = first.take() {
            (prepared.0, true)
        } else {
            let read = match input.next_traced_expansion_token(stores) {
                Ok(Some(read)) => read,
                Ok(None) => return Ok(None),
                Err(tex_lex::LexError::InvalidCharacter { .. }) => {
                    // TeX.web `get_next` reports a catcode-15 character and
                    // restarts after consuming it. Keeping recovery here prevents
                    // nested scanners (notably \read) from unwinding.
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            (read, false)
        };
        let token = read.token();
        let traced = read.traced_token();

        if token.is_frozen_end_template() {
            return Ok(Some(TracedTokenWord::pack(
                stores.frozen_endv_token(),
                read.origin(),
            )));
        }

        if read.suppress_expansion() {
            if !alignment_prepared && intercept_suppressed_alignment_token(input, stores, traced) {
                continue;
            }
            return Ok(Some(traced));
        }

        let symbol = match expandable_symbol(stores, traced) {
            Some(symbol) => symbol,
            None => {
                if !alignment_prepared && intercept_alignment_token(input, stores, traced) {
                    continue;
                }
                return Ok(Some(traced));
            }
        };

        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);
        if protect_macros
            && matches!(meaning, Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED))
        {
            return Ok(Some(traced));
        }
        if input.has_active_alignment_cell()
            && matches!(meaning, Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::OUTER))
        {
            return Err(ExpandError::ForbiddenOuterTokenInAlignment { context: traced });
        }

        let dispatched = dispatch_with_hooks(
            token,
            read.origin(),
            input,
            stores,
            recorder,
            hooks,
            meaning,
        );
        let dispatched = match dispatched {
            Ok(dispatched) => dispatched,
            Err(ExpandError::MacroCall(args::MacroCallError::DoesNotMatchDefinition {
                ..
            })) => continue,
            Err(ExpandError::MacroCall(args::MacroCallError::EndOfInput { .. })) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        match dispatched {
            Dispatch::Continue => {}
            Dispatch::DeliverNoExpand(token) => {
                if intercept_suppressed_alignment_token(input, stores, token) {
                    continue;
                }
                return Ok(Some(token));
            }
            Dispatch::Deliver(token) => {
                if !alignment_prepared && intercept_alignment_token(input, stores, token) {
                    continue;
                }
                return Ok(Some(token));
            }
            push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
        }
    }
}

/// Reads one token under the semantic `get_next` rules needed before TeX82's
/// `x_token`, retaining one-shot expansion suppression for the shared loop.
pub(crate) fn next_prepared_expansion_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<Option<PreparedExpansionToken>, tex_lex::LexError>
where
    S: InputSource,
{
    loop {
        let Some(read) = input.next_traced_expansion_token(stores)? else {
            return Ok(None);
        };
        let traced = read.traced_token();
        let intercepted = if read.suppress_expansion() {
            intercept_suppressed_alignment_token(input, stores, traced)
        } else {
            intercept_alignment_token(input, stores, traced)
        };
        if !intercepted {
            return Ok(Some(PreparedExpansionToken(read)));
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
    // WEB updates align_state only in the character-token branches of
    // get_next. A control sequence whose meaning was \let to a brace still
    // has that command code for execution, but it does not change the input
    // scanner's brace level merely by being delivered.
    let delivery = if matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    ) {
        tex_lex::AlignmentTokenDelivery::LeftBrace
    } else if matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    ) {
        tex_lex::AlignmentTokenDelivery::RightBrace
    } else {
        tex_lex::AlignmentTokenDelivery::Other
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
    input.intercept_alignment_token(traced, delivery, terminator, stores.execution_group_depth())
}

/// Canonical TeX `get_next`-style raw semantic delivery.
///
/// Expansion primitives and scanners must use this path whenever they consume
/// a raw token. It applies alignment brace accounting and cell-terminator
/// interception before the token can be observed. The lower-level
/// `InputStack` reads remain reserved for the expansion loop and `\noexpand`,
/// which must first classify one-shot suppression.
pub fn next_semantic_raw_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<Option<TracedTokenWord>, tex_lex::LexError>
where
    S: InputSource,
{
    loop {
        let Some(traced) = input.next_traced_token(stores)? else {
            return Ok(None);
        };
        if !intercept_alignment_token(input, stores, traced) {
            return Ok(Some(traced));
        }
    }
}

/// Raw replay lookahead used by `\noexpand`, which must restore the next
/// meaning before applying the alignment-sensitive part of `get_next`.
pub(crate) fn next_unintercepted_raw_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<Option<TracedTokenWord>, tex_lex::LexError>
where
    S: InputSource,
{
    input.next_traced_token(stores)
}

pub(crate) fn next_suppressed_semantic_raw_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<Option<TracedTokenWord>, tex_lex::LexError>
where
    S: InputSource,
{
    loop {
        let Some(traced) = input.next_traced_token(stores)? else {
            return Ok(None);
        };
        if !intercept_suppressed_alignment_token(input, stores, traced) {
            return Ok(Some(traced));
        }
    }
}

/// Applies TeX82's `dont_expand` command-code test to alignment delivery.
///
/// An expandable meaning is delivered as the one-shot `no_expand_flag`
/// variant of `\relax`. An already-unexpandable meaning retains its ordinary
/// brace or delimiter behavior, including `\cr` interception.
fn intercept_suppressed_alignment_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    traced: TracedTokenWord,
) -> bool {
    let expandable = expandable_symbol(stores, traced).is_some_and(|symbol| {
        matches!(
            stores.meaning(symbol),
            Meaning::Undefined | Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_)
        )
    });
    if expandable {
        input.intercept_alignment_token(
            traced,
            tex_lex::AlignmentTokenDelivery::Other,
            None,
            stores.execution_group_depth(),
        )
    } else {
        intercept_alignment_token(input, stores, traced)
    }
}

pub fn back_input<S, I>(input: &mut InputStack<S>, stores: &mut impl ExpansionState, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = TracedTokenWord>,
{
    #[cfg(test)]
    BACK_INPUT_CALLS.with(|calls| calls.set(calls.get() + 1));
    let traced = tokens.into_iter().collect::<Vec<_>>();
    if traced.is_empty() {
        return;
    }
    for &token in &traced {
        input.back_input_alignment_token(token);
    }
    if traced.len() == 1 {
        if let Some((list, _, index)) = input.current_token_list_frame()
            && index > 0
            && stores.tokens(list).get(index - 1).copied() == Some(semantic_token(traced[0]))
            && input.rewind_current_token_list_frame()
        {
            return;
        }
        if input.push_current_source_pending(traced[0]) {
            return;
        }
    }
    let semantic = traced
        .iter()
        .copied()
        .map(semantic_token)
        .collect::<Vec<_>>();
    let token_list = stores.intern_token_list(&semantic);
    let mut origins = stores.origin_list_builder();
    for token in traced {
        origins.push(token.origin());
    }
    let origin_list = stores.finish_origin_list(&mut origins);
    input.push_token_list_with_origins(token_list, origin_list, TokenListReplayKind::Inserted);
}

#[cfg(test)]
thread_local! {
    static BACK_INPUT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_back_input_call_count() {
    BACK_INPUT_CALLS.with(|calls| calls.set(0));
}

#[cfg(test)]
pub(crate) fn back_input_call_count() -> usize {
    BACK_INPUT_CALLS.with(std::cell::Cell::get)
}

/// Inserts traced tokens that were not previously delivered by `get_next`.
///
/// Unlike [`back_input`], this does not reverse alignment brace accounting.
pub fn insert_input<S, I>(input: &mut InputStack<S>, stores: &mut impl ExpansionState, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = TracedTokenWord>,
{
    let traced = tokens.into_iter().collect::<Vec<_>>();
    if traced.is_empty() {
        return;
    }
    let semantic = traced
        .iter()
        .copied()
        .map(semantic_token)
        .collect::<Vec<_>>();
    let token_list = stores.intern_token_list(&semantic);
    let mut origins = stores.origin_list_builder();
    for token in traced {
        origins.push(token.origin());
    }
    let origin_list = stores.finish_origin_list(&mut origins);
    input.push_token_list_with_origins(token_list, origin_list, TokenListReplayKind::Inserted);
}

/// Implements TeX's unexpanded `get_token`, including alignment delimiter
/// interception performed by `get_next` before the token reaches its caller.
pub fn get_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<Option<TracedTokenWord>, ExpandError>
where
    S: InputSource,
{
    Ok(next_semantic_raw_token(input, stores)?)
}

/// Implements TeX82's `get_preamble_token` operation after `\span`: fetch
/// one raw token, expand that token once when it is expandable, then fetch
/// one raw token from the resulting input. Unlike `get_x_token`, this does
/// not recursively expand the token produced by that single expansion.
pub fn expand_once_then_get_token_with_hooks<S, R, H>(
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
    let Some(target) = get_token(input, stores)? else {
        return Ok(None);
    };
    let Some(symbol) = expandable_symbol(stores, target) else {
        return Ok(Some(target));
    };
    let meaning = stores.meaning(symbol);
    recorder.record_meaning(symbol, meaning);
    let dispatch = dispatch_with_hooks(
        semantic_token(target),
        target.origin(),
        input,
        stores,
        recorder,
        hooks,
        meaning,
    )?;
    push_dispatch_result(input, stores, dispatch);
    get_token(input, stores)
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
        let read = match input.next_traced_expansion_token(stores) {
            Ok(Some(read)) => read,
            Ok(None) => return Ok(None),
            Err(tex_lex::LexError::InvalidCharacter { .. }) => continue,
            Err(error) => return Err(error.into()),
        };
        let token = read.token();
        let traced = read.traced_token();

        if read.suppress_expansion() {
            if intercept_suppressed_alignment_token(input, stores, traced) {
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

        let dispatched = dispatch::dispatch_without_input_open(
            token,
            read.origin(),
            input,
            stores,
            recorder,
            hooks,
            meaning,
        );
        let dispatched = match dispatched {
            Ok(dispatched) => dispatched,
            Err(ExpandError::MacroCall(args::MacroCallError::DoesNotMatchDefinition {
                ..
            })) => continue,
            Err(ExpandError::MacroCall(args::MacroCallError::EndOfInput { .. })) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        match dispatched {
            Dispatch::Continue => {}
            Dispatch::DeliverNoExpand(token) => {
                if intercept_suppressed_alignment_token(input, stores, token) {
                    continue;
                }
                return Ok(Some(token));
            }
            Dispatch::Deliver(token) => {
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
