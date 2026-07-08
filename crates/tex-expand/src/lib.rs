//! TeX expansion engine core loop.
//!
//! This crate owns the gullet's single `get_x_token` interpreter loop. It
//! reads meanings through the aggregate state facade and pushes expansion
//! output back through `tex-lex` token-list replay frames.

#![forbid(unsafe_code)]

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError, MacroArguments, TokenListReplayKind};
use tex_state::interner::Symbol;
use tex_state::meaning::Meaning;
use tex_state::stores::Stores;
use tex_state::token::Token;

pub mod args;
pub mod scan;
pub mod scan_dimen;
pub mod scan_int;

mod conditionals;
mod dispatch;
mod primitives;
mod scan_helpers;
#[cfg(test)]
mod tests;
mod values;

pub use dispatch::{dispatch, dispatch_expandable_opcode, dispatch_with_hooks};

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

/// Result of one expansion dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dispatch {
    Continue,
    Deliver(Token),
    DeliverNoExpand(Token),
    Push {
        replay_kind: ExpansionReplayKind,
        token_list: tex_state::ids::TokenListId,
        macro_arguments: MacroArguments,
    },
}

/// Errors raised by `get_x_token`.
#[derive(Debug)]
pub enum ExpandError {
    Lex(LexError),
    MacroCall(args::MacroCallError),
    UnimplementedExpandable(ExpandableOpcode),
    MissingTokenAfterPrimitive(ExpandableOpcode),
    MissingEndCsName,
    NonCharacterInCsName(Token),
    MissingInputName,
    NonCharacterInInputName(Token),
    InputOpen { name: String, message: String },
    ScanInt(Box<scan_int::ScanIntError>),
    ScanDimen(Box<scan_dimen::ScanDimenError>),
    UnsupportedTheTarget(Token),
    InvalidConditionalRelation(Token),
    IncompleteIf,
    ExtraConditionalControl(&'static str),
    ForbiddenOuterTokenInSkippedConditional { name: String },
}

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(err) => write!(f, "{err}"),
            Self::MacroCall(err) => write!(f, "{err}"),
            Self::UnimplementedExpandable(opcode) => {
                write!(f, "expandable opcode {opcode:?} is not implemented yet")
            }
            Self::MissingTokenAfterPrimitive(opcode) => {
                write!(f, "missing token after expandable primitive {opcode:?}")
            }
            Self::MissingEndCsName => write!(f, "missing \\endcsname for \\csname"),
            Self::NonCharacterInCsName(token) => {
                write!(f, "non-character token {token:?} while scanning \\csname")
            }
            Self::MissingInputName => write!(f, "missing file name after \\input"),
            Self::NonCharacterInInputName(token) => {
                write!(
                    f,
                    "non-character token {token:?} while scanning \\input file name"
                )
            }
            Self::InputOpen { name, message } => {
                write!(f, "failed to open input {name:?}: {message}")
            }
            Self::ScanInt(err) => write!(f, "{err}"),
            Self::ScanDimen(err) => write!(f, "{err}"),
            Self::UnsupportedTheTarget(token) => {
                write!(f, "unsupported token {token:?} after \\the")
            }
            Self::InvalidConditionalRelation(token) => {
                write!(f, "invalid conditional relation token {token:?}")
            }
            Self::IncompleteIf => write!(f, "Incomplete \\if; all text was ignored after line"),
            Self::ExtraConditionalControl(name) => write!(f, "Extra \\{name}"),
            Self::ForbiddenOuterTokenInSkippedConditional { name } => {
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
            Self::Lex(err) => Some(err),
            Self::MacroCall(err) => Some(err),
            Self::ScanInt(err) => Some(err),
            Self::ScanDimen(err) => Some(err),
            Self::UnimplementedExpandable(_)
            | Self::MissingTokenAfterPrimitive(_)
            | Self::MissingEndCsName
            | Self::NonCharacterInCsName(_)
            | Self::MissingInputName
            | Self::NonCharacterInInputName(_)
            | Self::InputOpen { .. }
            | Self::UnsupportedTheTarget(_)
            | Self::InvalidConditionalRelation(_)
            | Self::IncompleteIf
            | Self::ExtraConditionalControl(_)
            | Self::ForbiddenOuterTokenInSkippedConditional { .. } => None,
        }
    }
}

