use tex_expand::{ExpansionHooks, NoopRecorder, ReadRecorder};
use tex_lex::{InputSource, InputStack};
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
use tex_state::provenance::InsertedOriginKind;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{ContentHash, GroupKind, GroupMismatch, Universe};

use crate::executor::sync_engine_state;
use crate::{ExecError, Mode, ModeNest, assignments};

/// Main-control progress counters.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionStats {
    pub delivered_tokens: usize,
    pub shipped_artifacts: Vec<ContentHash>,
    pub dumped_format: bool,
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
    traced: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    dispatch_delivered_token_with_recorder(nest, traced, input, stores, &mut recorder, hooks)
}

pub(crate) fn dispatch_delivered_token_with_recorder<S, R, H>(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
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
    let token = tex_expand::semantic_token(traced);
    let origin = traced.origin();
    if stores.origin_is_inserted_kind(origin, InsertedOriginKind::NoExpand) {
        return Ok(DispatchAction::Continue);
    }
    let mode = nest.current_mode();
    if matches!(mode, Mode::Math | Mode::DisplayMath) {
        return crate::math::dispatch_math_token_with_recorder(
            nest, traced, input, stores, recorder, hooks,
        );
    }
    if matches!(
        token,
        Token::Char {
            cat: Catcode::Superscript | Catcode::Subscript,
            ..
        }
    ) {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Missing $ inserted.\nI've inserted a begin-math/end-math symbol since I think\nyou left one out. Proceed, with fingers crossed.\n",
        );
        push_traced_tokens(input, stores, [traced]);
        return crate::math::enter_math(nest, input, stores, hooks);
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
            ch,
            cat: Catcode::Active,
        } => {
            let symbol = assignments::active_character_symbol(stores, ch);
            stores.meaning(symbol)
        }
        Token::Char { .. } => return dispatch_character_token(nest, traced, input, stores, hooks),
        Token::Param(_) | Token::Frozen(_) => {
            return Ok(DispatchAction::NotConsumed);
        }
    };

    let continues_character_run = matches!(
        meaning,
        Meaning::CharGiven(_)
            | Meaning::CharToken {
                cat: Catcode::Letter | Catcode::Other,
                ..
            }
            | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
    );
    if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) && !continues_character_run {
        assignments::flush_pending_hchars(nest, stores)?;
        sync_engine_state::<S, _>(hooks, nest, stores);
    }

    if matches!(mode, Mode::Vertical | Mode::InternalVertical)
        && matches!(
            meaning,
            Meaning::CharGiven(_)
                | Meaning::CharToken {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                }
        )
    {
        start_paragraph_before_replaying_character(nest, traced, input, stores)?;
        return Ok(DispatchAction::Continue);
    }

    match meaning {
        Meaning::Relax => Ok(DispatchAction::Continue),
        Meaning::Undefined => {
            // Undefined tokens can reach main control without passing through
            // expansion (for example after \noexpand or scanner recovery).
            // TeX diagnoses and consumes them rather than aborting the run.
            let name = stores.resolve_cs_name(token);
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                &format!("\n! Undefined control sequence \\{name}.\n"),
            );
            Ok(DispatchAction::Continue)
        }
        Meaning::CharGiven(ch) => {
            assignments::append_given_char(nest, input, stores, ch)?;
            Ok(DispatchAction::Continue)
        }
        Meaning::CharToken { ch, cat } => dispatch_character_token(
            nest,
            TracedTokenWord::pack(Token::Char { ch, cat }, origin),
            input,
            stores,
            hooks,
        ),
        Meaning::Macro { .. } => Err(ExecError::UnexpectedMacroDelivery {
            name: stores.resolve_cs_name(token),
            origin,
        }),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) => {
            stores.world_mut().write_text(
                tex_state::PrintSink::TerminalAndLog,
                "\n! Extra \\endcsname.\nI'm ignoring this control sequence.\n",
            );
            Ok(DispatchAction::Continue)
        }
        Meaning::ExpandablePrimitive(primitive) => {
            dispatch_delivered_expandable(token, primitive, origin)
        }
        Meaning::UnexpandablePrimitive(primitive) => {
            assignments::execute_unexpandable_with_recorder(
                primitive, traced, nest, input, stores, recorder, hooks,
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
        Meaning::InternalInteger(_) => {
            // TeX.web's mode-dependent main-control table routes a bare
            // internal quantity (TRIP uses `\badness`) to `report_illegal_case`
            // and then continues; it is not a fatal assignment error.
            crate::diagnostics::report_illegal_case(stores, token, mode);
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
        | Meaning::MuGlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)) => {
            assignments::execute_assignment_meaning(meaning, traced, input, stores, hooks)
        }
        Meaning::MathCharGiven(_) => {
            unimplemented_typesetting(mode, token, origin, "math character command")
        }
        Meaning::Unknown(raw) => Err(ExecError::UnsupportedCommand {
            token,
            opcode: raw.op(),
            origin,
        }),
    }
}

