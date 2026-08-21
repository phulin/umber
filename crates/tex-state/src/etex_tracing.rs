//! e-TeX 2.6's `\tracinggroups`, `\tracingassigns`, and `\tracingifs`
//! rendered-transcript trace lines.
//!
//! These are presentation-level transcript output, not schema-v1 semantic
//! events: they read live state through the shared §245 diagnostic channel
//! ([`crate::diagnostic`]) exactly as `\showgroups`/`\showifs`/`\showtokens`
//! do, and never feed the detached oracle transport. `\tracinggroups` is
//! implemented here because [`crate::env::group::GroupKind`] and the group
//! stack it displays are owned by this crate; `\tracingassigns`'s value
//! rendering needs the name tables and typed value formatters that live in
//! `tex-exec`, so its trace lines are assembled there against the primitives
//! declared in this module.

use crate::GroupKind;
use crate::env::banks::IntParam;
use crate::universe::Universe;

impl<G> Universe<G> {
    /// e-TeX 2.6 [19.274]'s `group_trace(false)`, called when
    /// [`Self::enter_group_with_kind_at_line`] opens a new save level.
    ///
    /// `level` is e-TeX's `qo(cur_level)`: the number of groups open
    /// *including* the one just entered, matching `\currentgrouplevel` and
    /// the level `\showgroups` prints for the same frame.
    pub(crate) fn trace_group_enter(
        &self,
        effects: &mut crate::diagnostic::DiagnosticEffects,
        kind: GroupKind,
        level: u32,
        entered_line: u32,
    ) {
        if self.int_param(IntParam::TRACING_GROUPS) <= 0 {
            return;
        }
        let mut diagnostic = self.begin_diagnostic(effects);
        diagnostic
            .print_char('{')
            .print("entering ")
            .print(kind.group_text())
            .print(" (level ")
            .print_int(saturating_i32(level))
            .print_char(')');
        if entered_line != 0 {
            diagnostic
                .print(" at line ")
                .print_int(saturating_i32(entered_line));
        }
        diagnostic.print_char('}');
        diagnostic.end(false);
    }

    /// e-TeX 2.6 [19.282]'s `group_trace(true)`, called when
    /// [`Self::leave_group_with_kind`]/[`Self::leave_group`] closes the
    /// innermost save level after its local values have been restored.
    ///
    /// `level` and `entered_line` were captured from the group being closed
    /// before restoration popped its frame.
    pub(crate) fn trace_group_leave(
        &self,
        effects: &mut crate::diagnostic::DiagnosticEffects,
        kind: GroupKind,
        level: u32,
        entered_line: u32,
    ) {
        if self.int_param(IntParam::TRACING_GROUPS) <= 0 {
            return;
        }
        let mut diagnostic = self.begin_diagnostic(effects);
        diagnostic
            .print_char('{')
            .print("leaving ")
            .print(kind.group_text())
            .print(" (level ")
            .print_int(saturating_i32(level))
            .print_char(')');
        if entered_line != 0 {
            diagnostic
                .print(" entered at line ")
                .print_int(saturating_i32(entered_line));
        }
        diagnostic.print_char('}');
        diagnostic.end(false);
    }
}

fn saturating_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests;
