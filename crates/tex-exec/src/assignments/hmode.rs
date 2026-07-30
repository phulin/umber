use tex_expand::get_x_token_with_context;
use tex_fonts::{LigKernChar, LigKernCommand};
use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::FontId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::{DiscKind, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{ExpansionState, Universe};
use tex_typeset::{INF_BAD, PackSpec, VpackParams};

use super::paragraph::{
    end_paragraph_with_fuel, ensure_horizontal_for_character, normal_paragraph,
};
use super::*;
use crate::dispatch::dispatch_delivered_token_with_context;
use crate::mode::{PendingHRun, PendingHRunChar};
use crate::packing_params::vpack;
use crate::vertical::{append_vertical_contribution, build_page_if_outer_vertical};
use crate::{DispatchAction, ExecError, Mode, ModeNest, push_traced_tokens};

pub(crate) fn try_append_character(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<bool, ExecError> {
    let token = tex_expand::semantic_token(traced);
    match (nest.current_mode(), token) {
        (Mode::RestrictedHorizontal | Mode::Horizontal, Token::Char { ch, cat }) => {
            if cat == Catcode::Space {
                append_space(nest, stores, fuel)?;
            } else {
                append_hchar_with_fuel(nest, stores, ch, traced.origin(), fuel)?;
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Consumes a preclassified horizontal text span into the pending TFM machine.
/// OpenType runs retain their shaping-specific source collection and
/// deliberately use the scalar path.
pub(crate) fn try_append_tfm_character_span(
    nest: &mut ModeNest,
    traced: &[TracedTokenWord],
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<bool, ExecError> {
    let mode = nest.current_mode();
    if !matches!(mode, Mode::RestrictedHorizontal | Mode::Horizontal) {
        return Ok(false);
    }
    let font = stores.current_font();
    if is_ltr_shaping_font(stores, font) {
        return Ok(false);
    }

    let mut offset = 0;
    while offset < traced.len() {
        let Token::Char { cat, .. } = tex_expand::semantic_token(traced[offset]) else {
            unreachable!("preclassified horizontal text spans contain only character tokens")
        };
        if cat == Catcode::Space {
            append_space(nest, stores, fuel)?;
            offset += 1;
            continue;
        }
        fix_hyphen_language_with_fuel(nest, stores, mode, fuel)?;

        // A TFM run cannot continue an OpenType pending run. This is normally
        // a font-command boundary, but keeping the guard here makes this
        // entry point correct for any future span producer.
        if nest.current_list().pending_hchars().is_some_and(|pending| {
            pending.first.font != font && is_ltr_shaping_font(stores, pending.first.font)
        }) {
            flush_pending_hchar_run_with_fuel(nest, stores, mode == Mode::Horizontal, false, fuel)?;
        }

        let mut list = nest.current_list_mutation();
        let mut pending = list.take_pending_hchars();
        let mut space_factor = list.space_factor();
        while offset < traced.len() {
            let Token::Char { ch, cat } = tex_expand::semantic_token(traced[offset]) else {
                unreachable!("preclassified horizontal text spans contain only character tokens")
            };
            if cat == Catcode::Space {
                break;
            }
            if append_tfm_hchar(
                &mut pending,
                stores,
                font,
                ch,
                traced[offset].origin(),
                list.nodes().len(),
            ) {
                space_factor = next_space_factor(space_factor, stores, ch);
            }
            offset += 1;
        }
        if let Some(pending) = pending {
            list.set_pending_hchars(pending);
        }
        list.set_space_factor(space_factor);
    }
    Ok(true)
}

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
            append_hchar_with_fuel(nest, stores, ch, origin, fuel)
        }
        Mode::Vertical | Mode::InternalVertical => {
            ensure_horizontal_for_character(nest, input, stores, fuel)?;
            append_hchar_with_fuel(nest, stores, ch, origin, fuel)
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

/// Appends a character after canonical main control has already selected
/// horizontal mode.  Keeping this small entry point here preserves the one
/// ligature/space-factor implementation while ensuring canonical replay has
/// no `InputStack` fallback.
pub(crate) fn append_canonical_character_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    ch: char,
    origin: OriginId,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    debug_assert!(matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ));
    append_hchar_with_fuel(nest, stores, ch, origin, fuel)
}

#[cfg(test)]
pub(crate) fn append_canonical_character(
    nest: &mut ModeNest,
    stores: &mut Universe,
    ch: char,
    origin: OriginId,
) -> Result<(), ExecError> {
    let mut fuel = tex_command::CommandFuelLedger::default();
    append_canonical_character_with_fuel(nest, stores, ch, origin, fuel.fuel_mut())
}

/// Appends an ordinary space from canonical main control after horizontal
/// mode has been selected by TeX82 §1095.
pub(crate) fn append_canonical_space_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    debug_assert!(matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ));
    flush_pending_hchars_with_fuel(nest, stores, fuel)?;
    append_space_after_flush(nest, stores)
}

pub(crate) fn flush_pending_hchars(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let insert_hyphen_discs = nest.current_mode() == Mode::Horizontal;
    flush_pending_hchar_run_with_fuel(nest, stores, insert_hyphen_discs, false, fuel)
}

pub(crate) fn flush_pending_hchars_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores, fuel)
}

/// Flushes the active TeX82 §1038 character run after its lookahead consumed
/// `\noboundary`. This suppresses only the right boundary; a separate flag on
/// the list records §1030's left-boundary cancellation before a new run.
pub(crate) fn flush_pending_hchars_without_right_boundary(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let insert_hyphen_discs = nest.current_mode() == Mode::Horizontal;
    flush_pending_hchar_run_with_fuel(nest, stores, insert_hyphen_discs, true, fuel)
}

/// Closes the current list's mutable construction phase.
///
/// `ModeNest::pop` rejects a level that still owns a pending character run,
/// making this the only successful path from a live list to a packaged,
/// frozen, or otherwise detached list. Non-commit barriers can still call
/// [`flush_pending_hchars`] directly when TeX needs the run materialized but
/// must keep the mode level open.
pub(crate) fn commit_current_list(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<crate::mode::ModeLevelSummary, ExecError> {
    flush_pending_hchars(nest, stores, fuel)?;
    nest.pop()
}

fn flush_pending_hchar_run_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    insert_hyphen_discs: bool,
    suppress_right_boundary: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let Some(pending) = nest.current_list_mutation().take_pending_hchars() else {
        return Ok(());
    };
    if is_ltr_shaping_font(stores, pending.first.font) && is_supported_script(pending.script) {
        let language = nest.current_list().hyphen_language();
        let breaks = if insert_hyphen_discs {
            super::hyphenation::candidate_positions_for_chars(
                stores,
                language,
                &pending.source,
                stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(1) as usize,
                stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(1) as usize,
            )
        } else {
            Vec::new()
        };
        let shaped = shape_open_type_chars(stores, &pending.source, &breaks);
        let mut list = nest.current_list_mutation();
        list.set_no_boundary(false);
        list.append(shaped);
        return Ok(());
    }
    let no_boundary = nest.current_list().no_boundary();
    let nodes = match run_tfm_ligature_machine(
        stores,
        &pending.source,
        no_boundary,
        suppress_right_boundary,
        insert_hyphen_discs,
        fuel,
    ) {
        Ok(nodes) => nodes,
        Err(error) => {
            nest.current_list_mutation().set_pending_hchars(pending);
            return Err(ExecError::Command(error));
        }
    };
    let mut list = nest.current_list_mutation();
    list.set_no_boundary(false);
    list.append(nodes);
    Ok(())
}

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
                report.error();
                replace = stores.freeze_node_list(&[]);
            }
            nest.current_list_mutation().push(Node::Disc {
                kind: DiscKind::Discretionary,
                pre,
                post,
                replace,
            });
        }
        UnexpandablePrimitive::DiscretionaryHyphen => {
            flush_pending_hchars(nest, stores, execution.command_fuel())?;
            let font = stores.current_font();
            let hyphen = u8::try_from(stores.font_hyphen_char(font))
                .ok()
                .map(char::from)
                .unwrap_or('-');
            let pre = stores.freeze_node_list(&[Node::Char {
                font,
                ch: hyphen,
                origin: context.origin(),
            }]);
            let empty = stores.freeze_node_list(&[]);
            nest.current_list_mutation().push(Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                pre,
                post: empty,
                replace: empty,
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
                report.int_error(value);
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
        report.error();
        value = 0;
    }
    let opener =
        next_non_space_traced_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
            context: "\\insert group",
        })?;
    if !has_catcode_meaning(
        stores,
        tex_expand::semantic_token(opener),
        Catcode::BeginGroup,
    ) {
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
    nest.current_list_mutation().push(Node::Adjust(content));
    Ok(())
}

