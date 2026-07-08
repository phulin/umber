//! TeX expansion engine core loop.
//!
//! This crate owns the gullet's single `get_x_token` interpreter loop. It
//! reads meanings through the aggregate state facade and pushes expansion
//! output back through `tex-lex` token-list replay frames.

#![forbid(unsafe_code)]

use std::fmt;

use tex_lex::{
    ConditionFrameSummary, ConditionKind, ConditionLimb, InputSource, InputStack, LexError,
    MacroArguments, TokenListReplayKind,
};
use tex_state::env::banks::IntParam;
use tex_state::ids::TokenListId;
use tex_state::interner::Symbol;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::scaled::Scaled;
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

pub mod args;
pub mod scan;
pub mod scan_dimen;
pub mod scan_int;

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
    IncompleteIf,
    ExtraConditionalControl(&'static str),
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
            Self::IncompleteIf => write!(f, "Incomplete \\if; all text was ignored after line"),
            Self::ExtraConditionalControl(name) => write!(f, "Extra \\{name}"),
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
            | Self::IncompleteIf
            | Self::ExtraConditionalControl(_) => None,
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
        let Some(read) = input.next_expansion_token_readonly(stores)? else {
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

/// Dispatches one token/meaning pair.
///
/// TODO(umber2-5qt.3): implement expandable primitive arms.
/// TODO(umber2-5qt.5): implement conditional scan/evaluation arms.
pub fn dispatch<S, R>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
{
    dispatch_with_hooks(
        token,
        input,
        stores,
        recorder,
        &mut NoopExpansionHooks,
        meaning,
    )
}

pub fn dispatch_with_hooks<S, R, H>(
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    meaning: Meaning,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    match meaning {
        Meaning::Macro { flags, definition } if is_expandable_macro(flags) => {
            let macro_meaning = stores.macro_definition(definition);
            let arguments = args::match_macro_call_with_recorder(
                input,
                stores,
                recorder,
                token,
                macro_meaning,
            )?;
            Ok(Dispatch::Push {
                replay_kind: ExpansionReplayKind::MacroBody,
                token_list: macro_meaning.replacement_text(),
                macro_arguments: arguments.as_macro_arguments(),
            })
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
            expand_after(input, stores, recorder, hooks)?;
            Ok(Dispatch::Continue)
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) => {
            let Some(token) = input.next_token_readonly(stores)? else {
                return Err(ExpandError::MissingTokenAfterPrimitive(
                    ExpandableOpcode::NoExpand,
                ));
            };
            Ok(Dispatch::DeliverNoExpand(token))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName) => {
            let name = scan_csname(input, stores, recorder, hooks)?;
            let symbol = CsNameInterner::intern_relaxed_control_sequence(stores, &name);
            Ok(Dispatch::Deliver(Token::Cs(symbol)))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) => {
            Ok(Dispatch::Deliver(token))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::String) => {
            let Some(target) = input.next_token_readonly(stores)? else {
                return Err(ExpandError::MissingTokenAfterPrimitive(
                    ExpandableOpcode::String,
                ));
            };
            Ok(push_rendered_tokens(
                stores,
                ExpansionReplayKind::NumberOutput,
                string_tokens(stores, target),
            ))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Number) => {
            let scanned = scan_int::scan_int(input, stores)?;
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::NumberOutput,
                &scanned.value().to_string(),
            ))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::RomanNumeral) => {
            let scanned = scan_int::scan_int(input, stores)?;
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::NumberOutput,
                &roman_numeral(scanned.value()),
            ))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning) => {
            let Some(target) = input.next_token_readonly(stores)? else {
                return Err(ExpandError::MissingTokenAfterPrimitive(
                    ExpandableOpcode::Meaning,
                ));
            };
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::NumberOutput,
                &meaning_text(stores, target),
            ))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::The) => expand_the(input, stores),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Input) => {
            let name = scan_input_name(input, stores, recorder, hooks)?;
            let source = hooks
                .open_input(&name)
                .map_err(|message| ExpandError::InputOpen {
                    name: name.clone(),
                    message,
                })?;
            input.push_source(source);
            Ok(Dispatch::Continue)
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndInput) => {
            input.end_current_source_after_current_line();
            Ok(Dispatch::Continue)
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::JobName) => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::JobName,
            hooks.job_name(),
        )),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName) => {
            // TODO(umber2-fonts): consume/resolve a font selector once font
            // meanings exist. Until then this documented stub expands empty.
            let _ = next_non_space_x_token(input, stores)?;
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::NumberOutput,
                "",
            ))
        }
        Meaning::ExpandablePrimitive(
            ExpandablePrimitive::TopMark
            | ExpandablePrimitive::FirstMark
            | ExpandablePrimitive::BotMark
            | ExpandablePrimitive::SplitFirstMark
            | ExpandablePrimitive::SplitBotMark,
        ) => {
            // TODO(umber2-page): return the page builder's stored mark token
            // lists once mark nodes and page splitting exist.
            Ok(push_rendered_text(stores, ExpansionReplayKind::Mark, ""))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::IfTrue) => {
            begin_if(input, stores, recorder, hooks, true)
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::IfFalse) => {
            begin_if(input, stores, recorder, hooks, false)
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::If) => {
            let left = scan_condition_x_token(input, stores, recorder, hooks)?;
            let right = scan_condition_x_token(input, stores, recorder, hooks)?;
            begin_if(input, stores, recorder, hooks, if_char_equal(left, right))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCat) => {
            let left = scan_condition_x_token(input, stores, recorder, hooks)?;
            let right = scan_condition_x_token(input, stores, recorder, hooks)?;
            begin_if(input, stores, recorder, hooks, if_cat_equal(left, right))
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::IfX) => {
            let Some(left) = input.next_token_readonly(stores)? else {
                return Err(ExpandError::MissingTokenAfterPrimitive(
                    ExpandableOpcode::If,
                ));
            };
            let Some(right) = input.next_token_readonly(stores)? else {
                return Err(ExpandError::MissingTokenAfterPrimitive(
                    ExpandableOpcode::If,
                ));
            };
            begin_if(
                input,
                stores,
                recorder,
                hooks,
                ifx_equal(stores, left, right),
            )
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Else) => {
            handle_else(input, stores, recorder, hooks)
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi) => {
            input
                .pop_condition()
                .ok_or(ExpandError::ExtraConditionalControl("fi"))?;
            Ok(Dispatch::Continue)
        }
        Meaning::Macro { .. }
        | Meaning::Undefined
        | Meaning::Relax
        | Meaning::CharGiven(_)
        | Meaning::Unknown(_) => Ok(Dispatch::Deliver(token)),
    }
}

const fn is_expandable_macro(flags: MeaningFlags) -> bool {
    !flags.contains(MeaningFlags::PROTECTED)
}

