//! Retired `InputStack` main-control dispatch and token replay helpers.

use tex_lex::InputStack;
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
use tex_state::provenance::InsertedOriginKind;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{GroupKind, GroupMismatch, Universe};

use crate::dispatch::DispatchAction;
use crate::executor::sync_engine_state;
use crate::{ExecError, Mode, ModeNest, assignments};

/// Dispatches one gullet-delivered token in the current mode.
pub fn dispatch_delivered_token(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    dispatch_delivered_token_with_context(nest, traced, input, stores, execution)
}

pub(crate) fn dispatch_delivered_token_with_context(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    let token = traced.semantic_token();
    if token.is_frozen_endv() {
        match crate::align::legacy_execution::do_endv(traced, input, stores)? {
            crate::align::legacy_execution::DoEndV::Recovered => {
                return Ok(DispatchAction::Continue);
            }
            crate::align::legacy_execution::DoEndV::NotApplicable
            | crate::align::legacy_execution::DoEndV::FinishCell => {}
        }
    }
    let origin = traced.origin();
    if stores.origin_is_inserted_kind(origin, InsertedOriginKind::NoExpand) {
        return Ok(DispatchAction::Continue);
    }
    let mode = nest.current_mode();
    if matches!(mode, Mode::Math | Mode::DisplayMath) {
        return crate::math::legacy_front::dispatch_math_token_with_context(
            nest, traced, input, stores, execution,
        );
    }
    if matches!(
        token,
        Token::Char {
            cat: Catcode::Superscript | Catcode::Subscript,
            ..
        }
    ) {
        crate::math::legacy_front::insert_dollar_sign(traced, input, stores)?;
        if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
            assignments::ensure_horizontal_for_character(
                nest,
                input,
                stores,
                execution.command_fuel(),
            )?;
        }
        return Ok(DispatchAction::Continue);
    }
    if matches!(
        token,
        Token::Char {
            cat: Catcode::MathShift,
            ..
        }
    ) {
        if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
            // tex.web §1090 backs up the math shift before `new_graf`, so
            // \everypar must run before main control retries the `$` and
            // performs the doubled-shift lookahead in horizontal mode.
            push_traced_tokens(input, stores, [traced]);
            assignments::ensure_horizontal_for_character(
                nest,
                input,
                stores,
                execution.command_fuel(),
            )?;
            return Ok(DispatchAction::Continue);
        }
        return crate::math::legacy_front::enter_math(nest, input, stores, execution);
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
        Token::Char { .. } => {
            return dispatch_character_token(nest, traced, input, stores, execution);
        }
        Token::Frozen(_) if stores.frozen_primitive_meaning(token).is_some() => stores
            .frozen_primitive_meaning(token)
            .expect("guard established frozen primitive meaning"),
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
        assignments::flush_pending_hchars(nest, stores, execution.command_fuel())?;
        sync_engine_state(execution, nest, stores);
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
        start_paragraph_before_replaying_character(nest, traced, input, stores, execution)?;
        return Ok(DispatchAction::Continue);
    }

    match meaning {
        Meaning::Relax | Meaning::EndV => Ok(DispatchAction::Continue),
        Meaning::Undefined => {
            // Undefined tokens can reach main control without passing through
            // expansion (for example after \noexpand or scanner recovery).
            // TeX diagnoses and consumes them rather than aborting the run.
            crate::executor::report_undefined_control_sequence(input, stores)?;
            Ok(DispatchAction::Continue)
        }
        Meaning::CharGiven(ch) => {
            assignments::append_given_char(
                nest,
                input,
                stores,
                ch,
                origin,
                execution.command_fuel(),
            )?;
            Ok(DispatchAction::Continue)
        }
        Meaning::CharToken { ch, cat } => dispatch_character_token(
            nest,
            TracedTokenWord::pack(Token::Char { ch, cat }, origin),
            input,
            stores,
            execution,
        ),
        Meaning::Macro { .. } => Err(ExecError::UnexpectedMacroDelivery {
            name: stores.resolve_cs_name(token),
            origin,
        }),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) => {
            // tex.web §1135's `cs_error`.
            crate::error_report::report_input_error(
                input,
                stores,
                "Extra \\endcsname",
                &["I'm ignoring this, since I wasn't doing a \\csname."],
            )?;
            Ok(DispatchAction::Continue)
        }
        Meaning::ExpandablePrimitive(primitive) => {
            dispatch_delivered_expandable(token, primitive, origin)
        }
        Meaning::UnexpandablePrimitive(primitive) => {
            assignments::execute_unexpandable_with_context(
                primitive, traced, nest, input, stores, execution,
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
            crate::diagnostics::report_illegal_case(stores, token, mode)?;
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
            assignments::execute_assignment_meaning(meaning, traced, input, stores, execution)
        }
        Meaning::MathCharGiven(_) => {
            crate::math::legacy_front::insert_dollar_sign(traced, input, stores)?;
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                assignments::ensure_horizontal_for_character(
                    nest,
                    input,
                    stores,
                    execution.command_fuel(),
                )?;
            }
            Ok(DispatchAction::Continue)
        }
        Meaning::Unknown(raw) => Err(ExecError::UnsupportedCommand {
            token,
            opcode: raw.op(),
            origin,
        }),
    }
}