fn append_space(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores, fuel)?;
    append_space_after_flush(nest, stores)
}

fn append_space_after_flush(nest: &mut ModeNest, stores: &mut Universe) -> Result<(), ExecError> {
    let configuration = stores.pdf_font_configuration();
    let sf = if configuration.adjusts_interword_glue() {
        1000
    } else {
        nest.current_list().space_factor()
    };
    let mut spec = if sf >= 2000 {
        nonzero_glue_param_or_font_space(stores, GlueParam::XSPACE_SKIP, sf)
    } else {
        nonzero_glue_param_or_font_space(stores, GlueParam::SPACE_SKIP, sf)
    };
    if configuration.adjusts_interword_glue() {
        adjust_interword_glue(stores, nest.current_list().nodes(), &mut spec);
    }
    let id = stores.intern_glue(spec);
    nest.current_list_mutation().push(Node::Glue {
        spec: id,
        kind: GlueKind::Normal,
        leader: None,
    });
    Ok(())
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

/// Appends the explicit `\ ` control-space glue after horizontal mode has
/// already been selected. TeX82 §1030's `hmode+ex_space,mmode+ex_space: goto
/// append_normal_space` always takes the space-factor-1000 branch, unlike an
/// ordinary `spacer` token, which only reaches `append_normal_space` when
/// `space_factor=1000` and otherwise scales the glue through `app_space`
/// (§1042). This is shared by the legacy `InputStack`-driven dispatch above
/// and canonical main control's mode-switch-then-append split below.
fn append_control_space_glue(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores, fuel)?;
    append_control_space_glue_after_flush(nest, stores)
}

fn append_control_space_glue_after_flush(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let mut spec = nonzero_glue_param_or_font_space(stores, GlueParam::SPACE_SKIP, 1000);
    if stores.pdf_font_configuration().adjusts_interword_glue() {
        adjust_interword_glue(stores, nest.current_list().nodes(), &mut spec);
    }
    let id = stores.intern_glue(spec);
    nest.current_list_mutation().push(Node::Glue {
        spec: id,
        kind: GlueKind::Normal,
        leader: None,
    });
    Ok(())
}