/// Skeleton dispatch table for all expandable opcode families in this epic.
pub fn dispatch_expandable_opcode(opcode: ExpandableOpcode) -> Result<(), ExpandError> {
    match opcode {
        ExpandableOpcode::Macro => Ok(()),
        ExpandableOpcode::ExpandAfter
        | ExpandableOpcode::NoExpand
        | ExpandableOpcode::CsName
        | ExpandableOpcode::EndCsName
        | ExpandableOpcode::String
        | ExpandableOpcode::Number
        | ExpandableOpcode::RomanNumeral
        | ExpandableOpcode::Meaning
        | ExpandableOpcode::The
        | ExpandableOpcode::Input
        | ExpandableOpcode::EndInput
        | ExpandableOpcode::JobName
        | ExpandableOpcode::FontName
        | ExpandableOpcode::Mark
        | ExpandableOpcode::If
        | ExpandableOpcode::Else
        | ExpandableOpcode::Fi => Ok(()),
        ExpandableOpcode::Or => Err(unimplemented_expandable(opcode)),
    }
}

fn unimplemented_expandable(opcode: ExpandableOpcode) -> ExpandError {
    ExpandError::UnimplementedExpandable(opcode)
}

fn expand_after<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(saved) = input.next_token_readonly(stores)? else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::ExpandAfter,
        ));
    };
    let Some(target) = input.next_token_readonly(stores)? else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::ExpandAfter,
        ));
    };

    let target_dispatch =
        dispatch_one_raw_token_with_hooks(target, input, stores, recorder, hooks)?;
    push_dispatch_result(input, stores, target_dispatch);
    push_inserted_token(input, stores, saved);
    Ok(())
}

fn begin_if<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
    condition: bool,
) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    input.push_condition(ConditionFrameSummary::new_if(condition));
    if !condition {
        skip_false_limb(input, stores, recorder, hooks)?;
    }
    Ok(Dispatch::Continue)
}

fn handle_else<S, R, H>(
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
    let frame = input
        .pop_condition()
        .ok_or(ExpandError::ExtraConditionalControl("else"))?;
    if frame.kind() != ConditionKind::If || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl("else"));
    }

    let else_frame = frame.with_else_limb(!frame.any_limb_taken());
    input.push_condition(else_frame);
    if frame.any_limb_taken() {
        skip_to_fi(input, stores, recorder, hooks)?;
    }
    Ok(Dispatch::Continue)
}

fn skip_false_limb<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    skip_until(input, stores, recorder, hooks, SkipTarget::ElseOrFi)
}

fn skip_to_fi<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    skip_until(input, stores, recorder, hooks, SkipTarget::Fi)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SkipTarget {
    ElseOrFi,
    Fi,
}

fn skip_until<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    _hooks: &mut H,
    target: SkipTarget,
) -> Result<(), ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut nesting = 0_u32;
    loop {
        let Some(token) = input.next_token_readonly(stores)? else {
            return Err(ExpandError::IncompleteIf);
        };
        let Some(primitive) = conditional_primitive(stores, token, recorder) else {
            continue;
        };

        match primitive {
            ConditionalPrimitive::If => {
                nesting = nesting.saturating_add(1);
            }
            ConditionalPrimitive::Else if nesting == 0 && target == SkipTarget::ElseOrFi => {
                move_current_if_to_else(input)?;
                return Ok(());
            }
            ConditionalPrimitive::Fi if nesting == 0 => {
                input
                    .pop_condition()
                    .ok_or(ExpandError::ExtraConditionalControl("fi"))?;
                return Ok(());
            }
            ConditionalPrimitive::Fi => {
                nesting = nesting.saturating_sub(1);
            }
            ConditionalPrimitive::Else => {}
        }
    }
}

fn move_current_if_to_else<S>(input: &mut InputStack<S>) -> Result<(), ExpandError> {
    let frame = input
        .pop_condition()
        .ok_or(ExpandError::ExtraConditionalControl("else"))?;
    if frame.kind() != ConditionKind::If || frame.limb() == ConditionLimb::Else {
        return Err(ExpandError::ExtraConditionalControl("else"));
    }
    input.push_condition(frame.with_else_limb(!frame.any_limb_taken()));
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConditionalPrimitive {
    If,
    Else,
    Fi,
}

fn conditional_primitive<R>(
    stores: &Stores,
    token: Token,
    recorder: &mut R,
) -> Option<ConditionalPrimitive>
where
    R: ReadRecorder,
{
    let Token::Cs(symbol) = token else {
        return None;
    };
    let meaning = stores.meaning(symbol);
    recorder.record_meaning(symbol, meaning);
    match meaning {
        Meaning::ExpandablePrimitive(
            ExpandablePrimitive::IfTrue
            | ExpandablePrimitive::IfFalse
            | ExpandablePrimitive::If
            | ExpandablePrimitive::IfCat
            | ExpandablePrimitive::IfX,
        ) => Some(ConditionalPrimitive::If),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Else) => Some(ConditionalPrimitive::Else),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi) => Some(ConditionalPrimitive::Fi),
        _ => None,
    }
}

fn scan_condition_x_token<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<Token, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?.ok_or(
        ExpandError::MissingTokenAfterPrimitive(ExpandableOpcode::If),
    )
}

fn if_char_equal(left: Token, right: Token) -> bool {
    match (left, right) {
        (Token::Char { ch: left, .. }, Token::Char { ch: right, .. }) => left == right,
        _ => false,
    }
}

fn if_cat_equal(left: Token, right: Token) -> bool {
    match (left, right) {
        (Token::Char { cat: left, .. }, Token::Char { cat: right, .. }) => left == right,
        (Token::Cs(_), Token::Cs(_)) => true,
        (Token::Param(_), Token::Param(_)) => true,
        _ => false,
    }
}

fn ifx_equal(stores: &Stores, left: Token, right: Token) -> bool {
    match (left, right) {
        (Token::Char { .. } | Token::Param(_), Token::Char { .. } | Token::Param(_)) => {
            left == right
        }
        (Token::Cs(left), Token::Cs(right)) => meaning_words_ifx_equal(stores, left, right),
        _ => false,
    }
}

fn meaning_words_ifx_equal(stores: &Stores, left: Symbol, right: Symbol) -> bool {
    let left = stores.meaning(left);
    let right = stores.meaning(right);
    match (left, right) {
        (
            Meaning::Macro {
                flags: left_flags,
                definition: left_definition,
            },
            Meaning::Macro {
                flags: right_flags,
                definition: right_definition,
            },
        ) => left_flags == right_flags && left_definition == right_definition,
        (Meaning::Macro { .. }, _) | (_, Meaning::Macro { .. }) => false,
        _ => left == right,
    }
}

