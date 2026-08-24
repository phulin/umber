//! TeX82 §§532--536 job-output identity and transcript-open state.

use std::path::Path;

use tex_state::print::{Printer, Selector};
use tex_state::{RetainedOutputOpenOutcome, Universe};

/// Engine-owned output names and the one semantic transcript-open bit.
///
/// Host publication remains outside the engine. This owner decides the names
/// TeX asks the host to open, including interactive retry, before bytes cross
/// that boundary.
#[derive(Clone, Debug, Default)]
pub(crate) struct JobOutput {
    dvi_name: Option<String>,
    log_name: Option<String>,
    log_opened: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JobOutputOpenError {
    TerminalInputUnavailable,
    NonInteractive,
}

impl JobOutput {
    pub(crate) fn dvi_name<G>(
        &mut self,
        stores: &mut Universe<G>,
        job_name: &str,
    ) -> Result<&str, JobOutputOpenError> {
        if self.dvi_name.is_none() {
            let initial = derived_name(job_name, ".dvi");
            let opened = open_with_retry(stores, initial, ".dvi", false)?;
            // TeX82 §§525/532 retain `b_make_name_string(dvi_file)`.
            stores
                .command_context()
                .expect("job output belongs to a live generation")
                .make_string_pool_string(&opened);
            self.dvi_name = Some(opened);
        }
        Ok(self.dvi_name.as_deref().expect("DVI name was initialized"))
    }

    pub(crate) fn open_log<G>(
        &mut self,
        stores: &mut Universe<G>,
        job_name: &str,
    ) -> Result<&str, JobOutputOpenError> {
        if self.log_name.is_none() {
            let initial = derived_name(job_name, ".log");
            self.log_name = Some(open_with_retry(stores, initial, ".log", true)?);
            self.log_opened = true;
        }
        Ok(self.log_name.as_deref().expect("log name was initialized"))
    }

    #[cfg(test)]
    pub(crate) const fn log_opened(&self) -> bool {
        self.log_opened
    }
}

fn derived_name(job_name: &str, extension: &str) -> String {
    format!(
        "{}{extension}",
        if job_name.is_empty() {
            "texput"
        } else {
            job_name
        }
    )
}

fn open_with_retry<G>(
    stores: &mut Universe<G>,
    mut candidate: String,
    default_extension: &str,
    transcript: bool,
) -> Result<String, JobOutputOpenError> {
    loop {
        if stores.world().retained_output_open_outcome(&candidate)
            != RetainedOutputOpenOutcome::Unavailable
        {
            return Ok(candidate);
        }
        // §535's transcript exception temporarily makes the terminal visible
        // even when batch mode selected log-only output. The prompt itself is
        // otherwise routed through the ambient selector.
        if transcript {
            Printer::new(stores, Selector::TermOnly)
                .print_nl("I can't write on file `")
                .print(&candidate)
                .print("'.");
        } else {
            stores
                .printer()
                .print_nl("I can't write on file `")
                .print(&candidate)
                .print("'.");
        }
        if !stores
            .command_context()
            .expect("job output belongs to a live generation")
            .interaction_permits_terminal_input()
        {
            return Err(JobOutputOpenError::NonInteractive);
        }
        Printer::new(stores, Selector::TermOnly)
            .print_nl("Please type another output file name")
            .print("? ");
        let Some(line) = stores
            .world_mut()
            .read_terminal_line()
            .map_err(|_| JobOutputOpenError::TerminalInputUnavailable)?
        else {
            return Err(JobOutputOpenError::TerminalInputUnavailable);
        };
        stores.world_mut().echo_terminal_input(&line);
        candidate = with_default_extension(line.trim(), default_extension);
    }
}

fn with_default_extension(name: &str, extension: &str) -> String {
    if Path::new(name).extension().is_some() {
        name.to_owned()
    } else {
        format!("{name}{extension}")
    }
}

#[cfg(test)]
mod tests;