/// Appends the explicit `\ ` control-space glue from canonical main control
/// after TeX82 §1090's vertical-mode paragraph start (if any) has already run.
/// Mirrors `append_canonical_space`'s split from `append_space` above.
pub(crate) fn append_canonical_control_space_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    debug_assert!(matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ));
    flush_pending_hchars_with_fuel(nest, stores, fuel)?;
    append_control_space_glue_after_flush(nest, stores)
}

/// The `\ ` glue specification for TeX82 §1041's `append_normal_space` when
/// used from math mode (`mmode+ex_space`, §1030), which has no pending
/// ligature run or pdfTeX interword-glue adjustment to consider -- those are
/// exclusively horizontal-list concerns. Callers push the returned spec
/// directly onto the current (math) list.
pub(crate) fn control_space_glue_spec(stores: &Universe) -> GlueSpec {
    nonzero_glue_param_or_font_space(stores, GlueParam::SPACE_SKIP, 1000)
}

fn append_hchar_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    ch: char,
    origin: OriginId,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let mode = nest.current_mode();
    fix_hyphen_language_with_fuel(nest, stores, mode, fuel)?;
    let font = stores.current_font();
    let (character_exists, font_is_ltr_shaping) = {
        let loaded = stores.font(font);
        (
            loaded.character_exists(ch),
            loaded.shaping_font().is_some()
                && loaded.shaping_direction() == Some(tex_fonts::WritingDirection::LeftToRight),
        )
    };
    let false_boundary_character = font_code(ch)
        .ok()
        .is_some_and(|code| stores.font_false_boundary_char(font) == Some(code));
    if character_exists || false_boundary_character {
        let flush_incompatible_run = nest.current_list().pending_hchars().is_some_and(|pending| {
            (font_is_ltr_shaping
                || (pending.first.font != font && is_ltr_shaping_font(stores, pending.first.font)))
                && (pending.first.font != font
                    || !scripts_compatible(pending.script, tex_shape::character_script(ch)))
        });
        if flush_incompatible_run {
            let insert_hyphen_discs = mode == Mode::Horizontal;
            flush_pending_hchar_run_with_fuel(nest, stores, insert_hyphen_discs, false, fuel)?;
        }
        let mut list = nest.current_list_mutation();
        append_pending_hchar(
            &mut list,
            stores,
            mode,
            font,
            font_is_ltr_shaping,
            ch,
            origin,
        );
        update_space_factor(&mut list, stores, ch);
        return Ok(());
    }
    report_missing_character(stores, font, ch);
    Ok(())
}

#[cfg(test)]
fn append_hchar(
    nest: &mut ModeNest,
    stores: &mut Universe,
    ch: char,
    origin: OriginId,
) -> Result<(), ExecError> {
    append_hchar_with_fuel(
        nest,
        stores,
        ch,
        origin,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
}

#[cfg(test)]
pub(crate) fn test_fix_hyphen_language(nest: &mut ModeNest, stores: &mut Universe, mode: Mode) {
    fix_hyphen_language(nest, stores, mode).expect("test ligature run is fueled");
}

/// TeX82 §1091's `norm_min`, verbatim: `if h<=0 then norm_min:=1 else if
/// h>=63 then norm_min:=63 else norm_min:=h`.
///
/// tex.web states this clamp once and applies it at every site that stores a
/// hyphen minimum in a fixed-width field: §1091's and §1200's `prev_graf`
/// packing, §1376's `fix_language`, and §1377's `\setlanguage`. It lives
/// here so all of them read the same function rather than each transcribing
/// the bounds.
pub(crate) const fn norm_min(value: i32) -> u8 {
    if value <= 0 {
        1
    } else if value >= 63 {
        63
    } else {
        value as u8
    }
}

#[cfg(test)]
fn fix_hyphen_language(
    nest: &mut ModeNest,
    stores: &mut Universe,
    mode: Mode,
) -> Result<(), ExecError> {
    let mut fuel = tex_command::CommandFuelLedger::default();
    fix_hyphen_language_with_fuel(nest, stores, mode, fuel.fuel_mut())
}

fn fix_hyphen_language_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    mode: Mode,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    if mode != Mode::Horizontal {
        return Ok(());
    }
    let language = u8::try_from(stores.int_param(IntParam::LANGUAGE)).unwrap_or(0);
    if language == nest.current_list().hyphen_language() {
        return Ok(());
    }
    // tex.web's fix_language flushes the current ligature word before
    // recording the new language and its current hyphen minima.
    flush_pending_hchar_run_with_fuel(nest, stores, true, false, fuel)?;
    let left_hyphen_min = norm_min(stores.int_param(IntParam::LEFT_HYPHEN_MIN));
    let right_hyphen_min = norm_min(stores.int_param(IntParam::RIGHT_HYPHEN_MIN));
    nest.current_list_mutation()
        .push(Node::Whatsit(tex_state::node::Whatsit::Language {
            language,
            left_hyphen_min,
            right_hyphen_min,
        }));
    nest.current_list_mutation().set_hyphen_language(language);
    Ok(())
}