fn scan_csname<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<String, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mut name = String::new();

    loop {
        let Some(read) = input.next_expansion_token_readonly(stores)? else {
            return Err(ExpandError::MissingEndCsName);
        };
        let token = read.token();

        if read.suppress_expansion() {
            append_csname_token(&mut name, token)?;
            continue;
        }

        let Token::Cs(symbol) = token else {
            append_csname_token(&mut name, token)?;
            continue;
        };

        let meaning = stores.meaning(symbol);
        recorder.record_meaning(symbol, meaning);

        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) {
            return Ok(name);
        }

        match dispatch_with_hooks(token, input, stores, recorder, hooks, meaning)? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => {
                append_csname_token(&mut name, token)?;
            }
            push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
        }
    }
}

fn append_csname_token(name: &mut String, token: Token) -> Result<(), ExpandError> {
    match token {
        Token::Char { ch, .. } => {
            name.push(ch);
            Ok(())
        }
        Token::Cs(_) | Token::Param(_) => Err(ExpandError::NonCharacterInCsName(token)),
    }
}

fn scan_input_name<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<String, ExpandError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let Some(first) = next_non_space_x_token_with_hooks(input, stores, recorder, hooks)? else {
        return Err(ExpandError::MissingInputName);
    };

    if is_begin_group(first) {
        let mut name = String::new();
        loop {
            let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
            else {
                return Err(ExpandError::MissingInputName);
            };
            if is_end_group(token) {
                return if name.is_empty() {
                    Err(ExpandError::MissingInputName)
                } else {
                    Ok(name)
                };
            }
            append_input_name_token(&mut name, token)?;
        }
    }

    let mut name = String::new();
    append_input_name_token(&mut name, first)?;
    loop {
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            break;
        };
        if matches!(
            token,
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            break;
        }
        append_input_name_token(&mut name, token)?;
    }

    if name.is_empty() {
        Err(ExpandError::MissingInputName)
    } else {
        Ok(name)
    }
}

fn append_input_name_token(name: &mut String, token: Token) -> Result<(), ExpandError> {
    match token {
        Token::Char { ch, .. } => {
            name.push(ch);
            Ok(())
        }
        Token::Cs(_) | Token::Param(_) => Err(ExpandError::NonCharacterInInputName(token)),
    }
}

fn is_begin_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    )
}

fn is_end_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    )
}

fn expand_the<S>(input: &mut InputStack<S>, stores: &mut Stores) -> Result<Dispatch, ExpandError>
where
    S: InputSource,
{
    let Some(token) = next_non_space_x_token(input, stores)? else {
        return Err(ExpandError::MissingTokenAfterPrimitive(
            ExpandableOpcode::The,
        ));
    };
    let Token::Cs(symbol) = token else {
        return Err(ExpandError::UnsupportedTheTarget(token));
    };

    match stores.resolve(symbol) {
        "count" => {
            let index = scan_register_index(input, stores)?;
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &stores.count(index).to_string(),
            ))
        }
        "dimen" => {
            let index = scan_register_index(input, stores)?;
            Ok(push_rendered_text(
                stores,
                ExpansionReplayKind::TheOutput,
                &format_scaled(stores.dimen(index)),
            ))
        }
        "toks" => {
            let index = scan_register_index(input, stores)?;
            Ok(Dispatch::Push {
                replay_kind: ExpansionReplayKind::TheOutput,
                token_list: stores.toks(index),
                macro_arguments: MacroArguments::new(),
            })
        }
        "endlinechar" => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.int_param(IntParam::END_LINE_CHAR).to_string(),
        )),
        "escapechar" => Ok(push_rendered_text(
            stores,
            ExpansionReplayKind::TheOutput,
            &stores.int_param(IntParam::ESCAPE_CHAR).to_string(),
        )),
        // TODO(umber2-5qt): support `\the` for glue, muglue, font dimensions,
        // code tables, box dimensions, page state, and time/job parameters as
        // those Env classes become semantically available to the gullet.
        _ => Err(ExpandError::UnsupportedTheTarget(token)),
    }
}

fn next_non_space_x_token<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<Option<Token>, ExpandError>
where
    S: InputSource,
{
    loop {
        let Some(token) = get_x_token(input, stores)? else {
            return Ok(None);
        };
        if !matches!(
            token,
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            return Ok(Some(token));
        }
    }
}

fn next_non_space_x_token_with_hooks<S, R, H>(
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
        let Some(token) = get_x_token_with_recorder_and_hooks(input, stores, recorder, hooks)?
        else {
            return Ok(None);
        };
        if !matches!(
            token,
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            return Ok(Some(token));
        }
    }
}

fn scan_register_index<S>(
    input: &mut InputStack<S>,
    stores: &mut Stores,
) -> Result<u16, ExpandError>
where
    S: InputSource,
{
    let value = scan_int::scan_int(input, stores)?.value();
    if !(0..=32_767).contains(&value) {
        return Err(scan_int::ScanIntError::RegisterNumberOutOfRange(value).into());
    }
    Ok(value as u16)
}

fn push_rendered_text(
    stores: &mut Stores,
    replay_kind: ExpansionReplayKind,
    text: &str,
) -> Dispatch {
    push_rendered_tokens(stores, replay_kind, text_tokens(text))
}

fn push_rendered_tokens<I>(
    stores: &mut Stores,
    replay_kind: ExpansionReplayKind,
    tokens: I,
) -> Dispatch
where
    I: IntoIterator<Item = Token>,
{
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let token_list = freeze_output_tokens(stores, &tokens);
    Dispatch::Push {
        replay_kind,
        token_list,
        macro_arguments: MacroArguments::new(),
    }
}

fn freeze_output_tokens(stores: &mut Stores, tokens: &[Token]) -> TokenListId {
    stores.intern_token_list(tokens)
}

fn string_tokens(stores: &Stores, token: Token) -> Vec<Token> {
    match token {
        Token::Char { ch, .. } => vec![rendered_char(ch)],
        Token::Cs(symbol) => {
            let mut out = Vec::new();
            if let Some(escape) = escapechar(stores) {
                out.push(rendered_char(escape));
            }
            out.extend(stores.resolve(symbol).chars().map(rendered_char));
            out
        }
        Token::Param(slot) => text_tokens(&format!("#{slot}")),
    }
}

fn meaning_text(stores: &Stores, token: Token) -> String {
    match token {
        Token::Char {
            ch,
            cat: Catcode::Letter,
        } => format!("the letter {ch}"),
        Token::Char { ch, .. } => format!("the character {ch}"),
        Token::Param(slot) => format!("macro parameter character #{slot}"),
        Token::Cs(symbol) => match stores.meaning(symbol) {
            Meaning::Undefined => "undefined".to_owned(),
            Meaning::Relax => "\\relax".to_owned(),
            Meaning::CharGiven(ch) => format!("the character {ch}"),
            Meaning::ExpandablePrimitive(_) => format!("\\{}", stores.resolve(symbol)),
            Meaning::Macro { flags, definition } => {
                let macro_meaning = stores.macro_definition(definition);
                let mut text = String::new();
                if flags.contains(MeaningFlags::PROTECTED) {
                    text.push_str("protected");
                }
                text.push_str("macro:");
                text.push_str(&token_list_text(stores, macro_meaning.parameter_text()));
                text.push_str("->");
                text.push_str(&token_list_text(stores, macro_meaning.replacement_text()));
                text
            }
            Meaning::Unknown(_) => "unknown".to_owned(),
        },
    }
}

