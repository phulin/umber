//! Retired assignment scanner and execution-context facade.
//!
//! The implementation remains beside the source-free assignment mutation and
//! packing helpers while those helpers are progressively assigned narrower
//! owners. Legacy command drivers import this facade so canonical command
//! control cannot accidentally regain an `InputStack`/`ExecutionContext`
//! assignment front.

pub(crate) use crate::assignments::*;
pub use crate::assignments::try_execute_assignment;