/// Driver hooks for expandable primitives that need outside-world state.
///
/// `tex-expand` never opens files itself. A driver or test harness supplies
/// sources through this trait; the eventual `World` implementation is expected
/// to record and snapshot those reads.
pub trait ExpansionHooks<S> {
    fn open_input(&mut self, name: &str) -> Result<S, String>;

    fn job_name(&self) -> &str {
        "texput"
    }

    fn mode(&self) -> EngineMode {
        EngineMode::Vertical
    }

    fn is_inner_mode(&self) -> bool {
        false
    }

    fn input_stream_eof(&self, _stream: u8) -> bool {
        // TODO(umber2-io): replace this documented no-stream-table stub once
        // \openin/\closein state is represented by the driver/World layer.
        true
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopExpansionHooks;

impl<S> ExpansionHooks<S> for NoopExpansionHooks {
    fn open_input(&mut self, _name: &str) -> Result<S, String> {
        Err("no input source hook is installed".to_owned())
    }
}

/// Narrow capability for `\csname`'s sanctioned state mutation.
pub trait CsNameInterner {
    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol;
}

impl CsNameInterner for Stores {
    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        Stores::intern_relaxed_control_sequence(self, name)
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
    stores: &mut Stores,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
{
    get_x_token_with_recorder_and_hooks(input, stores, &mut NoopRecorder, &mut NoopExpansionHooks)
}

/// Pulls the next fully expanded token while recording meaning reads.
pub fn get_x_token_with_recorder<S, R>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
{
    get_x_token_with_recorder_and_hooks(input, stores, recorder, &mut NoopExpansionHooks)
}

/// Pulls the next fully expanded token using driver-provided expansion hooks.
pub fn get_x_token_with_hooks<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    hooks: &mut H,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    get_x_token_with_recorder_and_hooks(input, stores, &mut NoopRecorder, hooks)
}

/// Pulls the next fully expanded token while recording reads and using hooks.
pub fn get_x_token_with_recorder_and_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    loop {
        let Some(read) = input.next_expansion_token(stores)? else {
            return Ok(None);
        };
        let token = read.token();

        if read.suppress_expansion() {
            return Ok(Some(token));
        }

        let Token::Cs(symbol) = token else {
            return Ok(Some(token));
        };

        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);

        match dispatch_with_hooks(token, input, stores, recorder, hooks, meaning)? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => return Ok(Some(token)),
            push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
        }
    }
}

pub(crate) fn dispatch_one_raw_token_with_hooks<S, R, H>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Token::Cs(symbol) = token else {
        return Ok(Dispatch::Deliver(token));
    };

    let meaning = stores.meaning(symbol);
    recorder.record_meaning(symbol, meaning);
    dispatch_with_hooks(token, input, stores, recorder, hooks, meaning)
}

pub(crate) fn push_dispatch_result<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    dispatch: Dispatch,
) {
    match dispatch {
        Dispatch::Deliver(token) => push_inserted_token(input, stores, token),
        Dispatch::DeliverNoExpand(token) => push_noexpand_token(input, stores, token),
        Dispatch::Continue => {}
        push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
    }
}

pub(crate) fn apply_dispatch_push<S>(input: &mut InputStack<S>, dispatch: Dispatch) {
    let Dispatch::Push {
        replay_kind,
        token_list,
        macro_arguments,
    } = dispatch
    else {
        return;
    };

    if replay_kind == ExpansionReplayKind::MacroBody {
        input.push_macro_body(token_list, macro_arguments);
    } else {
        input.push_token_list(token_list, replay_kind.as_lex_kind());
    }
}

pub(crate) fn push_inserted_token<S>(input: &mut InputStack<S>, stores: &mut Stores, token: Token) {
    let token_list = stores.intern_token_list(&[token]);
    input.push_token_list(token_list, TokenListReplayKind::Inserted);
}

pub(crate) fn push_noexpand_token<S>(input: &mut InputStack<S>, stores: &mut Stores, token: Token) {
    let token_list = stores.intern_token_list(&[token]);
    input.push_token_list(token_list, TokenListReplayKind::NoExpand);
}
