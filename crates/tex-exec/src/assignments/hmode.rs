use tex_expand::get_x_token_with_context;
use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::Order;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::{DiscKind, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{ExpansionState, Universe};
use tex_typeset::{INF_BAD, PackSpec, VpackParams};

use super::paragraph::{end_paragraph_with_fuel, ensure_horizontal_for_character};
use super::*;
use crate::canonical_box_runtime::hmode::*;
use crate::canonical_paragraph_end::normal_paragraph;
use crate::legacy_dispatch::dispatch_delivered_token_with_context;
use crate::packing_params::vpack;
use crate::vertical::{append_vertical_contribution, build_page_if_outer_vertical};
use crate::{DispatchAction, ExecError, Mode, ModeNest, push_traced_tokens};

pub(crate) fn append_given_char(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    ch: char,
    origin: OriginId,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    match nest.current_mode() {
        Mode::RestrictedHorizontal | Mode::Horizontal => {
            append_hchar_with_fuel(nest, stores, ch, origin, false, fuel)
        }
        Mode::Vertical | Mode::InternalVertical => {
            ensure_horizontal_for_character(nest, input, stores, fuel)?;
            append_hchar_with_fuel(nest, stores, ch, origin, false, fuel)
        }
        mode => Err(ExecError::UnimplementedTypesetting {
            mode,
            token: Token::Char {
                ch,
                cat: Catcode::Other,
            },
            origin: OriginId::UNKNOWN,
            operation: "character",
        }),
    }
}

fn append_control_space(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        ensure_horizontal_for_character(nest, input, stores, fuel)?;
    }
    append_control_space_glue(nest, stores, fuel)
}

