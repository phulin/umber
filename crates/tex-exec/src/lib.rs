//! TeX execution engine scaffold.
//!
//! This crate owns the stomach's mode nest and main-control dispatch. It pulls
//! only fully expanded tokens from `tex_expand::get_x_token*`; raw token reads
//! stay in the lexer/gullet pipeline.

#![forbid(unsafe_code)]

mod align;
mod assignments;
mod checkpoint;
mod diagnostics;
mod dispatch;
mod error;
mod executor;
mod math;
mod mode;
mod node_dump;
mod output;
mod packing_params;
mod page_builder;
mod splitting;
mod transaction;
mod vertical;

pub use assignments::{
    install_etex_unexpandable_primitives, install_unexpandable_primitives,
    register_etex_unexpandable_primitives, register_unexpandable_primitives,
    try_execute_assignment,
};
pub use checkpoint::{
    CheckpointSink, ENGINE_CHECKPOINT_SCHEMA_VERSION, EditorRestoreError, EngineBoundary,
    EngineCheckpoint, EngineRestoreError,
};
pub use dispatch::{DispatchAction, ExecutionStats, dispatch_delivered_token};
pub use error::ExecError;
pub use executor::{ExecutionContext, Executor, FontResolver, FontSource};
pub use mode::{
    AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec, Mode, ModeLevelSummary, ModeList,
    ModeNest, ModeNestSummary,
};

pub(crate) use dispatch::{
    insert_traced_tokens, leave_group, leave_group_with_origin, push_tokens, push_traced_tokens,
};

#[cfg(test)]
mod tests;
