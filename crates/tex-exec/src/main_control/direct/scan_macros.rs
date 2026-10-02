//! Local scanner control flow; no returned operation carrier.
// Consume the native scanner's actual typed product without publishing it
// through a generic frame and projecting it back out.
macro_rules! take_native_scalar {
    ($scan:expr, $origin:expr $(,)?) => {{
        match $scan {
            tex_command::RetainedScalarScan::Complete(value) => value,
            tex_command::RetainedScalarScan::Failed(error) => {
                return Err(scan_error(command_error(error), $origin));
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
     $result:ident, $apply:expr) => {{
        $this.cursor = Some($processor.delivery_cursor());
        let diagnostics = $processor.take_semantic_diagnostics();
        if !diagnostics.is_empty() {
            $this
                .diagnostics
                .extend(diagnostics.into_iter().map(PendingDiagnostic::Command));
        }
        drop($processor);
        $this.control.report_scan_in_context(
            &mut $admission,
            $this.effects,
            std::mem::take(&mut $this.diagnostics),
        )?;
        $this.control.execution.main_loop_active = false;
        let $receipts = &mut $this
            .control
            .operation_observations
            .is_some()
            .then(Vec::new);
        let $result = $apply;
        if $result.is_ok() && $this.control.command.has_pending_named_token_list_pushes() {
            publish_named_token_list_pushes(
                &mut $this.control.command,
                &mut $admission,
                $this.effects,
                &mut $this.control.operation_observations,
            );
        }
        drop($admission);
        if $result.is_ok() && $this.control.operation_observations.is_some() {
            $this.control.publish_semantic_observations(
                $this.stores,
                $this.output_start,
                None,
                $receipts.take(),
            );
        }
        $result
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
            semantic::execute_relax(
                &mut $admission,
                Some(&mut $this.control.execution.modes),
                $this.effects,
                $this.control.fuel.fuel_mut(),
            )
        )
    }};
}