/// Appends a character after canonical main control has already selected
/// horizontal mode.  Keeping this small entry point here preserves the one
/// ligature/space-factor implementation while ensuring canonical replay has
/// no `InputStack` fallback.
pub(super) fn execute_hmode_material(
    context: TracedTokenWord,
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    match primitive {
        UnexpandablePrimitive::Char => {
            let value = scan_i32(input, stores, execution, context)?;
            let ch = char::from_u32(value as u32).ok_or(ExecError::InvalidCode {
                context: "\\char",
                value,
            })?;
            append_given_char(
                nest,
                input,
                stores,
                ch,
                context.origin(),
                execution.command_fuel(),
            )?;
        }
        UnexpandablePrimitive::HFil
        | UnexpandablePrimitive::HFill
        | UnexpandablePrimitive::HSs
        | UnexpandablePrimitive::HFilNeg => {
            flush_pending_hchars(nest, stores, execution.command_fuel())?;
            let spec = match primitive {
                UnexpandablePrimitive::HFil => infinite_glue(Order::Fil, false, false),
                UnexpandablePrimitive::HFill => infinite_glue(Order::Fill, false, false),
                UnexpandablePrimitive::HSs => infinite_glue(Order::Fil, false, true),
                UnexpandablePrimitive::HFilNeg => infinite_glue(Order::Fil, true, false),
                _ => unreachable!(),
            };
            let spec = stores.intern_glue(spec);
            nest.current_list_mutation().push(Node::Glue {
                spec,
                kind: GlueKind::Normal,
                leader: None,
            });
        }
        UnexpandablePrimitive::Penalty => {
            flush_pending_hchars(nest, stores, execution.command_fuel())?;
            let penalty = scan_i32(input, stores, execution, context)?;
            append_vertical_contribution(nest, stores, Node::Penalty(penalty));
            build_page_if_outer_vertical(nest, stores)?;
        }
        UnexpandablePrimitive::VRule => {
            flush_pending_hchars(nest, stores, execution.command_fuel())?;
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                ensure_horizontal_for_character(nest, input, stores, execution.command_fuel())?;
            }
            nest.current_list_mutation().push(scan_rule_node(
                input, stores, execution, primitive, context,
            )?);
            nest.current_list_mutation().set_space_factor(1000);
        }
        UnexpandablePrimitive::ControlSpace => {
            append_control_space(nest, input, stores, execution.command_fuel())?
        }
        UnexpandablePrimitive::ItalicCorrection => {
            append_italic_correction_with_fuel(nest, stores, execution.command_fuel())?
        }
        UnexpandablePrimitive::Discretionary => {
            let math_mode = matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath);
            flush_pending_hchars(nest, stores, execution.command_fuel())?;
            let pre = scan_hlist_group(input, stores, execution, "\\discretionary pre")?;
            let post = scan_hlist_group(input, stores, execution, "\\discretionary post")?;
            let mut replace =
                scan_hlist_group(input, stores, execution, "\\discretionary replace")?;
            if math_mode && !stores.nodes(replace).is_empty() {
                // TeX.web §1120 deletes the third list and reports; the
                // primitive name comes from `print_esc` so `\escapechar`
                // still governs it.
                let report_context = crate::diagnostics::show_context(stores, &input.summary());
                let mut report = stores.print_err("Illegal math ");
                report
                    .print_esc("discretionary")
                    .help(&[
                        "Sorry: The third part of a discretionary break must be",
                        "empty, in math formulas. I had to delete your third part.",
                    ])
                    .context(report_context);
                report.error().jump_out()?;
                replace = stores.freeze_node_list(&[]);
            }
            nest.current_list_mutation().push(Node::Disc {
                kind: DiscKind::Discretionary,
                pre,
                post,
                replace,
                physical_replace_count: stores
                    .nodes(replace)
                    .len()
                    .try_into()
                    .expect("TeX discretionary replacement count fits a quarterword"),
            });
        }
        UnexpandablePrimitive::DiscretionaryHyphen => {
            flush_pending_hchars(nest, stores, execution.command_fuel())?;
            let font = stores.current_font();
            let pre = match u8::try_from(stores.font_hyphen_char(font)) {
                Ok(hyphen) => stores.freeze_node_list(&[Node::Char {
                    font,
                    ch: char::from(hyphen),
                    origin: context.origin(),
                }]),
                Err(_) => stores.freeze_node_list(&[]),
            };
            let empty = stores.freeze_node_list(&[]);
            nest.current_list_mutation().push(Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                pre,
                post: empty,
                replace: empty,
                physical_replace_count: 0,
            });
        }
        UnexpandablePrimitive::NoBoundary => nest.current_list_mutation().set_no_boundary(true),
        UnexpandablePrimitive::SpaceFactor => {
            skip_optional_equals_x(input, stores, execution)?;
            let value = scan_i32(input, stores, execution, context)?;
            if !(1..=32767).contains(&value) {
                // TeX.web §1243 rejects the value with §91's `int_error` and
                // leaves the space factor untouched.
                let report_context = crate::diagnostics::show_context(stores, &input.summary());
                let mut report = stores.print_err("Bad space factor");
                report
                    .help(&["I allow only values in the range 1..32767 here."])
                    .context(report_context);
                report.int_error(value).jump_out()?;
            } else {
                nest.current_list_mutation().set_space_factor(value);
            }
        }
        UnexpandablePrimitive::Accent => {
            execute_accent(nest, input, stores, execution, context)?;
        }
        UnexpandablePrimitive::Mark | UnexpandablePrimitive::Marks => {
            flush_pending_hchars(nest, stores, execution.command_fuel())?;
            let class = if primitive == UnexpandablePrimitive::Marks {
                let value = scan_i32(input, stores, execution, context)?;
                if (0..=32_767).contains(&value) {
                    value as u16
                } else {
                    stores.report_bad_register_code(value, 32_767);
                    0
                }
            } else {
                0
            };
            let tokens = scan_general_text_expanded_with_driver(
                input,
                &mut tex_state::ExpansionContext::new(stores),
                execution,
                context,
            )?;
            append_vertical_contribution(nest, stores, Node::Mark { class, tokens });
        }
        UnexpandablePrimitive::VAdjust => execute_vadjust(nest, input, stores, execution)?,
        UnexpandablePrimitive::Insert => execute_insert(nest, input, stores, execution, context)?,
        _ => unreachable!("caller restricts hmode material primitives"),
    }
    Ok(())
}

