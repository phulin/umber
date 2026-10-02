//! Direct page state semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_page_dimension<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    dimension: &mut PageDimension,
    value: &mut Scaled,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1245's `alter_page_so_far`: a direct
        // `page_so_far[c]:=cur_val` store with no mode check, no
        // diagnostic, and no save-stack entry (§1242: "these definitions
        // are always global"). The page builder reads the same slots.
        AssignmentCommitter::new(stores, command.diagnostic_effects)
            .unscoped(|stores| stores.set_page_dimension(*dimension, *value));
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_page_integer<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    integer: &mut PageInteger,
    value: &mut i32,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1246's `alter_integer`, scoped exactly like
        // `alter_page_so_far` above. `\deadcycles` in particular is what
        // §1024's output-routine loop guard compares against
        // `\maxdeadcycles`, so a wrong value here is only visible once a
        // page ships.
        AssignmentCommitter::new(stores, command.diagnostic_effects)
            .unscoped(|stores| stores.set_page_integer(*integer, *value));
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_continue() -> Result<ReplayStep, ExecError> {
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn execute_relax<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: Option<&mut ModeNest>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<ReplayStep, ExecError> {
    #[cfg(feature = "profiling")]
    tex_state::measurement::record_hot_core_phase(
        tex_state::measurement::HotCorePhase::SemanticApply,
    );
    #[cfg(feature = "profiling")]
    let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
        tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
    );
    {
        // TeX82 §1030 reaches §1045's do-nothing arm only after leaving
        // the ligature loop. The command itself has no list effect, but
        // it is still a word boundary: `?\\relax\\char96` must not form
        // the `?`` ligature across the relax.
        if let Some(modes) = modes {
            crate::box_runtime::flush_pending_hchars_with_fuel(
                modes,
                stores,
                diagnostic_effects,
                fuel,
            )?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_end_of_input() -> Result<ReplayStep, ExecError> {
    Ok(ReplayStep::EndOfInput)
}

pub(in crate::main_control) fn execute_end<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    dump: &mut bool,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1054 has accepted `its_all_over`; the page-builder-owned
        // replay fence must not enter terminal or INITEX format state.
        stores.finish_end_job();
        // §1335's final_cleanup tail -- closing every still-open paren,
        // reporting `incomplete_conditions`, the "(see the transcript
        // file..." note, and this same `dump` flag's
        // `(\dump is performed only by INITEX)` note, in that exact
        // order -- runs in `MainControl::end_of_job_final_cleanup`
        // once this returns: the paren close needs `self`'s job-framing
        // state, which this free function does not have, and tex.web
        // orders it first. `incomplete_conditions` is discarded here
        // rather than used, because the caller re-derives it from the
        // the end command passed to this
        // call (see `apply_operation`).
        //
        // TeX82 §1335's INITEX tail releases `last_glue` before
        // `store_fmt_file`; e-TeX 2.6's [45.999] change may meanwhile
        // have retained top-of-page glue, kerns, and penalties in
        // `page_discards`, which are deliberately absent from the
        // format. `its_all_over` (§1054) already proved that the page and
        // contribution lists contain no live material. Normalize the
        // remaining page-builder scalars while preserving both e-TeX
        // discard lists, so the host-side format encoder still rejects
        // genuine page material instead of mistaking `last_penalty` or
        // `last_node_type` for it.
        if *dump && command.initex && crate::page_output::job_is_all_over(stores) {
            stores.start_new_page();
        }
        // TeX82 §1378 closes every still-open numbered output file after
        // `final_cleanup`. The two normalized fallback selectors are not
        // file slots (§1342), so the state boundary exposes only 0..15
        // here. `close_out` preserves §1378's `if write_open[k]` guard:
        // never-opened and already-closed slots produce no close effect.
        // This runs here, synchronously with `\end`/`\dump` applying,
        // rather than in the driver-facing §1333 `finish_job` that prints
        // §642's DVI report: it is a `World` state effect, not a print,
        // and it has already happened by the time a driver can call
        // `finish_job` (which only runs after this step has already
        // returned), so its position here can't reorder anything
        // `finish_job` prints. Leaving it here also keeps it exactly
        // where `effects::tests::
        // output_stream_final_cleanup_closes_only_live_numbered_files`
        // already observes it, synchronous with the terminating step.
        for raw in 0..tex_state::world::STREAM_SLOT_COUNT as u8 {
            stores.close_output_stream(StreamSlot::new(raw));
        }
        Ok(ReplayStep::End)
    }
}

pub(in crate::main_control) fn execute_eject_residual_page<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1054's `its_all_over` false branch. The stop is already
        // backed up; appending the end-job trio and running §994's
        // `build_page` is all the ejection this step performs. §1005's
        // `@<Check if node p is a new champion breakpoint...@>` decides
        // whether the `-'10000000000` penalty fires §1012's `fire_up`,
        // and §1025 alone ever starts `\output`.
        // Each canonical retry appends a fresh trio. The page builder owns
        // the retry fence because only its journaled transition position
        // can distinguish a legitimate retry after default output, an
        // explicit routine, insertion heldovers, or a dead cycle from an
        // identical-state replay. The old executor boolean stayed set
        // after a prior trio had been consumed and could therefore leave
        // a terminal kern alone forever.
        let progress = stores
            .begin_end_job_ejection()
            .map_err(|_| ExecError::Fatal(FatalError::confusion("end job page progress")))?;
        crate::page_output::append_end_job_contributions(stores);
        crate::page_builder::build_page(stores, command.diagnostic_effects, command.state.state())?;
        if !stores.complete_end_job_ejection(progress) {
            return Err(ExecError::Fatal(FatalError::confusion(
                "end job page progress",
            )));
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_prev_depth<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    value: &mut Scaled,
) -> Result<ReplayStep, ExecError> {
    {
        debug_assert!(matches!(
            modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        ));
        AssignmentCommitter::new(stores, command.diagnostic_effects)
            .unscoped(|_| modes.current_list_mutation().set_prev_depth(*value));
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_space_factor<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    value: &mut i32,
) -> Result<ReplayStep, ExecError> {
    {
        debug_assert!(matches!(
            modes.current_mode(),
            Mode::Horizontal | Mode::RestrictedHorizontal
        ));
        // TeX82 §1243's `alter_aux`: `if (cur_val<=0)or(cur_val>32767)
        // then int_error(cur_val) else space_factor:=cur_val` -- an
        // out-of-range value is diagnosed and left unchanged rather than
        // clamped.
        if (1..=32767).contains(value) {
            AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(|_| {
                modes.current_list_mutation().set_space_factor(*value);
            });
        } else {
            // §91's `int_error` appends ` (value)` to the message before
            // §82 completes the report, so the value is not part of the
            // `print_err` text.
            let context = command.state.output_open_context(&*stores);
            let mut report = stores.print_err("Bad space factor");
            report
                .help(&["I allow only values in the range 1..32767 here."])
                .context(context);
            report
                .int_error(*value)
                .defer_recovery(command.diagnostic_effects)?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_prev_graf<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    value: &mut i32,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1244's `alter_prev_graf`: `\prevgraf` is `any_mode` (it
        // walks the mode nest up to its nearest enclosing vertical level
        // rather than checking the current mode), unlike `\spacefactor`/
        // `\prevdepth`'s §1243 `report_illegal_case`.
        if *value < 0 {
            let context = command.state.output_open_context(&*stores);
            let mut report = stores.print_err("Bad ");
            report
                .print_esc("prevgraf")
                .help(&["I allow only nonnegative values here."])
                .context(context);
            report
                .int_error(*value)
                .defer_recovery(command.diagnostic_effects)?;
        } else {
            AssignmentCommitter::new(stores, command.diagnostic_effects).unscoped(|_| {
                modes.set_enclosing_vertical_prev_graf(*value);
            });
        }
        Ok(ReplayStep::Continue)
    }
}