fn token_list_text(stores: &Stores, token_list: TokenListId) -> String {
    stores
        .tokens(token_list)
        .iter()
        .flat_map(|&token| string_tokens(stores, token))
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(ch),
            Token::Cs(_) | Token::Param(_) => None,
        })
        .collect()
}

fn text_tokens(text: &str) -> Vec<Token> {
    text.chars().map(rendered_char).collect()
}

fn rendered_char(ch: char) -> Token {
    Token::Char {
        ch,
        cat: if ch == ' ' {
            Catcode::Space
        } else {
            Catcode::Other
        },
    }
}

fn escapechar(stores: &Stores) -> Option<char> {
    u32::try_from(stores.int_param(IntParam::ESCAPE_CHAR))
        .ok()
        .filter(|&value| value < 256)
        .and_then(char::from_u32)
}

fn roman_numeral(value: i32) -> String {
    if value <= 0 {
        return String::new();
    }
    let mut value = value;
    let mut out = String::new();
    for (amount, text) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while value >= amount {
            out.push_str(text);
            value -= amount;
        }
    }
    out
}

fn format_scaled(value: Scaled) -> String {
    let raw = value.raw();
    let negative = raw < 0;
    let magnitude = if negative {
        i64::from(raw).wrapping_neg()
    } else {
        i64::from(raw)
    };
    let unity = i64::from(Scaled::UNITY);
    let mut integer = magnitude / unity;
    let fraction = magnitude % unity;
    let mut decimal = ((fraction * 100_000) + (unity / 2)) / unity;
    if decimal == 100_000 {
        integer += 1;
        decimal = 0;
    }
    let mut fraction_text = format!("{decimal:05}");
    while fraction_text.len() > 1 && fraction_text.ends_with('0') {
        fraction_text.pop();
    }
    let sign = if negative { "-" } else { "" };
    format!("{sign}{integer}.{fraction_text}pt")
}

