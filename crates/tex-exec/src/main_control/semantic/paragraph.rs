//! Direct paragraph semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_paragraph_indent<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    indent: &mut bool,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1090 routes only `vmode+start_par` to §1091 `new_graf`,
        // the single site that pushes `\everypar`. §1092 routes both
        // `hmode+start_par` and `mmode+start_par` to §1093
        // `indent_in_hmode`, which appends the paragraph-indent box (as
        // an ordinary `sub_box` noad in math mode) without beginning a
        // paragraph -- so an `\indent` inside a paragraph already under
        // way must not replay `\everypar` a second time.
        if matches!(
            modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            start_paragraph(
                command.state,
                modes,
                stores,
                command.diagnostic_effects,
                *indent,
            )?;
        } else {
            crate::box_runtime::indent_in_hmode(
                modes,
                stores,
                command.diagnostic_effects,
                *indent,
                command.fuel,
            )?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_set_language<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    language: &mut i32,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1377, verbatim:
        //
        //   new_whatsit(language_node,small_node_size);
        //   scan_int;
        //   if cur_val<=0 then clang:=0
        //   else if cur_val>255 then clang:=0
        //   else clang:=cur_val;
        //   what_lang(tail):=clang;
        //   what_lhm(tail):=norm_min(left_hyphen_min);
        //   what_rhm(tail):=norm_min(right_hyphen_min);
        //
        // Both out-of-range directions recover to language zero; only
        // `1..=255` survives. The pending character run is flushed
        // first, before `clang` moves, so it hyphenates under the
        // language that was current while it was being built.
        let clang = u8::try_from(*language).unwrap_or(0);
        let left_hyphen_min =
            crate::box_runtime::norm_min(stores.int_param(IntParam::LEFT_HYPHEN_MIN));
        let right_hyphen_min =
            crate::box_runtime::norm_min(stores.int_param(IntParam::RIGHT_HYPHEN_MIN));
        crate::box_runtime::append_whatsit(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            Whatsit::Language {
                language: clang,
                left_hyphen_min,
                right_hyphen_min,
            },
        )?;
        modes
            .current_list_mutation()
            .set_hyphen_context(clang, left_hyphen_min, right_hyphen_min);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_mark<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    class: &mut u16,
    tokens: &mut tex_state::TokenListId<G>,
) -> Result<ReplayStep, ExecError> {
    {
        // No `build_page` call afterward (unlike `\penalty`/`\insert`):
        // TeX82 §1101 and e-TeX 2.6 [26.424]'s `make_mark` append the
        // node in every mode and leave page building to a later trigger.
        crate::box_runtime::flush_pending_hchars_with_fuel(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        let migration_segment_start = (modes.current_mode() == Mode::RestrictedHorizontal
            && boxes.active_boxes.last().is_some_and(|box_state| {
                box_state.kind == ReplayBoxKind::HBox && box_state.box_segment_start.is_some()
            }))
        .then(|| stores.begin_page_node_region());
        let tokens = stores.node_token_list(tokens);
        crate::vertical::append_vertical_contribution(
            modes,
            stores,
            Node::Mark {
                class: *class,
                tokens,
            },
        );
        if let Some(start) = migration_segment_start {
            let segment = stores.close_page_box_segment(start);
            if !segment.is_empty() {
                boxes
                    .active_boxes
                    .last_mut()
                    .expect("mark remains inside its enclosing hbox")
                    .migration_segments
                    .push(segment);
            }
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_paragraph<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    current_line: &mut i32,
) -> Result<ReplayStep, ExecError> {
    {
        if matches!(
            modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ) {
            crate::paragraph_end::normal_paragraph(modes, stores, command.diagnostic_effects);
            crate::vertical::build_page_if_outer_vertical(
                modes,
                stores,
                command.diagnostic_effects,
                &*command.state,
            )?;
        } else {
            let mut diagnostic_context =
                command_diagnostic_context(command.state, command.output_routine_active);
            diagnostic_context.current_line = *current_line;
            let mut geometry = pack_geometry_sink(command.state, command.observations);
            crate::paragraph_end::end_paragraph_with_fuel(
                modes,
                stores,
                command.diagnostic_effects,
                &mut geometry,
                diagnostic_context,
                command.fuel,
            )?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_paragraph_start<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    {
        start_paragraph(
            command.state,
            modes,
            stores,
            command.diagnostic_effects,
            true,
        )?;
        Ok(ReplayStep::Continue)
    }
}
