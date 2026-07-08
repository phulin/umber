//! TeX execution engine scaffold.
//!
//! This crate owns the stomach's mode nest and main-control dispatch. It pulls
//! only fully expanded tokens from `tex_expand::get_x_token*`; raw token reads
//! stay in the lexer/gullet pipeline.

#![forbid(unsafe_code)]

mod assignments;
mod diagnostics;
mod dispatch;
mod error;
mod executor;
mod mode;

pub use assignments::{install_unexpandable_primitives, try_execute_assignment};
pub use diagnostics::{LogSink, NoopLogSink, StringLogSink};
pub use dispatch::{
    DispatchAction, ExecutionStats, dispatch_delivered_token,
    dispatch_delivered_token_with_log_sink,
};
pub use error::ExecError;
pub use executor::Executor;
pub use mode::{ListBuilderSummary, Mode, ModeLevelSummary, ModeNest, ModeNestSummary};

pub(crate) use dispatch::{leave_group, push_tokens};

#[cfg(test)]
mod tests;