fn execute_insert(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<(), ExecError> {
    // TeX's character loop finishes the pending run before main control
    // reswitches to `ital_corr`. Preserve that ordering: boundary processing
    // may leave a kern at the tail, and §1113 deliberately does nothing
    // unless the post-flush tail itself is a character or ligature.
    flush_pending_hchars(nest, stores, execution.command_fuel())?;
    let mut value = scan_i32(input, stores, execution, context)?;
    if !(0..=255).contains(&value) {
        return Err(ExecError::InvalidCode {
            context: "\\insert",
            value,
        });
    }
    if value == 255 {
        // TeX.web §1099 reserves box 255 for the output routine and silently
        // redirects the insertion to class 0 after reporting.
        let report_context = crate::diagnostics::show_context(stores, &input.summary());
        let mut report = stores.print_err("You can't ");
        report
            .print_esc("insert")
            .print_int(255)
            .help(&["I'm changing to \\insert0; box 255 is special."])
            .context(report_context);
        report.error().jump_out()?;
        value = 0;
    }
    let opener =
        next_non_space_traced_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
            context: "\\insert group",
        })?;
    if !has_catcode_meaning(stores, opener.semantic_token(), Catcode::BeginGroup) {
        return Err(ExecError::MissingToken {
            context: "\\insert group",
        });
    }

    stores.enter_group_with_kind(tex_state::GroupKind::Insert);
    let box_group_depth = stores.execution_group_depth();
    let mut inner = ModeNest::new();
    inner.push(Mode::InternalVertical)?;
    normal_paragraph(&mut inner, stores);
    scan_box_group(&mut inner, input, stores, execution, box_group_depth)?;
    if inner.current_mode() == Mode::Horizontal {
        end_paragraph_with_fuel(&mut inner, stores, execution.command_fuel())?;
    }
    let level =
        crate::assignments::commit_current_list(&mut inner, stores, execution.command_fuel())?;
    let content = stores.freeze_node_list(level.list().nodes());
    let packed = vpack(
        stores,
        content,
        PackSpec::Natural,
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: Scaled::MAX_DIMEN,
            box_max_depth: Scaled::MAX_DIMEN,
        },
    );
    let size = packed
        .node
        .height
        .checked_add(packed.node.depth)
        .ok_or(ExecError::ArithmeticOverflow)?;
    let split_top_skip = stores.glue_param(GlueParam::SPLIT_TOP_SKIP);
    let split_max_depth = stores.dimen_param(DimenParam::SPLIT_MAX_DEPTH);
    let floating_penalty = stores.int_param(IntParam::FLOATING_PENALTY);

    crate::leave_group(input, stores, tex_state::GroupKind::Insert)?;
    execution.paragraph_group_exited(stores);

    append_vertical_contribution(
        nest,
        stores,
        Node::Ins {
            class: value as u16,
            size,
            split_top_skip,
            split_max_depth,
            floating_penalty,
            content,
        },
    );
    build_page_if_outer_vertical(nest, stores)?;
    Ok(())
}

fn execute_vadjust(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    if !matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath
    ) {
        return Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: Token::Cs(stores.intern("vadjust").symbol()),
            origin: OriginId::UNKNOWN,
            operation: "\\vadjust",
        });
    }
    if matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) {
        flush_pending_hchars(nest, stores, execution.command_fuel())?;
    }
    let opener = next_non_space_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
        context: "\\vadjust group",
    })?;
    if !is_begin_group(opener) {
        return Err(ExecError::MissingToken {
            context: "\\vadjust group",
        });
    }
    stores.enter_group_with_kind(tex_state::GroupKind::AdjustedHBox);
    let box_group_depth = stores.execution_group_depth();
    let mut inner = ModeNest::new();
    inner.push(Mode::InternalVertical)?;
    normal_paragraph(&mut inner, stores);
    scan_box_group(&mut inner, input, stores, execution, box_group_depth)?;
    if inner.current_mode() == Mode::Horizontal {
        end_paragraph_with_fuel(&mut inner, stores, execution.command_fuel())?;
    }
    let level =
        crate::assignments::commit_current_list(&mut inner, stores, execution.command_fuel())?;
    let content = stores.freeze_node_list(level.list().nodes());
    crate::leave_group(input, stores, tex_state::GroupKind::AdjustedHBox)?;
    execution.paragraph_group_exited(stores);
    nest.current_list_mutation()
        .push(Node::Adjust(tex_state::node::AdjustNode {
            content,
            pre: false,
        }));
    Ok(())
}

