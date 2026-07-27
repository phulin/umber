//! The differential tracer's exit-status contract.
//!
//! Four statuses, because three different things have been mistaken for a
//! fourth in this epic:
//!
//! - `0` CLEAN: every registered fixture was compared to exhaustion, none
//!   diverged.
//! - `1` DIVERGED: every registered fixture was compared to exhaustion, and
//!   the printed divergence total is exact.
//! - `2` PARTIAL: a registered fixture was never compared, or a fixture's
//!   comparison stopped at its `--max-divergences` budget. Every printed
//!   total is a lower bound, so a total of `0` does not mean convergence.
//! - `3` the run could not be performed at all (usage error, unreadable
//!   suite, registry inconsistent with its committed pin).
//!
//! The report is written whenever a run happened, including a clean one:
//! a gate that prints nothing cannot be told apart from a gate that did not
//! execute.

use std::process::ExitCode;

use tex_command_stream::EXIT_NOT_RUN;

fn main() -> ExitCode {
    match tex_command_stream::run_cli() {
        Ok(report) => {
            eprintln!("{report}");
            ExitCode::from(report.outcome().exit_code())
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(EXIT_NOT_RUN)
        }
    }
}