fn dispatch_character_token(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<DispatchAction, ExecError> {
    let token = traced.semantic_token();
    let origin = traced.origin();
    match token {
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => {
            assignments::flush_pending_hchars(nest, stores, execution.command_fuel())?;
            stores.enter_group_with_kind(GroupKind::Simple);
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        } => {
            assignments::flush_pending_hchars(nest, stores, execution.command_fuel())?;
            if let Err(error) = leave_group_with_origin(input, stores, GroupKind::Simple, origin) {
                match error {
                    // tex.web §1068's `bottom_level` case of `handle_right_brace`.
                    ExecError::TooManyRightBraces { .. } => {
                        crate::error_report::report_input_error(
                            input,
                            stores,
                            "Too many }'s",
                            &[
                                "You've closed more groups than you opened.",
                                "Such booboos are generally harmless, so keep going.",
                            ],
                        )?;
                    }
                    // §1069's `extra_right_brace`, whose message names the
                    // closer the open group was actually waiting for.
                    ExecError::ExtraRightBraceOrForgottenDollar { .. } => {
                        report_extra_right_brace(input, stores, "$")?;
                    }
                    error => return Err(error),
                }
            } else {
                execution.paragraph_group_exited(stores);
            }
            Ok(DispatchAction::Continue)
        }
        Token::Char {
            cat: Catcode::MathShift,
            ..
        } => {
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                push_traced_tokens(input, stores, [traced]);
                assignments::ensure_horizontal_for_character(
                    nest,
                    input,
                    stores,
                    execution.command_fuel(),
                )?;
                Ok(DispatchAction::Continue)
            } else {
                crate::math::legacy_front::enter_math(nest, input, stores, execution)
            }
        }
        Token::Char {
            cat: Catcode::Space,
            ..
        } => {
            let _ =
                assignments::try_append_character(nest, traced, stores, execution.command_fuel())?;
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
            crate::diagnostics::report_illegal_case(stores, token, nest.current_mode())?;
            Ok(DispatchAction::Continue)
        }
        Token::Char { ch, .. } => {
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                start_paragraph_before_replaying_character(nest, traced, input, stores, execution)?;
                return Ok(DispatchAction::Continue);
            }
            if assignments::try_append_character(nest, traced, stores, execution.command_fuel())? {
                return Ok(DispatchAction::Continue);
            }
            assignments::append_given_char(
                nest,
                input,
                stores,
                ch,
                origin,
                execution.command_fuel(),
            )?;
            Ok(DispatchAction::Continue)
        }
        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {
            unreachable!("caller passes a character token")
        }
    }
}

/// tex.web §1069's `extra_right_brace`.
///
/// `expected` is what §1069's `case cur_group of` prints after the message
/// stem: `\endgroup`, `$`, or `\right`. Only the math-shift case reaches
/// here; the semi-simple one still leaves main control as a typed error.
fn report_extra_right_brace(
    input: &InputStack,
    stores: &mut Universe,
    expected: &str,
) -> Result<(), ExecError> {
    crate::error_report::report_input_error(
        input,
        stores,
        &format!("Extra }}, or forgotten {expected}"),
        &[
            "I've deleted a group-closing symbol because it seems to be",
            "spurious, as in `$x}$'. But perhaps the } is legitimate and",
            "you forgot something else, as in `\\hbox{$x}'. In such cases",
            "the way to recover is to insert both the forgotten and the",
            "deleted material, e.g., by typing `I$}'.",
        ],
    )?;
    Ok(())
}