fn execute_accent(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores, execution.command_fuel())?;
    let accent_value = scan_i32(input, stores, execution, context)?;
    let accent = u8::try_from(accent_value).map_err(|_| ExecError::InvalidCode {
        context: "\\accent",
        value: accent_value,
    })?;
    let accent_font = stores.current_font();
    let Some(accent_metrics) = stores.font_char_metrics(accent_font, accent) else {
        crate::diagnostics::report_missing_character_warning(
            stores,
            accent_font,
            char::from(accent),
            false,
        );
        return Ok(());
    };
    let base = scan_accent_base(nest, input, stores, execution, context)?;
    let Some(base) = base else {
        nest.current_list_mutation().push(Node::Char {
            font: accent_font,
            ch: char::from(accent),
            origin: context.origin(),
        });
        return Ok(());
    };
    let base_font = stores.current_font();
    let Some(base_metrics) = stores.font_char_metrics(base_font, base) else {
        crate::diagnostics::report_missing_character_warning(
            stores,
            base_font,
            char::from(base),
            false,
        );
        nest.current_list_mutation().push(Node::Char {
            font: accent_font,
            ch: char::from(accent),
            origin: context.origin(),
        });
        nest.current_list_mutation().set_space_factor(1000);
        return Ok(());
    };
    let accent_x_height = stores.font_parameter(accent_font, 5);
    let accent_slant = stores.font_parameter(accent_font, 1);
    let base_slant = stores.font_parameter(base_font, 1);
    let delta = tex_state::scaled::text_accent_delta(
        base_metrics.width,
        accent_metrics.width,
        base_metrics.height,
        base_slant,
        accent_x_height,
        accent_slant,
    );
    nest.current_list_mutation().push(Node::Kern {
        amount: delta,
        kind: KernKind::Accent,
    });
    let accent_node = Node::Char {
        font: accent_font,
        ch: char::from(accent),
        origin: context.origin(),
    };
    if base_metrics.height == accent_x_height {
        nest.current_list_mutation().push(accent_node);
    } else {
        let children = stores.freeze_node_list(&[accent_node]);
        let mut boxed = crate::canonical_box_runtime::hpack_with_overfull_rule(
            stores,
            children,
            PackSpec::Natural,
        );
        boxed.shift = accent_x_height
            .checked_sub(base_metrics.height)
            .ok_or(ExecError::ArithmeticOverflow)?;
        nest.current_list_mutation().push(Node::HList(boxed));
    }
    let back = Scaled::from_raw(-accent_metrics.width.raw() - delta.raw());
    nest.current_list_mutation().push(Node::Kern {
        amount: back,
        kind: KernKind::Accent,
    });
    nest.current_list_mutation().push(Node::Char {
        font: base_font,
        ch: char::from(base),
        origin: context.origin(),
    });
    nest.current_list_mutation().set_space_factor(1000);
    Ok(())
}

