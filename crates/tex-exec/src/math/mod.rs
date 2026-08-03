//! Source-free canonical math validation, lowering, and display packaging.

pub(crate) mod display;
#[cfg(any())]
pub(crate) mod legacy_front;
#[cfg(any())]
mod legacy_scan;
mod lower;
mod support;

use tex_state::Universe;
use tex_state::math::MathFontSize;

#[cfg(any())]
pub(crate) use lower::finish_math_lists;
pub(crate) use lower::finish_math_lists_owned;
pub(crate) use lower::{
    MathConversionErrorContext, finish_inline_math_list_node, finish_math_list_node,
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

pub(super) fn math_font_failure(stores: &mut Universe) -> Option<MathFontFailure> {
    const SIZES: [MathFontSize; 3] = [
        MathFontSize::Text,
        MathFontSize::Script,
        MathFontSize::ScriptScript,
    ];
    if SIZES.into_iter().any(|size| {
        stores.observe_semantic_dependency(tex_state::DependencyKey::Cell {
            bank: tex_state::DependencyBank::MathFamilyFont,
            index: u32::from(size.index()) * 16 + 2,
        });
        let font = stores.math_family_font(size, 2);
        stores.classic_math_parameter_count(font) < 22
            && !matches!(
                stores.font(font).math_metrics_source(),
                tex_fonts::MathMetricsSource::OpenType(_)
            )
    }) {
        return Some(MathFontFailure::Symbol);
    }
    if SIZES.into_iter().any(|size| {
        stores.observe_semantic_dependency(tex_state::DependencyKey::Cell {
            bank: tex_state::DependencyBank::MathFamilyFont,
            index: u32::from(size.index()) * 16 + 3,
        });
        let font = stores.math_family_font(size, 3);
        stores.classic_math_parameter_count(font) < 13
            && !matches!(
                stores.font(font).math_metrics_source(),
                tex_fonts::MathMetricsSource::OpenType(_)
            )
    }) {
        return Some(MathFontFailure::Extension);
    }
    None
}

pub(crate) fn reject_invalid_math_fonts(
    stores: &mut Universe,
    context: String,
) -> Result<bool, crate::ExecError> {
    let Some(failure) = math_font_failure(stores) else {
        return Ok(false);
    };
    let (message, help) = failure.report();
    crate::error_report::report_error(stores, message, &help, context)?;
    Ok(true)
}

#[cfg(any())]
pub(crate) fn testing_math_font_failure(stores: &mut Universe) -> Option<&'static str> {
    math_font_failure(stores).map(|failure| match failure {
        MathFontFailure::Symbol => "symbol",
        MathFontFailure::Extension => "extension",
    })
}

#[cfg(any())]
pub(crate) fn testing_finish_current_math_list(
    nest: &mut crate::ModeNest,
    stores: &mut Universe,
) -> tex_state::ids::NodeListId {
    support::finish_current_math_list(nest, stores)
}