fn dispatch_character_token<S, H>(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<DispatchAction, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let token = tex_expand::semantic_token(traced);
    let origin = traced.origin();
    match token {
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => {
            assignments::flush_pending_hchars(nest, stores)?;
            stores.enter_group_with_kind(GroupKind::Simple);
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        } => {
            assignments::flush_pending_hchars(nest, stores)?;
            if let Err(error) = leave_group_with_origin(input, stores, GroupKind::Simple, origin) {
                match error {
                    ExecError::TooManyRightBraces { .. } => stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        "\n! Too many }'s.\nYou've closed more groups than you opened.\nSuch booboos are generally harmless, so keep going.\n",
                    ),
                    ExecError::ExtraRightBraceOrForgottenDollar { .. } => stores
                        .world_mut()
                        .write_text(
                            tex_state::PrintSink::TerminalAndLog,
                            "\n! Extra }, or forgotten $.\nI've deleted a group-closing symbol because it seems to be\nspurious, as in `$x}$'. But perhaps the } is legitimate and\nyou forgot something else, as in `\\hbox{$x}'.\n",
                        ),
                    error => return Err(error),
                }
            }
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::MathShift,
            ..
        } => crate::math::enter_math(nest, input, stores, hooks),
        Token::Char {
            cat: Catcode::Space,
            ..
        } => {
            let _ = assignments::try_append_character(nest, token, stores)?;
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::Parameter,
            ..
        } => {
            // TeX.web §1046 routes `any_mode(mac_param)` through
            // `report_illegal_case`. In particular, a stray `#` in outer
            // vertical mode is consumed without starting a paragraph (and
            // therefore without invoking the page builder).
            crate::diagnostics::report_illegal_case(stores, token, nest.current_mode());
            Ok(DispatchAction::Continue)
        }
        Token::Char { ch, .. } => {
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                start_paragraph_before_replaying_character(nest, traced, input, stores)?;
                return Ok(DispatchAction::Continue);
            }
            if assignments::try_append_character(nest, token, stores)? {
                return Ok(DispatchAction::Continue);
            }
            assignments::append_given_char(nest, input, stores, ch)?;
            Ok(DispatchAction::Continue)
        }
        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {
            unreachable!("caller passes a character token")
        }
    }
}

fn start_paragraph_before_replaying_character<S>(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    input: &mut InputStack<S>,
    stores: &mut Universe,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    // TeX82 backs up the triggering token before `new_graf`, whose `every_par`
    // replay must therefore run before that first character is reconsidered.
    push_traced_tokens(input, stores, [traced]);
    assignments::ensure_horizontal_for_character(nest, input, stores)
}