fn scan_accent_base(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<Option<u8>, ExecError> {
    loop {
        let Some(traced) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            return Ok(None);
        };
        let token = traced.semantic_token();
        if is_space(token) {
            continue;
        }
        let meaning = match token {
            Token::Cs(symbol) => Some(stores.meaning(symbol)),
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                let symbol = active_character_symbol(stores, ch);
                Some(stores.meaning(symbol))
            }
            Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => None,
        };
        if meaning == Some(Meaning::Relax) {
            continue;
        }
        if meaning.is_some_and(is_accent_assignment_meaning) {
            match dispatch_delivered_token_with_context(nest, traced, input, stores, execution)? {
                DispatchAction::Continue => continue,
                DispatchAction::End | DispatchAction::Shipout(_) | DispatchAction::NotConsumed => {
                    unreachable!("TeX82 do_assignments only dispatches ordinary assignments")
                }
            }
        }
        let ch = match (token, meaning) {
            (
                Token::Char {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                },
                _,
            )
            | (_, Some(Meaning::CharGiven(ch)))
            | (
                _,
                Some(Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                }),
            ) => ch,
            (_, Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char))) => {
                let value = scan_i32(input, stores, execution, context)?;
                let ch = u8::try_from(value).map_err(|_| ExecError::InvalidCode {
                    context: "\\accent base",
                    value,
                })?;
                return Ok(Some(ch));
            }
            _ => {
                push_traced_tokens(input, stores, [traced]);
                return Ok(None);
            }
        };
        return u8::try_from(ch as u32)
            .map(Some)
            .map_err(|_| ExecError::InvalidCode {
                context: "\\accent base",
                value: ch as i32,
            });
    }
}

fn is_accent_assignment_meaning(meaning: Meaning) -> bool {
    if matches!(meaning, Meaning::Font(_)) {
        return true;
    }
    if !is_assignment_meaning(meaning) {
        return false;
    }
    !matches!(
        meaning,
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::BeginGroup
                | UnexpandablePrimitive::EndGroup
                | UnexpandablePrimitive::AfterGroup
                | UnexpandablePrimitive::AfterAssignment
                | UnexpandablePrimitive::OpenIn
                | UnexpandablePrimitive::CloseIn
                | UnexpandablePrimitive::OpenOut
                | UnexpandablePrimitive::CloseOut
                | UnexpandablePrimitive::Immediate
                | UnexpandablePrimitive::Write
        )
    )
}

pub(crate) fn scan_rule_node(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
) -> Result<Node, ExecError> {
    let default_rule = Scaled::from_raw(26_214);
    let (mut width, mut height, mut depth) = if primitive == UnexpandablePrimitive::VRule {
        (Some(default_rule), None, None)
    } else {
        (None, Some(default_rule), Some(Scaled::from_raw(0)))
    };
    loop {
        if scan_optional_keyword_x(input, stores, execution, "width")? {
            width = Some(scan_scaled(input, stores, execution, context)?);
        } else if scan_optional_keyword_x(input, stores, execution, "height")? {
            height = Some(scan_scaled(input, stores, execution, context)?);
        } else if scan_optional_keyword_x(input, stores, execution, "depth")? {
            depth = Some(scan_scaled(input, stores, execution, context)?);
        } else {
            break;
        }
    }
    Ok(Node::Rule {
        width,
        height,
        depth,
    })
}

fn scan_hlist_group(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: &'static str,
) -> Result<tex_state::ids::NodeListId, ExecError> {
    let opener =
        next_non_space_x(input, stores, execution)?.ok_or(ExecError::MissingToken { context })?;
    if !is_begin_group(opener) {
        return Err(ExecError::MissingToken { context });
    }
    stores.enter_group_with_kind(tex_state::GroupKind::Disc);
    let mut inner = ModeNest::new();
    inner.push(Mode::RestrictedHorizontal)?;
    let box_group_depth = stores.execution_group_depth();
    scan_box_group(&mut inner, input, stores, execution, box_group_depth)?;
    let level =
        crate::assignments::commit_current_list(&mut inner, stores, execution.command_fuel())?;
    let nodes = stores.freeze_node_list(level.list().nodes());
    crate::leave_group(input, stores, tex_state::GroupKind::Disc)?;
    execution.paragraph_group_exited(stores);
    Ok(nodes)
}

/// TeX82 §1113's `append_italic_correction` (`hmode+ital_corr`). Shared by
/// the legacy dispatcher and canonical main control's
/// `ScannedStep::ItalicCorrection` handler.
///
/// tex.web appends the italic-correction kern unconditionally whenever the
/// tail is a character or ligature node -- including when the correction
/// happens to be exactly zero (`tail_append(new_kern(char_italic(...)))`
/// runs with no guard on the resulting width). Only an empty list, or a tail
/// that is neither a character nor a ligature, leaves the list untouched
/// (`return` with no append).
#[cfg(any())]
mod tests;
