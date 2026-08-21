//! Typed cold-command boundary for main control.
//!
//! Ranked commands remain in the persistent interpreter's direct hot path.
//! This module owns only uncommon operations whose operands or semantic work
//! are large enough to justify crossing a typed borrow barrier.

mod alignment;
mod apply;
mod operation;
mod pdf;
mod scan;
mod support;

pub(super) use alignment::*;
pub(super) use apply::{apply as apply_cold_operation, enter_group, leave_group_payloads};
pub(super) use operation::*;
pub(super) use pdf::*;
pub(super) use scan::scan as scan_cold_operation;
pub(super) use scan::{partoken_context_replays, report_incomplete_conditions, scan_off_save};
pub(super) use support::*;