fn append_pending_hchar(
    list: &mut crate::mode::ModeListMutation<'_>,
    _stores: &mut Universe,
    _mode: Mode,
    font: FontId,
    font_is_ltr_shaping: bool,
    ch: char,
    origin: OriginId,
) {
    let Some(mut pending) = list.take_pending_hchars() else {
        list.begin_pending_hchars(font, ch, origin);
        return;
    };
    if font_is_ltr_shaping
        && is_supported_script(pending.script)
        && is_supported_script(tex_shape::character_script(ch))
    {
        let script = tex_shape::character_script(ch);
        if is_strong_script(script) {
            pending.script = script;
        }
        pending
            .source
            .push(crate::mode::PendingHChar { font, ch, origin });
        pending.current = PendingHRunChar::new(font, ch, origin);
        list.set_pending_hchars(pending);
        return;
    }
    pending
        .source
        .push(crate::mode::PendingHChar { font, ch, origin });
    pending.current = PendingHRunChar::new(font, ch, origin);
    list.set_pending_hchars(pending);
}

fn append_tfm_hchar(
    pending: &mut Option<PendingHRun>,
    stores: &mut Universe,
    font: FontId,
    ch: char,
    origin: OriginId,
    insertion_index: usize,
) -> bool {
    if !stores.font(font).character_exists(ch)
        && font_code(ch)
            .ok()
            .is_none_or(|code| stores.font_false_boundary_char(font) != Some(code))
    {
        report_missing_character(stores, font, ch);
        return false;
    }
    let Some(mut current_run) = pending.take() else {
        *pending = Some(PendingHRun::new(font, ch, origin, insertion_index));
        return true;
    };
    current_run
        .source
        .push(crate::mode::PendingHChar { font, ch, origin });
    current_run.current = PendingHRunChar::new(font, ch, origin);
    *pending = Some(current_run);
    true
}

fn is_strong_script(script: tex_shape::Script) -> bool {
    !matches!(
        script,
        tex_shape::Script::Common | tex_shape::Script::Inherited | tex_shape::Script::Unknown
    )
}

fn scripts_compatible(left: tex_shape::Script, right: tex_shape::Script) -> bool {
    !is_strong_script(left) || !is_strong_script(right) || left == right
}

fn is_supported_script(script: tex_shape::Script) -> bool {
    matches!(
        script,
        tex_shape::Script::Common
            | tex_shape::Script::Inherited
            | tex_shape::Script::Latin
            | tex_shape::Script::Cyrillic
            | tex_shape::Script::Greek
            | tex_shape::Script::Han
            | tex_shape::Script::Hiragana
            | tex_shape::Script::Katakana
            | tex_shape::Script::Hangul
            | tex_shape::Script::Bopomofo
    )
}

fn is_ltr_shaping_font(stores: &Universe, font: FontId) -> bool {
    let font = stores.font(font);
    font.shaping_font().is_some()
        && font.shaping_direction() == Some(tex_fonts::WritingDirection::LeftToRight)
}

fn shape_open_type_chars(
    stores: &Universe,
    chars: &[crate::mode::PendingHChar],
    break_positions: &[usize],
) -> Vec<Node> {
    use std::collections::BTreeMap;

    let Some(first) = chars.first() else {
        return Vec::new();
    };
    let font = stores.font(first.font);
    let shaping_font = font.shaping_font().expect("OpenType run font");
    let features = font.shaping_features().expect("OpenType feature policy");
    let mut text = String::new();
    let mut byte_starts = Vec::with_capacity(chars.len());
    for entry in chars {
        byte_starts.push(text.len());
        if let Some(mapped) = font.mapped_text(entry.ch) {
            text.push_str(mapped);
        } else {
            text.push(entry.ch);
        }
    }
    let break_bytes = break_positions
        .iter()
        .filter_map(|&position| byte_starts.get(position).copied())
        .collect::<Vec<_>>();
    let direction = match font.shaping_direction() {
        Some(tex_fonts::WritingDirection::RightToLeft) => tex_shape::Direction::RightToLeft,
        Some(tex_fonts::WritingDirection::LeftToRight) | None => tex_shape::Direction::LeftToRight,
    };
    let shaped = tex_shape::shape_run_with_breaks_and_context(
        shaping_font,
        &text,
        features,
        direction,
        font.shaping_script(),
        font.shaping_language(),
        &break_bytes,
    );
    let mut cluster_advances = BTreeMap::<usize, i64>::new();
    for glyph in shaped.glyphs {
        let cluster_byte = glyph.cluster as usize;
        let source_index = byte_starts
            .partition_point(|&start| start <= cluster_byte)
            .saturating_sub(1);
        *cluster_advances.entry(source_index).or_default() += i64::from(glyph.x_advance.raw());
    }
    let cluster_starts = cluster_advances.keys().copied().collect::<Vec<_>>();
    let mut adjustments = vec![Scaled::from_raw(0); chars.len()];
    for (cluster_index, &start) in cluster_starts.iter().enumerate() {
        let end = cluster_starts
            .get(cluster_index + 1)
            .copied()
            .unwrap_or(chars.len());
        if start >= end {
            continue;
        }
        let nominal = chars[start..end].iter().fold(0_i64, |sum, entry| {
            sum + i64::from(
                stores
                    .font_character_metrics(entry.font, entry.ch)
                    .map_or(0, |metrics| metrics.width.raw()),
            )
        });
        let shaped = cluster_advances[&start];
        adjustments[end - 1] = Scaled::from_raw(
            i32::try_from(shaped - nominal).expect("shaped cluster adjustment fits Scaled"),
        );
    }
    let mut nodes = Vec::with_capacity(chars.len() * 2);
    for (entry, adjustment) in chars.iter().zip(adjustments) {
        nodes.push(Node::Char {
            font: entry.font,
            ch: entry.ch,
            origin: entry.origin,
        });
        if adjustment.raw() != 0 {
            nodes.push(Node::Kern {
                amount: adjustment,
                kind: KernKind::Font,
            });
        }
    }
    nodes
}

