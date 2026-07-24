//! Canonical TeX command-processing boundary.
//!
//! Semantic state machines are crate-private. The public facade grows only as
//! executor integration requires stable, end-state operations.

mod command;
mod conditionals;
mod error;
mod host;
mod input;
mod macro_call;
mod observation;
mod primitives;
mod processor;
mod profile;
mod provenance;
mod scan_toks;
mod scanners;
mod snapshot;
mod state;

pub use command::CurrentCommand;
pub use host::{CommandHostCapabilities, CommandHostContext};
pub use processor::CommandProcessor;
pub use state::{CommandRuntime, CommandState};
