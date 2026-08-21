//! Alignment stomach machinery.

mod execution;
pub(crate) use execution::{FinishedAlignment, append_finished_alignment};

pub(crate) mod packaging;
pub(crate) mod widths;

use crate::{Mode, ModeNest};
use tex_state::CommandContext;

/// TeX82 §787 `init_span`'s aux initialization for a freshly pushed span
/// level.
///
/// `init_span` is the *one* place a column (or group of `\span`-joined
/// columns) starts its own list, and it is reached from both §786 `init_row`
/// and §791 `fin_col`. Its whole body after `push_nest` is
/// `if mode=-hmode then space_factor:=1000 else begin prev_depth:=ignore_depth;
/// normal_paragraph; end`, so a vertically-set entry (`\valign`'s columns, and
/// `\halign`'s rows under `\valign`) begins with `\looseness`, `\hangindent`,
/// `\hangafter`, and `\parshape` back at their defaults. Transcribing only the
/// `prev_depth` half let a nondefault `\looseness`/`\hangafter`/`\hangindent`
/// survive into an entry (`umber2-hq8l`).
pub(crate) fn init_span_aux<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
) {
    if matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) {
        nest.current_list_mutation().set_space_factor(1000);
    } else {
        let ignored_depth = if stores.primitive_resolved("pdfignoreddimen").is_some() {
            stores.dimen_param(tex_state::env::banks::DimenParam::PDF_IGNORED_DIMEN)
        } else {
            crate::mode::IGNORE_DEPTH
        };
        nest.current_list_mutation().set_prev_depth(ignored_depth);
        crate::paragraph_end::normal_paragraph(nest, stores, diagnostic_effects);
    }
}

#[cfg(feature = "profiling")]
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

#[cfg(feature = "profiling")]
mod template_measurement {
    use std::sync::atomic::{AtomicU64, Ordering};

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

#[cfg(feature = "profiling")]
pub fn alignment_template_measurement() -> AlignmentTemplateMeasurement {
    template_measurement::snapshot()
}