fn dispatch_one_raw_token_with_hooks<S, R, H>(
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

fn push_dispatch_result<S>(input: &mut InputStack<S>, stores: &mut Stores, dispatch: Dispatch) {
    match dispatch {
        Dispatch::Deliver(token) => push_inserted_token(input, stores, token),
        Dispatch::DeliverNoExpand(token) => push_noexpand_token(input, stores, token),
        Dispatch::Continue => {}
        push @ Dispatch::Push { .. } => apply_dispatch_push(input, push),
    }
}

fn apply_dispatch_push<S>(input: &mut InputStack<S>, dispatch: Dispatch) {
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

fn push_inserted_token<S>(input: &mut InputStack<S>, stores: &mut Stores, token: Token) {
    let token_list = stores.intern_token_list(&[token]);
    input.push_token_list(token_list, TokenListReplayKind::Inserted);
}

fn push_noexpand_token<S>(input: &mut InputStack<S>, stores: &mut Stores, token: Token) {
    let token_list = stores.intern_token_list(&[token]);
    input.push_token_list(token_list, TokenListReplayKind::NoExpand);
}

#[cfg(test)]
mod tests {
    use super::{
        ExpandableOpcode, ExpansionHooks, NoopRecorder, ReadRecorder, dispatch,
        dispatch_expandable_opcode, get_x_token, get_x_token_with_hooks, get_x_token_with_recorder,
    };
    use std::collections::HashMap;
    use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
    use tex_state::interner::Symbol;
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
    use tex_state::stores::Stores;
    use tex_state::token::{Catcode, Token};

    #[derive(Default)]
    struct CountingRecorder {
        reads: usize,
    }

    impl ReadRecorder for CountingRecorder {
        fn record_meaning(&mut self, _symbol: Symbol, _meaning: Meaning) {
            self.reads += 1;
        }
    }

    #[test]
    fn noop_recorder_has_no_state() {
        assert_eq!(core::mem::size_of::<NoopRecorder>(), 0);
    }

    #[test]
    fn dispatch_delivers_unexpandable_tokens() {
        let mut stores = Stores::new();
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        assert_eq!(
            dispatch(
                token,
                &mut InputStack::new(MemoryInput::new("")),
                &mut stores,
                &mut NoopRecorder,
                Meaning::Relax,
            )
            .expect("dispatch should succeed"),
            super::Dispatch::Deliver(token)
        );
    }

    #[test]
    fn expandable_dispatch_table_covers_epic_opcode_families() {
        let opcodes = [
            ExpandableOpcode::Macro,
            ExpandableOpcode::ExpandAfter,
            ExpandableOpcode::NoExpand,
            ExpandableOpcode::CsName,
            ExpandableOpcode::EndCsName,
            ExpandableOpcode::String,
            ExpandableOpcode::Number,
            ExpandableOpcode::RomanNumeral,
            ExpandableOpcode::Meaning,
            ExpandableOpcode::The,
            ExpandableOpcode::Input,
            ExpandableOpcode::If,
            ExpandableOpcode::Else,
            ExpandableOpcode::Or,
            ExpandableOpcode::Fi,
        ];

        for opcode in opcodes {
            let result = dispatch_expandable_opcode(opcode);
            match opcode {
                ExpandableOpcode::Macro
                | ExpandableOpcode::ExpandAfter
                | ExpandableOpcode::NoExpand
                | ExpandableOpcode::CsName
                | ExpandableOpcode::EndCsName
                | ExpandableOpcode::String
                | ExpandableOpcode::Number
                | ExpandableOpcode::RomanNumeral
                | ExpandableOpcode::Meaning
                | ExpandableOpcode::The
                | ExpandableOpcode::Input
                | ExpandableOpcode::EndInput
                | ExpandableOpcode::JobName
                | ExpandableOpcode::FontName
                | ExpandableOpcode::Mark
                | ExpandableOpcode::If
                | ExpandableOpcode::Else
                | ExpandableOpcode::Fi => assert!(result.is_ok()),
                _ => assert!(matches!(
                    result,
                    Err(super::ExpandError::UnimplementedExpandable(found)) if found == opcode
                )),
            }
        }
    }

    #[test]
    fn get_x_token_delivers_unexpandable_control_sequence() {
        let mut stores = Stores::new();
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let mut input = InputStack::new(MemoryInput::new(""));
        let list = stores.intern_token_list(&[Token::Cs(relax)]);
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Cs(relax))
        );
    }

    #[test]
    fn get_x_token_pulls_from_source_frames_readonly() {
        let mut stores = Stores::new();
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let mut input = InputStack::new(MemoryInput::new("x\\relax"));

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("source expansion should succeed"),
            Some(Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            })
        );
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("source expansion should succeed"),
            Some(Token::Cs(relax))
        );
    }

    #[test]
    fn get_x_token_pushes_macro_body_frame_and_continues() {
        let mut stores = Stores::new();
        let macro_cs = stores.intern("m");
        let body = stores.intern_token_list(&[
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            },
        ]);
        let params = stores.intern_token_list(&[]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let invocation = stores.intern_token_list(&[Token::Cs(macro_cs)]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(invocation, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            })
        );
        assert!(matches!(
            input.summary().frames().last(),
            Some(tex_lex::InputFrameSummary::TokenList {
                token_list,
                replay_kind: TokenListReplayKind::MacroBody,
                index: 1,
                macro_arguments
            }) if *token_list == body && macro_arguments.is_empty()
        ));
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            })
        );
    }

    #[test]
    fn recorder_observes_one_meaning_read_per_control_sequence_token() {
        let mut stores = Stores::new();
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let list = stores.intern_token_list(&[Token::Cs(relax)]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);
        let mut recorder = CountingRecorder::default();

        assert_eq!(
            get_x_token_with_recorder(&mut input, &mut stores, &mut recorder)
                .expect("expansion should succeed"),
            Some(Token::Cs(relax))
        );
        assert_eq!(recorder.reads, 1);
    }

    #[test]
    fn expandafter_expands_second_token_then_replays_saved_token_first() {
        let mut stores = Stores::new();
        let expandafter =
            expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
        let macro_cs = stores.intern("m");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('x'), char_token('y')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );

        let input_list = stores.intern_token_list(&[
            Token::Cs(expandafter),
            char_token('a'),
            Token::Cs(macro_cs),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "axy");
    }

    #[test]
    fn expandafter_chains_match_tex_pushback_order() {
        let mut stores = Stores::new();
        let expandafter =
            expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
        let first = stores.intern("first");
        let second = stores.intern("second");
        let params = stores.intern_token_list(&[]);
        let first_body = stores.intern_token_list(&[char_token('1')]);
        let second_body = stores.intern_token_list(&[char_token('2')]);
        stores.set_macro_meaning(
            first,
            MacroMeaning::new(MeaningFlags::EMPTY, params, first_body),
        );
        stores.set_macro_meaning(
            second,
            MacroMeaning::new(MeaningFlags::EMPTY, params, second_body),
        );

        let input_list = stores.intern_token_list(&[
            Token::Cs(expandafter),
            Token::Cs(expandafter),
            Token::Cs(expandafter),
            Token::Cs(first),
            Token::Cs(second),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "12");
    }

    #[test]
    fn noexpand_suppresses_next_control_sequence_for_one_get_x_token() {
        let mut stores = Stores::new();
        let noexpand = expandable_primitive(&mut stores, "noexpand", ExpandablePrimitive::NoExpand);
        let macro_cs = stores.intern("m");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('x')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let input_list = stores.intern_token_list(&[
            Token::Cs(noexpand),
            Token::Cs(macro_cs),
            Token::Cs(macro_cs),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Cs(macro_cs))
        );
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(char_token('x'))
        );
    }

    #[test]
    fn expandafter_preserves_noexpand_for_later_frame_step() {
        let mut stores = Stores::new();
        let expandafter =
            expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
        let noexpand = expandable_primitive(&mut stores, "noexpand", ExpandablePrimitive::NoExpand);
        let macro_cs = stores.intern("m");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('x')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let input_list = stores.intern_token_list(&[
            Token::Cs(expandafter),
            char_token('a'),
            Token::Cs(noexpand),
            Token::Cs(macro_cs),
            Token::Cs(macro_cs),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(char_token('a'))
        );
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(Token::Cs(macro_cs))
        );
        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
            Some(char_token('x'))
        );
    }

    #[test]
    fn csname_interns_undefined_name_and_assigns_relax() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let input_list = stores.intern_token_list(&[
            Token::Cs(csname),
            char_token('f'),
            char_token('o'),
            char_token('o'),
            Token::Cs(endcsname),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        let created = stores.symbol("foo");
        assert!(created.is_none());
        let token = get_x_token(&mut input, &mut stores)
            .expect("csname expansion should succeed")
            .expect("csname should emit a token");
        let Token::Cs(created) = token else {
            panic!("expected control sequence, got {token:?}");
        };

        assert_eq!(stores.resolve(created), "foo");
        assert_eq!(stores.meaning(created), Meaning::Relax);
    }

    #[test]
    fn csname_expands_name_pieces_before_interning() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let macro_cs = stores.intern("piece");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('b'), char_token('a'), char_token('r')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let input_list = stores.intern_token_list(&[
            Token::Cs(csname),
            char_token('f'),
            Token::Cs(macro_cs),
            Token::Cs(endcsname),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("csname expansion should succeed"),
            Some(Token::Cs(
                stores
                    .symbol("fbar")
                    .expect("expanded name should be interned")
            ))
        );
    }

    #[test]
    fn csname_reports_non_character_material_after_expansion() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let input_list =
            stores.intern_token_list(&[Token::Cs(csname), Token::Cs(relax), Token::Cs(endcsname)]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert!(matches!(
            get_x_token(&mut input, &mut stores),
            Err(super::ExpandError::NonCharacterInCsName(Token::Cs(found))) if found == relax
        ));
    }

    #[test]
    fn csname_preserves_existing_meaning_for_ifx_relax_comparison() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let existing = stores.intern("known");
        stores.set_meaning(existing, Meaning::CharGiven('K'));
        let input_list = stores.intern_token_list(&[
            Token::Cs(csname),
            char_token('k'),
            char_token('n'),
            char_token('o'),
            char_token('w'),
            char_token('n'),
            Token::Cs(endcsname),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("csname expansion should succeed"),
            Some(Token::Cs(existing))
        );
        assert_eq!(stores.meaning(existing), Meaning::CharGiven('K'));
    }

    #[test]
    fn csname_created_undefined_name_is_meaning_equal_to_relax() {
        let mut stores = Stores::new();
        let (csname, endcsname) = csname_primitives(&mut stores);
        let relax = stores.intern("relax");
        stores.set_meaning(relax, Meaning::Relax);
        let input_list = stores.intern_token_list(&[
            Token::Cs(csname),
            char_token('n'),
            char_token('e'),
            char_token('w'),
            Token::Cs(endcsname),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(input_list, TokenListReplayKind::Inserted);

        let Some(Token::Cs(created)) =
            get_x_token(&mut input, &mut stores).expect("csname expansion should succeed")
        else {
            panic!("expected created control sequence");
        };

        assert_eq!(stores.meaning(created), stores.meaning(relax));
    }

    #[test]
    fn macro_body_replay_substitutes_frozen_argument_lists() {
        let mut stores = Stores::new();
        let macro_cs = stores.intern("m");
        let params = stores.intern_token_list(&[Token::param(1)]);
        let body = stores.intern_token_list(&[char_token('a'), Token::param(1), char_token('b')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let invocation = stores.intern_token_list(&[
            Token::Cs(macro_cs),
            char_token('{'),
            char_token('x'),
            char_token('y'),
            char_token('}'),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(invocation, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "axyb");
    }

    #[test]
    fn nested_macro_calls_replay_arguments_from_outer_frozen_frame() {
        let mut stores = Stores::new();
        let wrap = stores.intern("wrap");
        let wrap_params = stores.intern_token_list(&[Token::param(1)]);
        let wrap_body =
            stores.intern_token_list(&[char_token('['), Token::param(1), char_token(']')]);
        stores.set_macro_meaning(
            wrap,
            MacroMeaning::new(MeaningFlags::EMPTY, wrap_params, wrap_body),
        );

        let outer = stores.intern("outer");
        let outer_params = stores.intern_token_list(&[Token::param(1)]);
        let outer_body = stores.intern_token_list(&[
            Token::Cs(wrap),
            char_token('{'),
            Token::param(1),
            char_token('}'),
        ]);
        stores.set_macro_meaning(
            outer,
            MacroMeaning::new(MeaningFlags::EMPTY, outer_params, outer_body),
        );

        let invocation = stores.intern_token_list(&[
            Token::Cs(outer),
            char_token('{'),
            char_token('x'),
            char_token('y'),
            char_token('}'),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(invocation, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "[xy]");
    }

    #[test]
    fn identical_macro_bodies_keep_shared_body_identity_with_distinct_arguments() {
        let mut stores = Stores::new();
        let left = stores.intern("left");
        let right = stores.intern("right");
        let params = stores.intern_token_list(&[Token::param(1)]);
        let first_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
        let second_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
        assert_eq!(first_body, second_body);
        stores.set_macro_meaning(
            left,
            MacroMeaning::new(MeaningFlags::EMPTY, params, first_body),
        );
        stores.set_macro_meaning(
            right,
            MacroMeaning::new(MeaningFlags::EMPTY, params, second_body),
        );

        let left_arg = stores.intern_token_list(&[char_token('x')]);
        let mut left_input = InputStack::new(MemoryInput::new(""));
        left_input.push_token_list(left_arg, TokenListReplayKind::Inserted);
        let left_meaning = stores.meaning(left);
        let left_dispatch = dispatch(
            Token::Cs(left),
            &mut left_input,
            &mut stores,
            &mut NoopRecorder,
            left_meaning,
        )
        .expect("left dispatch should succeed");
        let super::Dispatch::Push {
            token_list: left_body,
            macro_arguments: left_arguments,
            ..
        } = left_dispatch
        else {
            panic!("expected left macro body push");
        };
        assert_eq!(left_body, first_body);
        assert_eq!(
            stores.tokens(left_arguments.get(1).expect("left #1")),
            &[char_token('x')]
        );

        let right_arg = stores.intern_token_list(&[char_token('y')]);
        let mut right_input = InputStack::new(MemoryInput::new(""));
        right_input.push_token_list(right_arg, TokenListReplayKind::Inserted);
        let right_meaning = stores.meaning(right);
        let right_dispatch = dispatch(
            Token::Cs(right),
            &mut right_input,
            &mut stores,
            &mut NoopRecorder,
            right_meaning,
        )
        .expect("right dispatch should succeed");
        let super::Dispatch::Push {
            token_list: right_body,
            macro_arguments: right_arguments,
            ..
        } = right_dispatch
        else {
            panic!("expected right macro body push");
        };
        assert_eq!(right_body, second_body);
        assert_eq!(
            stores.tokens(right_arguments.get(1).expect("right #1")),
            &[char_token('y')]
        );

        let invocation = stores.intern_token_list(&[
            Token::Cs(left),
            char_token('x'),
            Token::Cs(right),
            char_token('y'),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(invocation, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "x!y!");
    }

    #[test]
    fn string_respects_escapechar_and_renders_other_catcodes() {
        let mut stores = Stores::new();
        let string = expandable_primitive(&mut stores, "string", ExpandablePrimitive::String);
        let target = stores.intern("foo");
        let list = stores.intern_token_list(&[
            Token::Cs(string),
            Token::Cs(target),
            Token::Cs(string),
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(
            collect_expanded(&mut input, &mut stores),
            vec![
                Token::Char {
                    ch: '\\',
                    cat: Catcode::Other
                },
                Token::Char {
                    ch: 'f',
                    cat: Catcode::Other
                },
                Token::Char {
                    ch: 'o',
                    cat: Catcode::Other
                },
                Token::Char {
                    ch: 'o',
                    cat: Catcode::Other
                },
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Other
                },
            ]
        );
    }

    #[test]
    fn string_omits_invalid_escapechar() {
        let mut stores = Stores::new();
        stores.set_int_param(tex_state::env::banks::IntParam::ESCAPE_CHAR, -1);
        let string = expandable_primitive(&mut stores, "string", ExpandablePrimitive::String);
        let target = stores.intern("foo");
        let list = stores.intern_token_list(&[Token::Cs(string), Token::Cs(target)]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "foo");
    }

    #[test]
    fn number_and_romannumeral_scan_expanded_integer_edge_cases() {
        let mut stores = Stores::new();
        let number = expandable_primitive(&mut stores, "number", ExpandablePrimitive::Number);
        let roman = expandable_primitive(
            &mut stores,
            "romannumeral",
            ExpandablePrimitive::RomanNumeral,
        );
        let digits = stores.intern("digits");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('1'), char_token('9')]);
        stores.set_macro_meaning(digits, MacroMeaning::new(MeaningFlags::EMPTY, params, body));
        let list = stores.intern_token_list(&[
            Token::Cs(number),
            Token::Char {
                ch: '-',
                cat: Catcode::Other,
            },
            Token::Cs(digits),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(roman),
            Token::Char {
                ch: '0',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(roman),
            Token::Char {
                ch: '4',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '0',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '0',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '0',
                cat: Catcode::Other,
            },
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "-19mmmm");
    }

    #[test]
    fn meaning_renders_macro_text_and_output_catcodes() {
        let mut stores = Stores::new();
        let meaning = expandable_primitive(&mut stores, "meaning", ExpandablePrimitive::Meaning);
        let macro_cs = stores.intern("m");
        let params = stores.intern_token_list(&[Token::param(1)]);
        let body = stores.intern_token_list(&[char_token('a'), Token::param(1)]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let list = stores.intern_token_list(&[Token::Cs(meaning), Token::Cs(macro_cs)]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        let tokens = collect_expanded(&mut input, &mut stores);
        let text = tokens
            .iter()
            .map(|token| match token {
                Token::Char { ch, .. } => *ch,
                other => panic!("expected character token, got {other:?}"),
            })
            .collect::<String>();

        assert_eq!(text, "macro:#1->a#1");
        assert!(tokens.iter().all(|token| matches!(
            token,
            Token::Char {
                cat: Catcode::Other | Catcode::Space,
                ..
            }
        )));
    }

    #[test]
    fn the_renders_supported_registers_and_token_registers() {
        let mut stores = Stores::new();
        let the = expandable_primitive(&mut stores, "the", ExpandablePrimitive::The);
        let count = stores.intern("count");
        let dimen = stores.intern("dimen");
        let toks = stores.intern("toks");
        stores.set_count(2, -42);
        stores.set_dimen(3, tex_state::scaled::Scaled::from_raw(65_537));
        let toks_value = stores.intern_token_list(&[
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '!',
                cat: Catcode::Other,
            },
        ]);
        stores.set_toks(4, toks_value);
        let list = stores.intern_token_list(&[
            Token::Cs(the),
            Token::Cs(count),
            char_token('2'),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(the),
            Token::Cs(dimen),
            char_token('3'),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(the),
            Token::Cs(toks),
            char_token('4'),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(
            next_expanded_chars(&mut input, &mut stores),
            "-421.00002ptA!"
        );
    }

    #[test]
    fn rendered_output_is_frozen_and_rollback_removes_it() {
        let mut stores = Stores::new();
        let snapshot = stores.checkpoint();
        let number = expandable_primitive(&mut stores, "number", ExpandablePrimitive::Number);
        let list = stores.intern_token_list(&[Token::Cs(number), char_token('7')]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(
            get_x_token(&mut input, &mut stores).expect("number should expand"),
            Some(Token::Char {
                ch: '7',
                cat: Catcode::Other
            })
        );
        let rendered = match input.summary().frames().last() {
            Some(tex_lex::InputFrameSummary::TokenList { token_list, .. }) => *token_list,
            other => panic!("expected rendered token-list frame, got {other:?}"),
        };

        stores.rollback(snapshot);
        let err = std::panic::catch_unwind(|| stores.tokens(rendered));
        assert!(
            err.is_err(),
            "rendered output must be rollback-coupled frozen content"
        );
    }

    #[test]
    fn input_pushes_driver_source_and_returns_to_calling_source() {
        let mut stores = Stores::new();
        stores.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, 13);
        expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
        let mut input = InputStack::new(MemoryInput::new("\\input{inc}z"));
        let mut hooks = MemoryHooks::new("main").with_source("inc", "ab");

        assert_eq!(
            next_expanded_chars_with_hooks(&mut input, &mut stores, &mut hooks),
            "ab z "
        );
        assert_eq!(hooks.opened, vec!["inc"]);
    }

    #[test]
    fn endinput_finishes_current_line_then_pops_source() {
        let mut stores = Stores::new();
        stores.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, 13);
        expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
        expandable_primitive(&mut stores, "endinput", ExpandablePrimitive::EndInput);
        let mut input = InputStack::new(MemoryInput::new("\\input{inc}z"));
        let mut hooks = MemoryHooks::new("main").with_source("inc", "a\\endinput b\nc");

        assert_eq!(
            next_expanded_chars_with_hooks(&mut input, &mut stores, &mut hooks),
            "ab z "
        );
    }

    #[test]
    fn jobname_expands_from_driver_hook_as_rendered_tokens() {
        let mut stores = Stores::new();
        expandable_primitive(&mut stores, "jobname", ExpandablePrimitive::JobName);
        let mut input = InputStack::new(MemoryInput::new("\\jobname"));
        let mut hooks = MemoryHooks::new("paper");

        let tokens = collect_expanded_with_hooks(&mut input, &mut stores, &mut hooks);
        let text = tokens
            .iter()
            .map(|token| match token {
                Token::Char { ch, .. } => *ch,
                other => panic!("expected character token, got {other:?}"),
            })
            .collect::<String>();

        assert_eq!(text, "paper");
        assert!(tokens.iter().all(|token| matches!(
            token,
            Token::Char {
                cat: Catcode::Other,
                ..
            }
        )));
    }

    #[test]
    fn fontname_stub_consumes_selector_and_expands_empty() {
        let mut stores = Stores::new();
        expandable_primitive(&mut stores, "fontname", ExpandablePrimitive::FontName);
        let nullfont = stores.intern("nullfont");
        stores.set_meaning(nullfont, Meaning::Relax);
        let list = stores.intern_token_list(&[
            Token::Cs(stores.symbol("fontname").expect("fontname")),
            Token::Cs(nullfont),
            char_token('z'),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "z");
    }

    #[test]
    fn mark_family_stubs_expand_empty() {
        let mut stores = Stores::new();
        for (name, primitive) in [
            ("topmark", ExpandablePrimitive::TopMark),
            ("firstmark", ExpandablePrimitive::FirstMark),
            ("botmark", ExpandablePrimitive::BotMark),
            ("splitfirstmark", ExpandablePrimitive::SplitFirstMark),
            ("splitbotmark", ExpandablePrimitive::SplitBotMark),
        ] {
            expandable_primitive(&mut stores, name, primitive);
        }
        let list = stores.intern_token_list(&[
            Token::Cs(stores.symbol("topmark").expect("topmark")),
            Token::Cs(stores.symbol("firstmark").expect("firstmark")),
            Token::Cs(stores.symbol("botmark").expect("botmark")),
            Token::Cs(stores.symbol("splitfirstmark").expect("splitfirstmark")),
            Token::Cs(stores.symbol("splitbotmark").expect("splitbotmark")),
            char_token('z'),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "z");
    }

    #[test]
    fn iftrue_and_iffalse_select_expected_two_limb_branches() {
        let mut stores = Stores::new();
        let (iftrue, iffalse, else_cs, fi) = conditional_primitives(&mut stores);
        let list = stores.intern_token_list(&[
            Token::Cs(iftrue),
            char_token('t'),
            Token::Cs(else_cs),
            char_token('f'),
            Token::Cs(fi),
            Token::Cs(iffalse),
            char_token('f'),
            Token::Cs(else_cs),
            char_token('t'),
            Token::Cs(fi),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "tt");
    }

    #[test]
    fn if_expands_to_two_unexpandable_character_tokens_before_comparing_charcodes() {
        let mut stores = Stores::new();
        let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
        let if_cs = expandable_primitive(&mut stores, "if", ExpandablePrimitive::If);
        let left = stores.intern("left");
        let right = stores.intern("right");
        let params = stores.intern_token_list(&[]);
        let left_body = stores.intern_token_list(&[char_token('a')]);
        let right_body = stores.intern_token_list(&[Token::Char {
            ch: 'a',
            cat: Catcode::Other,
        }]);
        stores.set_macro_meaning(
            left,
            MacroMeaning::new(MeaningFlags::EMPTY, params, left_body),
        );
        stores.set_macro_meaning(
            right,
            MacroMeaning::new(MeaningFlags::EMPTY, params, right_body),
        );
        let list = stores.intern_token_list(&[
            Token::Cs(if_cs),
            Token::Cs(left),
            Token::Cs(right),
            char_token('y'),
            Token::Cs(else_cs),
            char_token('n'),
            Token::Cs(fi),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "y");
    }

    #[test]
    fn ifcat_compares_category_codes_after_expansion() {
        let mut stores = Stores::new();
        let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
        let ifcat = expandable_primitive(&mut stores, "ifcat", ExpandablePrimitive::IfCat);
        let macro_cs = stores.intern("letter");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('b')]);
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let list = stores.intern_token_list(&[
            Token::Cs(ifcat),
            char_token('a'),
            Token::Cs(macro_cs),
            char_token('y'),
            Token::Cs(else_cs),
            char_token('n'),
            Token::Cs(fi),
            Token::Cs(ifcat),
            char_token('a'),
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            char_token('n'),
            Token::Cs(else_cs),
            char_token('y'),
            Token::Cs(fi),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");
    }

    #[test]
    fn ifx_compares_identical_macro_definitions_by_flags_and_hash_consed_ids() {
        let mut stores = Stores::new();
        let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
        let ifx = expandable_primitive(&mut stores, "ifx", ExpandablePrimitive::IfX);
        let left = stores.intern("left");
        let right = stores.intern("right");
        let protected = stores.intern("protected");
        let params = stores.intern_token_list(&[Token::param(1)]);
        let left_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
        let right_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
        assert_eq!(left_body, right_body);
        stores.set_macro_meaning(
            left,
            MacroMeaning::new(MeaningFlags::EMPTY, params, left_body),
        );
        stores.set_macro_meaning(
            right,
            MacroMeaning::new(MeaningFlags::EMPTY, params, right_body),
        );
        stores.set_macro_meaning(
            protected,
            MacroMeaning::new(MeaningFlags::PROTECTED, params, right_body),
        );
        let list = stores.intern_token_list(&[
            Token::Cs(ifx),
            Token::Cs(left),
            Token::Cs(right),
            char_token('y'),
            Token::Cs(else_cs),
            char_token('n'),
            Token::Cs(fi),
            Token::Cs(ifx),
            Token::Cs(left),
            Token::Cs(protected),
            char_token('n'),
            Token::Cs(else_cs),
            char_token('y'),
            Token::Cs(fi),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");
    }

    #[test]
    fn ifx_uses_meaning_word_equality_for_non_macros_without_expansion() {
        let mut stores = Stores::new();
        let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
        let ifx = expandable_primitive(&mut stores, "ifx", ExpandablePrimitive::IfX);
        let first = stores.intern("first");
        let second = stores.intern("second");
        let macro_cs = stores.intern("macro");
        let params = stores.intern_token_list(&[]);
        let body = stores.intern_token_list(&[char_token('a')]);
        stores.set_meaning(first, Meaning::CharGiven('a'));
        stores.set_meaning(second, Meaning::CharGiven('a'));
        stores.set_macro_meaning(
            macro_cs,
            MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        );
        let list = stores.intern_token_list(&[
            Token::Cs(ifx),
            Token::Cs(first),
            Token::Cs(second),
            char_token('y'),
            Token::Cs(else_cs),
            char_token('n'),
            Token::Cs(fi),
            Token::Cs(ifx),
            Token::Cs(macro_cs),
            char_token('a'),
            char_token('n'),
            Token::Cs(else_cs),
            char_token('y'),
            Token::Cs(fi),
        ]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");
    }

    fn next_expanded_chars(input: &mut InputStack<MemoryInput>, stores: &mut Stores) -> String {
        let mut out = String::new();
        while let Some(token) = get_x_token(input, stores).expect("expansion should succeed") {
            let Token::Char { ch, .. } = token else {
                panic!("expected character token, got {token:?}");
            };
            out.push(ch);
        }
        out
    }

    fn collect_expanded(input: &mut InputStack<MemoryInput>, stores: &mut Stores) -> Vec<Token> {
        let mut out = Vec::new();
        while let Some(token) = get_x_token(input, stores).expect("expansion should succeed") {
            out.push(token);
        }
        out
    }

    fn next_expanded_chars_with_hooks(
        input: &mut InputStack<MemoryInput>,
        stores: &mut Stores,
        hooks: &mut MemoryHooks,
    ) -> String {
        let mut out = String::new();
        while let Some(token) =
            get_x_token_with_hooks(input, stores, hooks).expect("expansion should succeed")
        {
            let Token::Char { ch, .. } = token else {
                panic!("expected character token, got {token:?}");
            };
            out.push(ch);
        }
        out
    }

    fn collect_expanded_with_hooks(
        input: &mut InputStack<MemoryInput>,
        stores: &mut Stores,
        hooks: &mut MemoryHooks,
    ) -> Vec<Token> {
        let mut out = Vec::new();
        while let Some(token) =
            get_x_token_with_hooks(input, stores, hooks).expect("expansion should succeed")
        {
            out.push(token);
        }
        out
    }

    fn char_token(ch: char) -> Token {
        let cat = match ch {
            '{' => Catcode::BeginGroup,
            '}' => Catcode::EndGroup,
            '0'..='9' | '[' | ']' | '!' => Catcode::Other,
            _ => Catcode::Letter,
        };
        Token::Char { ch, cat }
    }

    fn expandable_primitive(
        stores: &mut Stores,
        name: &str,
        primitive: ExpandablePrimitive,
    ) -> Symbol {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
        symbol
    }

    fn csname_primitives(stores: &mut Stores) -> (Symbol, Symbol) {
        (
            expandable_primitive(stores, "csname", ExpandablePrimitive::CsName),
            expandable_primitive(stores, "endcsname", ExpandablePrimitive::EndCsName),
        )
    }

    fn conditional_primitives(stores: &mut Stores) -> (Symbol, Symbol, Symbol, Symbol) {
        (
            expandable_primitive(stores, "iftrue", ExpandablePrimitive::IfTrue),
            expandable_primitive(stores, "iffalse", ExpandablePrimitive::IfFalse),
            expandable_primitive(stores, "else", ExpandablePrimitive::Else),
            expandable_primitive(stores, "fi", ExpandablePrimitive::Fi),
        )
    }

    struct MemoryHooks {
        job_name: String,
        sources: HashMap<String, String>,
        opened: Vec<String>,
    }

    impl MemoryHooks {
        fn new(job_name: &str) -> Self {
            Self {
                job_name: job_name.to_owned(),
                sources: HashMap::new(),
                opened: Vec::new(),
            }
        }

        fn with_source(mut self, name: &str, input: &str) -> Self {
            self.sources.insert(name.to_owned(), input.to_owned());
            self
        }
    }

    impl ExpansionHooks<MemoryInput> for MemoryHooks {
        fn open_input(&mut self, name: &str) -> Result<MemoryInput, String> {
            let source = self
                .sources
                .get(name)
                .ok_or_else(|| "missing memory source".to_owned())?;
            self.opened.push(name.to_owned());
            Ok(MemoryInput::new(source.clone()))
        }

        fn job_name(&self) -> &str {
            &self.job_name
        }
    }
}
