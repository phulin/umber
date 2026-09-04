//! Source-free canonical math validation, lowering, and display packaging.

pub(crate) mod display;
mod lower;

use tex_state::CommandContext;
use tex_state::math::MathFontSize;

pub(crate) use lower::finish_math_lists_owned;
pub(crate) use lower::{
    MathConversionErrorContext, finish_inline_math_list_node,
    finish_math_list_node_to_shipout_scratch,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MathFontFailure {
    Symbol,
    Extension,
}

impl MathFontFailure {
    pub(super) const fn report(self) -> (&'static str, [&'static str; 3]) {
        match self {
            Self::Symbol => (
                "Math formula deleted: Insufficient symbol fonts",
                [
                    "Sorry, but I can't typeset math unless \\textfont 2",
                    "and \\scriptfont 2 and \\scriptscriptfont 2 have all",
                    "the \\fontdimen values needed in math symbol fonts.",
                ],
            ),
            Self::Extension => (
                "Math formula deleted: Insufficient extension fonts",
                [
                    "Sorry, but I can't typeset math unless \\textfont 3",
                    "and \\scriptfont 3 and \\scriptscriptfont 3 have all",
                    "the \\fontdimen values needed in math extension fonts.",
                ],
            ),
        }
    }
}

pub(super) fn math_font_failure<G>(stores: &mut CommandContext<'_, G>) -> Option<MathFontFailure> {
    const SIZES: [MathFontSize; 3] = [
        MathFontSize::Text,
        MathFontSize::Script,
        MathFontSize::ScriptScript,
    ];
    if SIZES.into_iter().any(|size| {
        let font = stores.math_family_font(size, 2);
        stores.classic_math_parameter_count(font) < 22
            && !matches!(
                stores.font_math_metrics_source(font),
                tex_fonts::MathMetricsSource::OpenType(_)
            )
    }) {
        return Some(MathFontFailure::Symbol);
    }
    if SIZES.into_iter().any(|size| {
        let font = stores.math_family_font(size, 3);
        stores.classic_math_parameter_count(font) < 13
            && !matches!(
                stores.font_math_metrics_source(font),
                tex_fonts::MathMetricsSource::OpenType(_)
            )
    }) {
        return Some(MathFontFailure::Extension);
    }
    None
}

pub(crate) fn reject_invalid_math_fonts<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    context: String,
) -> Result<bool, crate::ExecError> {
    let Some(failure) = math_font_failure(stores) else {
        return Ok(false);
    };
    let (message, help) = failure.report();
    crate::error_report::report_error(stores, diagnostic_effects, message, &help, context)?;
    Ok(true)
}

/// Reports §1194's font failure only after publishing diagnostics which the
/// caller already completed earlier in the same canonical operation.
///
/// Recoverable error dialogue still belongs to World, so this explicit outer
/// barrier releases admission, publishes the detached prefix, then opens a
/// fresh admitted report context. No printer or output-offset state crosses
/// the boundary.
pub(crate) fn reject_invalid_math_fonts_at_outer_barrier<G>(
    stores: &mut tex_state::Universe<G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    context: String,
) -> Result<bool, crate::ExecError> {
    let invalid = {
        let mut admitted = stores.command_context().expect("math-font admission");
        math_font_failure(&mut admitted).is_some()
    };
    if !invalid {
        return Ok(false);
    }
    stores
        .world_mut()
        .publish_diagnostic_effects_preserving(diagnostic_effects);
    reject_invalid_math_fonts(
        &mut stores
            .command_context()
            .expect("math-font report admission"),
        diagnostic_effects,
        context,
    )
}