fn start_paragraph_before_replaying_character(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    // TeX82 backs up the triggering token before `new_graf`, whose `every_par`
    // replay must therefore run before that first character is reconsidered.
    push_traced_tokens(input, stores, [traced]);
    assignments::ensure_horizontal_for_character(nest, input, stores, execution.command_fuel())
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

pub(crate) fn leave_group(
    input: &mut InputStack,
    stores: &mut Universe,
    expected: GroupKind,
) -> Result<(), ExecError> {
    leave_group_with_origin(input, stores, expected, OriginId::UNKNOWN)
}

pub(crate) fn leave_group_with_origin(
    input: &mut InputStack,
    stores: &mut Universe,
    expected: GroupKind,
    origin: OriginId,
) -> Result<(), ExecError> {
    match stores.leave_group_with_kind(expected) {
        Ok(tokens) => {
            let tokens: Vec<_> = tokens
                .into_iter()
                .map(|spelling| {
                    let token = spelling.semantic_token();
                    let inserted = stores.inserted_origin(
                        InsertedOriginKind::AfterGroup,
                        token,
                        if origin == OriginId::UNKNOWN {
                            spelling.origin()
                        } else {
                            origin
                        },
                    );
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
        (GroupKind::Simple, actual, false) if is_scanner_owned_group(actual) => {
            ExecError::ExtraRightBraceOrForgottenDollar { origin }
        }
        (GroupKind::SemiSimple, _, true) => ExecError::ExtraEndGroup { origin },
        (expected, _, true) if is_scanner_owned_group(expected) => ExecError::EndGroupMismatch {
            started_by: "the outer level",
            origin,
        },
        (expected, _, false) if is_scanner_owned_group(expected) => ExecError::EndGroupMismatch {
            started_by: mismatch.actual().start_text(),
            origin,
        },
        (GroupKind::SemiSimple, _, false) => ExecError::EndGroupMismatch {
            started_by: mismatch.actual().start_text(),
            origin,
        },
        (GroupKind::MathShift, _, true) => ExecError::MathShiftGroupMismatch {
            started_by: "the outer level",
            origin,
        },
        (GroupKind::MathShift, _, false) => ExecError::MathShiftGroupMismatch {
            started_by: mismatch.actual().start_text(),
            origin,
        },
        (GroupKind::Simple, GroupKind::Simple, false) => {
            unreachable!("matching group kinds are returned as successful leaves, not mismatches")
        }
        (GroupKind::Align, _, _) => ExecError::EndGroupMismatch {
            started_by: mismatch.actual().start_text(),
            origin,
        },
        _ => ExecError::EndGroupMismatch {
            started_by: mismatch.actual().start_text(),
            origin,
        },
    }
}

fn is_scanner_owned_group(kind: GroupKind) -> bool {
    !matches!(
        kind,
        GroupKind::Simple | GroupKind::SemiSimple | GroupKind::MathShift | GroupKind::Align
    )
}

pub(crate) fn push_tokens<I>(input: &mut InputStack, stores: &mut Universe, tokens: I)
where
    I: IntoIterator<Item = Token>,
{
    let tokens: Vec<_> = tokens.into_iter().collect();
    if tokens.is_empty() {
        return;
    }
    let token_list = stores.intern_token_list(&tokens);
    input.push_token_list(token_list, tex_state::TokenListReplayKind::Inserted);
}

pub(crate) fn push_traced_tokens<I>(input: &mut InputStack, _stores: &mut Universe, tokens: I)
where
    I: IntoIterator<Item = TracedTokenWord>,
{
    input.back_input(tokens);
}

pub(crate) fn insert_traced_tokens<I>(input: &mut InputStack, _stores: &mut Universe, tokens: I)
where
    I: IntoIterator<Item = TracedTokenWord>,
{
    let mut traced = input.take_transient_token_buffer();
    traced.extend(tokens);
    if traced.is_empty() {
        input.recycle_transient_token_buffer(traced);
        return;
    }
    input.push_transient_tokens(traced, tex_state::TokenListReplayKind::Inserted);
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
            Token::Char { ch, cat } => tex_command::character_command_text(ch, cat),
            Token::Param(slot) => format!("#{slot}"),
            Token::Frozen(_) => "\\endtemplate".to_owned(),
        }
    }
}