/// Replaces provisional OpenType shaping adjustments in a materialized list.
///
/// Every call shapes caller-delimited runs independently. Paragraph code uses
/// this after break selection, which restores ligatures on each unsplit side
/// while preventing a glyph cluster from crossing the chosen line boundary.
pub(super) fn reshape_open_type_runs(stores: &Universe, nodes: &mut Vec<Node>) {
    let mut index = 0;
    while index < nodes.len() {
        let Node::Char { font, ch, origin } = nodes[index] else {
            index += 1;
            continue;
        };
        if !is_ltr_shaping_font(stores, font)
            || !is_supported_script(tex_shape::character_script(ch))
        {
            index += 1;
            continue;
        }
        let mut chars = vec![crate::mode::PendingHChar { font, ch, origin }];
        let mut script = tex_shape::character_script(ch);
        let start = index;
        index += 1;
        while index < nodes.len() {
            match nodes[index] {
                Node::Kern {
                    kind: KernKind::Font,
                    ..
                } => index += 1,
                Node::Char {
                    font: next_font,
                    ch: next_ch,
                    origin: next_origin,
                } if next_font == font
                    && scripts_compatible(script, tex_shape::character_script(next_ch)) =>
                {
                    let next_script = tex_shape::character_script(next_ch);
                    if is_strong_script(next_script) {
                        script = next_script;
                    }
                    chars.push(crate::mode::PendingHChar {
                        font,
                        ch: next_ch,
                        origin: next_origin,
                    });
                    index += 1;
                }
                _ => break,
            }
        }
        let shaped = shape_open_type_chars(stores, &chars, &[]);
        let shaped_len = shaped.len();
        nodes.splice(start..index, shaped);
        index = start + shaped_len;
    }
}

#[cfg(test)]
pub(crate) fn reconstitute(
    stores: &mut Universe,
    pending: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    insert_hyphen_discs: bool,
) -> Vec<Node> {
    let mut fuel = tex_command::CommandFuelLedger::default();
    reconstitute_with_fuel(
        stores,
        pending,
        no_left_boundary,
        insert_hyphen_discs,
        fuel.fuel_mut(),
    )
    .expect("test reconstruction fuel")
}

pub(crate) fn reconstitute_with_fuel(
    stores: &mut Universe,
    pending: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<Vec<Node>, tex_command::CommandError> {
    run_tfm_ligature_machine(
        stores,
        pending,
        no_left_boundary,
        false,
        insert_hyphen_discs,
        fuel,
    )
}

#[derive(Clone)]
enum LigatureWorkItem {
    Boundary,
    Glyph(PendingHRunChar),
    Kern { amount: Scaled, kind: KernKind },
}

#[derive(Clone)]
struct LigatureWorkNode {
    item: LigatureWorkItem,
    previous: Option<usize>,
    next: Option<usize>,
    discard_if_missing: bool,
}

struct LigatureWorkList {
    nodes: Vec<LigatureWorkNode>,
    head: Option<usize>,
    tail: Option<usize>,
}

impl LigatureWorkList {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            head: None,
            tail: None,
        }
    }

    fn push_back(&mut self, item: LigatureWorkItem) -> usize {
        let index = self.nodes.len();
        self.nodes.push(LigatureWorkNode {
            item,
            previous: self.tail,
            next: None,
            discard_if_missing: false,
        });
        if let Some(tail) = self.tail {
            self.nodes[tail].next = Some(index);
        } else {
            self.head = Some(index);
        }
        self.tail = Some(index);
        index
    }

    fn insert_after(&mut self, index: usize, item: LigatureWorkItem) -> usize {
        let next = self.nodes[index].next;
        let inserted = self.nodes.len();
        self.nodes.push(LigatureWorkNode {
            item,
            previous: Some(index),
            next,
            discard_if_missing: false,
        });
        self.nodes[index].next = Some(inserted);
        if let Some(next) = next {
            self.nodes[next].previous = Some(inserted);
        } else {
            self.tail = Some(inserted);
        }
        inserted
    }

    fn remove(&mut self, index: usize) {
        let previous = self.nodes[index].previous;
        let next = self.nodes[index].next;
        if let Some(previous) = previous {
            self.nodes[previous].next = next;
        } else {
            self.head = next;
        }
        if let Some(next) = next {
            self.nodes[next].previous = previous;
        } else {
            self.tail = previous;
        }
        self.nodes[index].previous = None;
        self.nodes[index].next = None;
    }
}

fn replacement_glyph(
    font: FontId,
    replacement: u8,
    consumed: impl IntoIterator<Item = PendingHRunChar>,
) -> PendingHRunChar {
    let mut orig = smallvec::SmallVec::new();
    let mut origins = smallvec::SmallVec::new();
    for glyph in consumed {
        orig.extend(glyph.orig);
        origins.extend(glyph.origins);
    }
    PendingHRunChar {
        font,
        ch: char::from(replacement),
        orig,
        origins,
        ligature_present: true,
    }
}

