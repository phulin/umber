use tex_expand::{ExpansionHooks, NoopRecorder, ReadRecorder};
use tex_lex::{InputSource, InputStack};
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{Catcode, Token};
use tex_state::{ContentHash, GroupKind, GroupMismatch, Universe};

use crate::executor::sync_engine_state;
use crate::{ExecError, Mode, ModeNest, assignments};

/// Main-control progress counters.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionStats {
    pub delivered_tokens: usize,
    pub shipped_artifacts: Vec<ContentHash>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchAction {
    Continue,
    End,
    NotConsumed,
    Shipout(ContentHash),
}

/// Dispatches one gullet-delivered token in the current mode.
pub fn dispatch_delivered_token<S, H>(
    nest: &mut ModeNest,
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    dispatch_delivered_token_with_recorder(nest, token, input, stores, &mut recorder, hooks)
}

pub(crate) fn dispatch_delivered_token_with_recorder<S, R, H>(
    nest: &mut ModeNest,
    token: Token,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let mode = nest.current_mode();
    if matches!(mode, Mode::Math | Mode::DisplayMath) {
        return crate::math::dispatch_math_token_with_recorder(
            nest, token, input, stores, recorder, hooks,
        );
    }
    if matches!(
        token,
        Token::Char {
            cat: Catcode::MathShift,
            ..
        }
    ) {
        return crate::math::enter_math(nest, input, stores, hooks);
    }
    let meaning = match token {
        Token::Cs(symbol) => stores.meaning(symbol),
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => {
            stores.enter_group_with_kind(GroupKind::Simple);
            return Ok(DispatchAction::Continue);
        }
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        } => {
            leave_group(input, stores, GroupKind::Simple)?;
            return Ok(DispatchAction::Continue);
        }
        Token::Char {
            cat: Catcode::Space,
            ..
        } => {
            if assignments::try_append_character(nest, token, stores)? {
                return Ok(DispatchAction::Continue);
            }
            return Ok(DispatchAction::Continue);
        }
        Token::Char { ch, .. } => {
            if assignments::try_append_character(nest, token, stores)? {
                return Ok(DispatchAction::Continue);
            }
            assignments::append_given_char(nest, input, stores, ch)?;
            return Ok(DispatchAction::Continue);
        }
        Token::Param(_) => {
            return Ok(DispatchAction::NotConsumed);
        }
    };

    if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
        assignments::flush_pending_hchars(nest, stores)?;
        sync_engine_state::<S, _>(hooks, nest, stores);
    }

    match meaning {
        Meaning::Relax => Ok(DispatchAction::Continue),
        Meaning::Undefined => Err(ExecError::UndefinedControlSequence {
            name: stores.resolve_cs_name(token),
        }),
        Meaning::CharGiven(ch) => {
            assignments::append_given_char(nest, input, stores, ch)?;
            Ok(DispatchAction::Continue)
        }
        Meaning::Macro { .. } => Err(ExecError::UnexpectedMacroDelivery {
            name: stores.resolve_cs_name(token),
        }),
        Meaning::ExpandablePrimitive(primitive) => dispatch_delivered_expandable(token, primitive),
        Meaning::UnexpandablePrimitive(primitive) => {
            assignments::execute_unexpandable_with_recorder(
                primitive, nest, input, stores, recorder, hooks,
            )
        }
        Meaning::Font(id) => {
            if let Token::Cs(symbol) = token {
                stores.set_current_font_selector(symbol, id);
            } else {
                stores.set_current_font(id);
            }
            Ok(DispatchAction::Continue)
        }
        meaning @ (Meaning::CountRegister(_)
        | Meaning::DimenRegister(_)
        | Meaning::SkipRegister(_)
        | Meaning::MuskipRegister(_)
        | Meaning::ToksRegister(_)
        | Meaning::IntParam(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)) => {
            assignments::execute_assignment_meaning(meaning, input, stores, hooks)
        }
        Meaning::MathCharGiven(_) => {
            unimplemented_typesetting(mode, token, "math character command")
        }
        Meaning::Unknown(raw) => Err(ExecError::UnsupportedCommand {
            token,
            opcode: raw.op(),
        }),
    }
}

fn dispatch_delivered_expandable(
    token: Token,
    primitive: ExpandablePrimitive,
) -> Result<DispatchAction, ExecError> {
    match primitive {
        ExpandablePrimitive::EndCsName => Err(ExecError::ExtraEndCsName),
        ExpandablePrimitive::Fi | ExpandablePrimitive::Else | ExpandablePrimitive::Or => {
            Err(ExecError::ExtraConditionalControl(primitive))
        }
        _ => Err(ExecError::UnexpectedExpandableDelivery { token, primitive }),
    }
}

pub(crate) fn leave_group<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    expected: GroupKind,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    match stores.leave_group_with_kind(expected) {
        Ok(tokens) => {
            push_tokens(input, stores, tokens);
            Ok(())
        }
        Err(mismatch) => Err(group_mismatch_error(expected, mismatch)),
    }
}

fn group_mismatch_error(expected: GroupKind, mismatch: GroupMismatch) -> ExecError {
    let no_open_group = mismatch.actual() == expected;
    match (expected, mismatch.actual(), no_open_group) {
        (GroupKind::Simple, _, true) => ExecError::TooManyRightBraces,
        (GroupKind::Simple, GroupKind::SemiSimple, false) => {
            ExecError::ExtraRightBraceOrForgottenEndgroup
        }
        (GroupKind::SemiSimple, _, true) => ExecError::ExtraEndGroup,
        (GroupKind::SemiSimple, GroupKind::Simple, false) => ExecError::EndGroupMismatch {
            started_by: mismatch.actual().start_text(),
        },
        (GroupKind::Simple, GroupKind::Simple, false)
        | (GroupKind::SemiSimple, GroupKind::SemiSimple, false) => {
            unreachable!("matching group kinds are returned as successful leaves, not mismatches")
        }
    }
}

pub(crate) fn push_tokens<S, I>(input: &mut InputStack<S>, stores: &mut Universe, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = Token>,
{
    let tokens: Vec<_> = tokens.into_iter().collect();
    if tokens.is_empty() {
        return;
    }
    let token_list = stores.intern_token_list(&tokens);
    input.push_token_list(token_list, tex_lex::TokenListReplayKind::Inserted);
}

pub(crate) fn unimplemented_typesetting(
    mode: Mode,
    token: Token,
    operation: &'static str,
) -> Result<DispatchAction, ExecError> {
    Err(ExecError::UnimplementedTypesetting {
        mode,
        token,
        operation,
    })
}

trait ResolveTokenName {
    fn resolve_cs_name(&self, token: Token) -> String;
}

impl ResolveTokenName for Universe {
    fn resolve_cs_name(&self, token: Token) -> String {
        match token {
            Token::Cs(symbol) => self.resolve(symbol).to_owned(),
            Token::Char { ch, cat } => format!("{ch:?}/{cat:?}"),
            Token::Param(slot) => format!("#{slot}"),
        }
    }
}
