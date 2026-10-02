//! Local scanner control flow; no returned operation carrier.
macro_rules! take_control_scalar {
    ($frame:expr,$status:expr,$take:ident,$origin:expr $(,)?) => {{
        match $status {
            tex_command::ScalarScanStatus::Complete => $frame.$take(),
            tex_command::ScalarScanStatus::Failed => {
                return Err(scan_error(command_error($frame.take_error()), $origin));
            }
        }
    }};
}

macro_rules! direct_finish_scan {
    ($this:ident, $processor:ident, $admission:ident, $facts:ident, $call:expr) => {{
        $this.cursor = Some($processor.delivery_cursor());
        $this.diagnostics.extend(
            $processor
                .take_semantic_diagnostics()
                .into_iter()
                .map(PendingDiagnostic::Command),
        );
        drop($processor);
        drop($admission);
        $call
    }};
}

// End the actual scanner loan, then apply concrete native operands through
// the still-live context. Evidence remains demand-owned at the common tail.
macro_rules! direct_native_semantic {
    ($this:ident, $processor:ident, $admission:ident, $receipts:ident,
     $result:ident, $pending:ident, $checked:ident, $apply:expr, $settle:block) => {{
        $this.cursor = Some($processor.delivery_cursor());
        let diagnostics = $processor.take_semantic_diagnostics();
        if !diagnostics.is_empty() {
            $this.diagnostics.extend(diagnostics.into_iter().map(PendingDiagnostic::Command));
        }
        drop($processor);
        $this.control.report_scan_in_context(
            &mut $admission, $this.effects, std::mem::take(&mut $this.diagnostics),
        )?;
        $this.control.main_loop_active = false;
        let $receipts = &mut $this.control.operation_observations.is_some().then(Vec::new);
        let $result = $apply;
        if $result.is_ok() && $this.control.command.has_pending_named_token_list_pushes() {
            publish_named_token_list_pushes(
                &mut $this.control.command, &mut $admission, $this.effects,
                &mut $this.control.operation_observations,
            );
        }
        let $checked = MainControl::save_stack_words(
            &$admission, &$this.control.boxes, &$this.control.command,
            $this.control.command.profile(),
        );
        let $pending = PendingPageOutputFacts::capture(&$admission);
        $settle
        drop($admission);
        $this.control.finish_direct_semantic_step(
            $this.stores, $this.effects, $this.output_start, $result, $pending,
            $checked, None, $receipts.take(),
        )
    }};
}

macro_rules! direct_apply_relax {
    ($this:ident, $processor:ident, $admission:ident) => {{
        direct_native_semantic!(
            $this,
            $processor,
            $admission,
            receipts,
            result,
            pending,
            checked,
            semantic::execute_relax(
                &mut $admission,
                &mut $this.control.modes,
                $this.effects,
                $this.control.fuel.fuel_mut(),
            ),
            {}
        )
    }};
}
