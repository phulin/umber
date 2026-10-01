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
