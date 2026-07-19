//! Alignment stomach machinery.

mod execution;
pub(crate) use execution::FinishedAlignment;
#[cfg(test)]
pub(crate) use execution::append_finished_alignment;
pub(crate) use execution::{DoEndV, do_endv};

mod noalign;
mod packaging;
mod preamble;
mod support;
mod template;
mod widths;

use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::meaning::UnexpandablePrimitive;
#[cfg(feature = "profiling-stats")]
use tex_state::token::Token;
use tex_state::token::TracedTokenWord;

use crate::{ExecError, ModeNest};

pub(crate) use preamble::scan_preamble;

#[cfg(feature = "profiling-stats")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AlignmentTemplateMeasurement {
    pub invocations: u64,
    pub delivered_tokens: u64,
    pub character_tokens: u64,
    pub control_sequence_tokens: u64,
    pub relax_commands: u64,
    pub font_commands: u64,
    pub unexpandable_commands: u64,
    pub inert_glue_commands: u64,
    pub other_commands: u64,
}

#[cfg(feature = "profiling-stats")]
mod template_measurement {
    use std::sync::atomic::{AtomicU64, Ordering};

    use tex_state::Universe;
    use tex_state::meaning::Meaning;
    use tex_state::token::Token;

    use super::AlignmentTemplateMeasurement;

    static INVOCATIONS: AtomicU64 = AtomicU64::new(0);
    static DELIVERED: AtomicU64 = AtomicU64::new(0);
    static CHARACTERS: AtomicU64 = AtomicU64::new(0);
    static CONTROL_SEQUENCES: AtomicU64 = AtomicU64::new(0);
    static RELAX: AtomicU64 = AtomicU64::new(0);
    static FONTS: AtomicU64 = AtomicU64::new(0);
    static UNEXPANDABLE: AtomicU64 = AtomicU64::new(0);
    static INERT_GLUE: AtomicU64 = AtomicU64::new(0);
    static OTHER: AtomicU64 = AtomicU64::new(0);

    pub(super) fn record_invocation() {
        INVOCATIONS.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_token(token: Token, stores: &Universe) {
        DELIVERED.fetch_add(1, Ordering::Relaxed);
        match token {
            Token::Char { .. } => {
                CHARACTERS.fetch_add(1, Ordering::Relaxed);
            }
            Token::Cs(symbol) => {
                CONTROL_SEQUENCES.fetch_add(1, Ordering::Relaxed);
                let meaning = stores.meaning(symbol);
                if matches!(
                    meaning,
                    Meaning::UnexpandablePrimitive(
                        tex_state::meaning::UnexpandablePrimitive::HFil
                            | tex_state::meaning::UnexpandablePrimitive::HFill
                            | tex_state::meaning::UnexpandablePrimitive::HSs
                            | tex_state::meaning::UnexpandablePrimitive::HFilNeg
                    )
                ) {
                    INERT_GLUE.fetch_add(1, Ordering::Relaxed);
                }
                let counter = match meaning {
                    Meaning::Relax => &RELAX,
                    Meaning::Font(_) => &FONTS,
                    Meaning::UnexpandablePrimitive(_) => &UNEXPANDABLE,
                    _ => &OTHER,
                };
                counter.fetch_add(1, Ordering::Relaxed);
            }
            Token::Param(_) | Token::Frozen(_) => {
                OTHER.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(super) fn snapshot() -> AlignmentTemplateMeasurement {
        AlignmentTemplateMeasurement {
            invocations: INVOCATIONS.load(Ordering::Relaxed),
            delivered_tokens: DELIVERED.load(Ordering::Relaxed),
            character_tokens: CHARACTERS.load(Ordering::Relaxed),
            control_sequence_tokens: CONTROL_SEQUENCES.load(Ordering::Relaxed),
            relax_commands: RELAX.load(Ordering::Relaxed),
            font_commands: FONTS.load(Ordering::Relaxed),
            unexpandable_commands: UNEXPANDABLE.load(Ordering::Relaxed),
            inert_glue_commands: INERT_GLUE.load(Ordering::Relaxed),
            other_commands: OTHER.load(Ordering::Relaxed),
        }
    }
}

#[cfg(feature = "profiling-stats")]
pub fn alignment_template_measurement() -> AlignmentTemplateMeasurement {
    template_measurement::snapshot()
}

#[cfg(feature = "profiling-stats")]
fn record_template_invocation() {
    template_measurement::record_invocation();
}

#[cfg(feature = "profiling-stats")]
fn record_template_token(token: Token, stores: &Universe) {
    template_measurement::record_token(token, stores);
}

pub(crate) fn execute_alignment(
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    if stores.world().execution_tracing_enabled() {
        stores
            .world_mut()
            .trace_execution("alignment", format!("begin {primitive:?}"));
    }
    let suspended = input.suspend_alignment_cell();
    input.begin_alignment();
    let mut transaction = crate::transaction::ExecutionTransaction::begin(nest, stores);
    let result = (|| {
        let (nest, stores) = transaction.parts();
        let state = scan_preamble(primitive, context, input, stores, execution)?;
        execution::execute_alignment(state, nest, input, stores, execution)
    })();
    match result {
        Ok(()) => {
            input.finish_alignment();
            input.resume_alignment_cell(suspended);
            transaction.commit();
            stores.world_mut().trace_execution("alignment", "commit");
            Ok(())
        }
        Err(error) => {
            input.abort_alignment_and_resume(suspended);
            drop(transaction);
            stores.world_mut().trace_execution("alignment", "rollback");
            Err(error)
        }
    }
}

pub(crate) fn execute_display_halign(
    context: TracedTokenWord,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<FinishedAlignment, ExecError> {
    if stores.world().execution_tracing_enabled() {
        stores
            .world_mut()
            .trace_execution("alignment", "begin display halign");
    }
    let suspended = input.suspend_alignment_cell();
    input.begin_alignment();
    let mut transaction = crate::transaction::ExecutionTransaction::begin(nest, stores);
    let result = (|| {
        let (nest, stores) = transaction.parts();
        let state = scan_preamble(
            UnexpandablePrimitive::HAlign,
            context,
            input,
            stores,
            execution,
        )?;
        execution::execute_alignment_to_nodes(state, nest, input, stores, execution)
    })();
    match result {
        Ok(finished) => {
            input.finish_alignment();
            input.resume_alignment_cell(suspended);
            transaction.commit();
            stores
                .world_mut()
                .trace_execution("alignment", "commit display halign");
            Ok(finished)
        }
        Err(error) => {
            input.abort_alignment_and_resume(suspended);
            drop(transaction);
            stores
                .world_mut()
                .trace_execution("alignment", "rollback display halign");
            Err(error)
        }
    }
}