/// TeX82 §§1034-1036's complete ligature cursor.
///
/// Source glyphs, generated pseudo-ligatures, and both boundaries share one
/// work list. Thus every replacement pair re-enters the TFM program, and the
/// retain/delete and pass-over bits move one authoritative cursor.
fn run_tfm_ligature_machine(
    stores: &mut Universe,
    source: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    suppress_right_boundary: bool,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<Vec<Node>, tex_command::CommandError> {
    let Some(first) = source.first() else {
        return Ok(Vec::new());
    };
    let font = first.font;
    let false_bchar = stores.font_false_boundary_char(font);
    let mut work = LigatureWorkList::with_capacity(source.len() + 4);
    if !no_left_boundary {
        work.push_back(LigatureWorkItem::Boundary);
    }
    for entry in source {
        work.push_back(LigatureWorkItem::Glyph(PendingHRunChar::new(
            entry.font,
            entry.ch,
            entry.origin,
        )));
    }
    if !suppress_right_boundary {
        work.push_back(LigatureWorkItem::Boundary);
    }

    let mut cursor = work.head;
    while let Some(left_index) = cursor {
        let Some(right_index) = work.nodes[left_index].next else {
            break;
        };
        fuel.charge()?;
        let left_item = work.nodes[left_index].item.clone();
        let right_item = work.nodes[right_index].item.clone();
        if matches!(left_item, LigatureWorkItem::Kern { .. })
            || matches!(right_item, LigatureWorkItem::Kern { .. })
        {
            cursor = Some(right_index);
            continue;
        }
        let pair: Option<(LigKernChar, LigKernChar)> = match (&left_item, &right_item) {
            (LigatureWorkItem::Boundary, LigatureWorkItem::Glyph(right)) => font_code(right.ch)
                .ok()
                .map(|right| (LigKernChar::Boundary, LigKernChar::Char(right))),
            (LigatureWorkItem::Glyph(left), LigatureWorkItem::Boundary) => font_code(left.ch)
                .ok()
                .map(|left| (LigKernChar::Char(left), LigKernChar::Boundary)),
            (LigatureWorkItem::Glyph(left), LigatureWorkItem::Glyph(right))
                if left.font == right.font =>
            {
                font_code(left.ch)
                    .ok()
                    .zip(font_code(right.ch).ok())
                    .map(|(left, right)| (LigKernChar::Char(left), LigKernChar::Char(right)))
            }
            _ => None,
        };
        let false_boundary_match = matches!(
            &right_item,
            LigatureWorkItem::Glyph(right)
                if right.font == font
                    && font_code(right.ch).ok().is_some_and(|code| Some(code) == false_bchar)
        );
        if false_boundary_match {
            work.nodes[right_index].discard_if_missing = true;
        }
        let Some((left_code, right_code)) = pair else {
            cursor = Some(right_index);
            continue;
        };

        if false_boundary_match {
            if let LigatureWorkItem::Glyph(right) = &right_item
                && !stores.font(right.font).character_exists(right.ch)
            {
                report_missing_character(stores, right.font, right.ch);
                work.remove(right_index);
                break;
            }
            cursor = Some(right_index);
            continue;
        }

        let auto = match (&left_item, &right_item) {
            (LigatureWorkItem::Boundary, LigatureWorkItem::Glyph(right)) => {
                auto_kern(stores, right, Some(true))
            }
            (LigatureWorkItem::Glyph(left), LigatureWorkItem::Boundary) => {
                auto_kern(stores, left, None)
            }
            (LigatureWorkItem::Glyph(left), LigatureWorkItem::Glyph(right)) => {
                auto_kern_between(stores, left, right)
            }
            _ => None,
        };
        if let Some(Node::Kern { amount, kind }) = auto {
            let inserted = work.insert_after(left_index, LigatureWorkItem::Kern { amount, kind });
            cursor = work.nodes[inserted].next;
            continue;
        }

        let Some(command) = stores.tfm_lig_kern_command(font, left_code, right_code) else {
            cursor = Some(right_index);
            continue;
        };
        match command {
            LigKernCommand::Kern(amount) => {
                let inserted = work.insert_after(
                    left_index,
                    LigatureWorkItem::Kern {
                        amount,
                        kind: KernKind::Font,
                    },
                );
                cursor = work.nodes[inserted].next;
            }
            LigKernCommand::Ligature(lig) => {
                let consumed = [
                    lig.delete_current.then(|| work_glyph(&left_item)).flatten(),
                    lig.delete_next.then(|| work_glyph(&right_item)).flatten(),
                ]
                .into_iter()
                .flatten();
                let replacement =
                    LigatureWorkItem::Glyph(replacement_glyph(font, lig.replacement, consumed));
                match (lig.delete_current, lig.delete_next) {
                    (true, true) => {
                        work.nodes[left_index].item = replacement;
                        work.remove(right_index);
                    }
                    (true, false) => work.nodes[left_index].item = replacement,
                    (false, true) => work.nodes[right_index].item = replacement,
                    (false, false) => {
                        work.insert_after(left_index, replacement);
                    }
                }
                let op_byte = lig.pass_over * 4
                    + u8::from(!lig.delete_current) * 2
                    + u8::from(!lig.delete_next);
                cursor = Some(left_index);
                for _ in 0..match op_byte {
                    5..=7 => 1,
                    11 => 2,
                    _ => 0,
                } {
                    cursor = cursor.and_then(|index| work.nodes[index].next);
                }
            }
        }
    }

    let mut out = Vec::with_capacity(work.nodes.len() * 2);
    let mut pending_disc = None;
    let mut index = work.head;
    while let Some(current) = index {
        let item = work.nodes[current].item.clone();
        index = work.nodes[current].next;
        if !matches!(
            item,
            LigatureWorkItem::Kern {
                kind: KernKind::Auto,
                ..
            }
        ) {
            out.extend(pending_disc.take());
        }
        match item {
            LigatureWorkItem::Boundary => {}
            LigatureWorkItem::Glyph(glyph) => {
                if work.nodes[current].discard_if_missing
                    && !stores.font(glyph.font).character_exists(glyph.ch)
                {
                    report_missing_character(stores, glyph.font, glyph.ch);
                    continue;
                }
                let disc = literal_hyphen_disc(stores, &glyph, insert_hyphen_discs);
                out.push(rechar_node(glyph));
                pending_disc = disc;
            }
            LigatureWorkItem::Kern { amount, kind } => out.push(Node::Kern { amount, kind }),
        }
    }
    out.extend(pending_disc);
    Ok(out)
}

fn work_glyph(item: &LigatureWorkItem) -> Option<PendingHRunChar> {
    match item {
        LigatureWorkItem::Glyph(glyph) => Some(glyph.clone()),
        LigatureWorkItem::Boundary | LigatureWorkItem::Kern { .. } => None,
    }
}

fn auto_kern_between(
    stores: &Universe,
    left: &PendingHRunChar,
    right: &PendingHRunChar,
) -> Option<Node> {
    if left.font == right.font {
        return auto_kern_codes(stores, left.font, Some(left.ch), Some(right.ch));
    }
    // Font changes normally flush the old run before the assignment. Keep the
    // fallback deterministic for reconstructed mixed-font runs by applying
    // only the old font's trailing append code here.
    auto_kern_codes(stores, left.font, Some(left.ch), None)
}

fn auto_kern(stores: &Universe, glyph: &PendingHRunChar, leading: Option<bool>) -> Option<Node> {
    match leading {
        Some(true) => auto_kern_codes(stores, glyph.font, None, Some(glyph.ch)),
        _ => auto_kern_codes(stores, glyph.font, Some(glyph.ch), None),
    }
}

fn auto_kern_codes(
    stores: &Universe,
    font: FontId,
    left: Option<char>,
    right: Option<char>,
) -> Option<Node> {
    let configuration = stores.pdf_font_configuration();
    let mut amount = Scaled::from_raw(0);
    if configuration.appends_kerns()
        && let Some(left) = left.and_then(|ch| u8::try_from(ch as u32).ok())
    {
        amount = add_scaled(
            amount,
            scaled_font_code(
                stores,
                font,
                stores.pdf_font_code(tex_state::PdfFontCode::Knac, font, left),
            ),
        );
    }
    if configuration.prepends_kerns()
        && let Some(right) = right.and_then(|ch| u8::try_from(ch as u32).ok())
    {
        amount = add_scaled(
            amount,
            scaled_font_code(
                stores,
                font,
                stores.pdf_font_code(tex_state::PdfFontCode::Knbc, font, right),
            ),
        );
    }
    (amount.raw() != 0).then_some(Node::Kern {
        amount,
        kind: KernKind::Auto,
    })
}

fn add_scaled(left: Scaled, right: Scaled) -> Scaled {
    left.checked_add(right)
        .expect("pdfTeX inter-character kern adjustment fits Scaled")
}

fn adjust_interword_glue(stores: &Universe, nodes: &[Node], spec: &mut GlueSpec) {
    let mut glyph = None;
    for node in nodes.iter().rev() {
        match node {
            Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => {
                glyph = u8::try_from(*ch as u32).ok().map(|code| (*font, code));
                break;
            }
            Node::Kern {
                kind: KernKind::Auto,
                ..
            } => {}
            _ => return,
        }
    }
    let Some((font, code)) = glyph else {
        return;
    };
    let width = scaled_font_code(
        stores,
        font,
        stores.pdf_font_code(tex_state::PdfFontCode::Knbs, font, code),
    );
    let stretch = scaled_font_code(
        stores,
        font,
        stores.pdf_font_code(tex_state::PdfFontCode::Stbs, font, code),
    );
    let shrink = scaled_font_code(
        stores,
        font,
        stores.pdf_font_code(tex_state::PdfFontCode::Shbs, font, code),
    );
    spec.width = spec
        .width
        .checked_add(width)
        .expect("pdfTeX interword width adjustment fits Scaled");
    spec.stretch = spec
        .stretch
        .checked_add(stretch)
        .expect("pdfTeX interword stretch adjustment fits Scaled");
    spec.shrink = spec
        .shrink
        .checked_add(shrink)
        .expect("pdfTeX interword shrink adjustment fits Scaled");
}

fn scaled_font_code(stores: &Universe, font: FontId, code: i32) -> Scaled {
    let product = i64::from(stores.font_parameter(font, 6).raw()) * i64::from(code);
    let rounded = if product >= 0 {
        (product + 500) / 1000
    } else {
        -((-product + 500) / 1000)
    };
    Scaled::from_raw(i32::try_from(rounded).unwrap_or(if rounded < 0 {
        i32::MIN
    } else {
        i32::MAX
    }))
}

fn rechar_node(current: PendingHRunChar) -> Node {
    if current.ligature_present {
        Node::Lig {
            font: current.font,
            ch: current.ch,
            orig: current.orig.into_vec(),
            origins: current.origins.into_vec(),
        }
    } else {
        Node::Char {
            font: current.font,
            ch: current.ch,
            origin: current
                .origins
                .first()
                .copied()
                .unwrap_or(OriginId::UNKNOWN),
        }
    }
}

fn literal_hyphen_disc(
    stores: &mut Universe,
    current: &PendingHRunChar,
    enabled: bool,
) -> Option<Node> {
    if !enabled
        || stores.font_hyphen_char(current.font)
            != current.orig.last().copied().unwrap_or(current.ch) as i32
    {
        return None;
    }
    let empty = stores.freeze_node_list(&[]);
    Some(Node::Disc {
        kind: DiscKind::ExplicitHyphen,
        pre: empty,
        post: empty,
        replace: empty,
    })
}

fn update_space_factor(list: &mut crate::mode::ModeListMutation<'_>, stores: &Universe, ch: char) {
    list.set_space_factor(next_space_factor(list.space_factor(), stores, ch));
}

fn next_space_factor(current: i32, stores: &Universe, ch: char) -> i32 {
    let sf = i32::from(stores.sfcode(ch));
    if sf == 0 {
        return current;
    }
    if sf > 1000 && current < 1000 {
        1000
    } else {
        sf
    }
}

fn nonzero_glue_param_or_font_space(
    stores: &Universe,
    override_param: GlueParam,
    space_factor: i32,
) -> GlueSpec {
    let override_spec = stores.glue(stores.glue_param(override_param));
    if override_spec != GlueSpec::ZERO {
        return override_spec;
    }
    let font = stores.current_font();
    let mut spec = GlueSpec {
        width: stores.font_parameter(font, 2),
        stretch: stores.font_parameter(font, 3),
        stretch_order: Order::Normal,
        shrink: stores.font_parameter(font, 4),
        shrink_order: Order::Normal,
    };
    if space_factor >= 2000 {
        spec.width = spec
            .width
            .checked_add(stores.font_parameter(font, 7))
            .unwrap_or(spec.width);
    }
    if space_factor != 1000 {
        spec.stretch = scale_by_factor(spec.stretch, space_factor, 1000);
        spec.shrink = scale_by_factor(spec.shrink, 1000, space_factor);
    }
    spec
}

fn scale_by_factor(value: Scaled, num: i32, den: i32) -> Scaled {
    Scaled::from_raw(((i64::from(value.raw()) * i64::from(num)) / i64::from(den)) as i32)
}

pub(super) fn infinite_glue(order: Order, negative: bool, shrink: bool) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(0),
        stretch: Scaled::from_raw(if negative {
            -Scaled::UNITY
        } else {
            Scaled::UNITY
        }),
        stretch_order: order,
        shrink: if shrink {
            Scaled::from_raw(Scaled::UNITY)
        } else {
            Scaled::from_raw(0)
        },
        shrink_order: if shrink { order } else { Order::Normal },
    }
}