fn dispatch_delivered_expandable(
    token: Token,
    primitive: ExpandablePrimitive,
    origin: OriginId,
) -> Result<DispatchAction, ExecError> {
    match primitive {
        ExpandablePrimitive::EndCsName => Err(ExecError::ExtraEndCsName { origin }),
        ExpandablePrimitive::Fi | ExpandablePrimitive::Else | ExpandablePrimitive::Or => {
            Err(ExecError::ExtraConditionalControl { primitive, origin })
        }
        _ => Err(ExecError::UnexpectedExpandableDelivery {
            token,
            primitive,
            origin,
        }),
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
    leave_group_with_origin(input, stores, expected, OriginId::UNKNOWN)
}

pub(crate) fn leave_group_with_origin<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    expected: GroupKind,
    origin: OriginId,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    match stores.leave_group_with_kind(expected) {
        Ok(tokens) => {
            let tokens: Vec<_> = tokens
                .into_iter()
                .map(|token| {
                    let inserted =
                        stores.inserted_origin(InsertedOriginKind::AfterGroup, token, origin);
                    TracedTokenWord::pack(token, inserted)
                })
                .collect();
            insert_traced_tokens(input, stores, tokens);
            Ok(())
        }
        Err(mismatch) => Err(group_mismatch_error(expected, mismatch, origin)),
    }
}

fn group_mismatch_error(
    expected: GroupKind,
    mismatch: GroupMismatch,
    origin: OriginId,
) -> ExecError {
    let no_open_group = mismatch.actual() == expected;
    match (expected, mismatch.actual(), no_open_group) {
        (GroupKind::Simple, _, true) => ExecError::TooManyRightBraces { origin },
        (GroupKind::Simple, GroupKind::SemiSimple, false) => {
            ExecError::ExtraRightBraceOrForgottenEndgroup { origin }
        }
        (GroupKind::Simple, GroupKind::MathShift, false) => {
            ExecError::ExtraRightBraceOrForgottenDollar { origin }
        }
        (GroupKind::Simple, GroupKind::Box | GroupKind::Align, false) => {
            ExecError::ExtraRightBraceOrForgottenDollar { origin }
        }
        (GroupKind::SemiSimple, _, true) => ExecError::ExtraEndGroup { origin },
        (GroupKind::Box, _, true) => ExecError::EndGroupMismatch {
            started_by: "the outer level",
            origin,
        },
        (GroupKind::Box, _, false) => ExecError::EndGroupMismatch {
            started_by: mismatch.actual().start_text(),
            origin,
        },
        (
            GroupKind::SemiSimple,
            GroupKind::Simple | GroupKind::Box | GroupKind::MathShift | GroupKind::Align,
            false,
        ) => ExecError::EndGroupMismatch {
            started_by: mismatch.actual().start_text(),
            origin,
        },
        (GroupKind::MathShift, _, true) => ExecError::MathShiftGroupMismatch {
            started_by: "the outer level",
            origin,
        },
        (
            GroupKind::MathShift,
            GroupKind::Simple | GroupKind::Box | GroupKind::SemiSimple | GroupKind::Align,
            false,
        ) => ExecError::MathShiftGroupMismatch {
            started_by: mismatch.actual().start_text(),
            origin,
        },
        (GroupKind::Simple, GroupKind::Simple, false)
        | (GroupKind::SemiSimple, GroupKind::SemiSimple, false)
        | (GroupKind::MathShift, GroupKind::MathShift, false) => {
            unreachable!("matching group kinds are returned as successful leaves, not mismatches")
        }
        (GroupKind::Align, _, _) => ExecError::EndGroupMismatch {
            started_by: mismatch.actual().start_text(),
            origin,
        },
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

pub(crate) fn push_traced_tokens<S, I>(input: &mut InputStack<S>, stores: &mut Universe, tokens: I)
where
    S: InputSource,
    I: IntoIterator<Item = TracedTokenWord>,
{
    tex_expand::back_input(input, stores, tokens);
}

pub(crate) fn insert_traced_tokens<S, I>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    tokens: I,
) where
    S: InputSource,
    I: IntoIterator<Item = TracedTokenWord>,
{
    tex_expand::insert_input(input, stores, tokens);
}

pub(crate) fn unimplemented_typesetting(
    mode: Mode,
    token: Token,
    origin: OriginId,
    operation: &'static str,
) -> Result<DispatchAction, ExecError> {
    Err(ExecError::UnimplementedTypesetting {
        mode,
        token,
        origin,
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
            Token::Frozen(_) => "\\endtemplate".to_owned(),
        }
    }
}