pub(crate) fn fixed_infinite_glue(primitive: UnexpandablePrimitive) -> GlueSpec {
    match primitive {
        UnexpandablePrimitive::HFil | UnexpandablePrimitive::VFil => {
            infinite_glue(Order::Fil, false, false)
        }
        UnexpandablePrimitive::HFill | UnexpandablePrimitive::VFill => {
            infinite_glue(Order::Fill, false, false)
        }
        UnexpandablePrimitive::HSs | UnexpandablePrimitive::VSs => {
            infinite_glue(Order::Fil, false, true)
        }
        UnexpandablePrimitive::HFilNeg | UnexpandablePrimitive::VFilNeg => {
            infinite_glue(Order::Fil, true, false)
        }
        _ => unreachable!("caller restricts fixed infinite glue primitives"),
    }
}

fn report_missing_character(stores: &mut Universe, font: tex_state::ids::FontId, ch: char) {
    if stores.int_param(IntParam::new(36)) <= 0 {
        return;
    }
    let font_name = stores.font_name(font).to_owned();
    let mut diagnostic = stores.begin_diagnostic();
    diagnostic
        .print_nl("Missing character: There is no ")
        .print_char(ch)
        .print(" in font ")
        .print(&font_name)
        .print_char('!');
    diagnostic.end(false);
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
        report_missing_character(stores, accent_font, char::from(accent));
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
        report_missing_character(stores, base_font, char::from(base));
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
        let mut boxed = super::boxes::hpack_with_overfull_rule(stores, children, PackSpec::Natural);
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
        let token = tex_expand::semantic_token(traced);
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
#[cfg(test)]
pub(crate) fn append_italic_correction(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores, fuel)?;
    let Some((font, ch)) = last_font_char(nest.current_list().nodes()) else {
        return Ok(());
    };
    let Ok(code) = font_code(ch) else {
        return Ok(());
    };
    let Some(metrics) = stores.font_char_metrics(font, code) else {
        return Ok(());
    };
    nest.current_list_mutation().push(Node::Kern {
        amount: metrics.italic_correction,
        kind: KernKind::Explicit,
    });
    Ok(())
}

pub(crate) fn append_italic_correction_with_fuel(
    nest: &mut ModeNest,
    stores: &mut Universe,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    flush_pending_hchars_with_fuel(nest, stores, fuel)?;
    let Some((font, ch)) = last_font_char(nest.current_list().nodes()) else {
        return Ok(());
    };
    let Ok(code) = font_code(ch) else {
        return Ok(());
    };
    let Some(metrics) = stores.font_char_metrics(font, code) else {
        return Ok(());
    };
    nest.current_list_mutation().push(Node::Kern {
        amount: metrics.italic_correction,
        kind: KernKind::Explicit,
    });
    Ok(())
}

fn last_font_char(nodes: &[Node]) -> Option<(tex_state::ids::FontId, char)> {
    match nodes.last()? {
        Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => Some((*font, *ch)),
        _ => None,
    }
}

fn font_code(ch: char) -> Result<u8, ()> {
    u8::try_from(ch as u32).map_err(|_| ())
}

#[cfg(test)]
mod tests;
