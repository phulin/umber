//! Pure Appendix G math-list to horizontal-list conversion.

mod arithmetic;
mod convert;
mod delimiters;
mod fractions;
mod model;
mod operators;
mod params;
mod radicals;
mod rebox;
mod scripts;
mod spacing;
mod style;
mod variants;

#[cfg(test)]
use crate::test_state::TestState;
use tex_fonts::metrics::ExtensibleRecipe as MetricExtensibleRecipe;
use tex_fonts::{CharMetrics, LigKernChar, LigKernCommand, MathMetricsSource};
#[cfg(test)]
use tex_state::env::banks::DimenParam;
use tex_state::ids::FontId;
use tex_state::math::MathFontSize;
#[cfg(test)]
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

use crate::TypesetState;

pub(crate) use arithmetic::{add, mul, neg, sub};
pub use convert::mlist_to_hlist;
pub(crate) use convert::{
    Context, FetchedChar, char_box, clean_box, fetch, make_character_nucleus, source_box_payload,
};
pub use delimiters::left_right_delimiter_target;
#[cfg(test)]
pub(crate) use delimiters::test_var_delimiter;
pub use model::{
    BoxAxis, FrozenHList, MathBox, MathConversionEvent, MathGlueKind, MathLayout, MathNode,
    MathPackObservation, NativeBoxSource, NativeNodeEvidence,
};
pub(crate) use model::{NativeNodeTransaction, boxed_node, node_is_char};
pub use params::{ExtensionParams, MathParamState, MathParams, SizeParams, SymbolParams};
pub use spacing::{SpacingKind, inter_noad_spacing, math_glue, math_kern};
pub use style::{Style, StyleFamily};
pub(crate) use variants::variant_box;

/// Immutable state access needed by the math typesetting kernel.
pub trait MathTypesetState: TypesetState {
    fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId;
    fn font_parameter(&self, font: FontId, number: u16) -> Scaled;
    fn font_next_larger(&self, font: FontId, code: u8) -> Option<u8>;
    fn font_extensible_recipe(&self, font: FontId, code: u8) -> Option<MetricExtensibleRecipe>;
    fn lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand>;
    fn font_skew_char(&self, font: FontId) -> i32;
    fn classic_math_char_metrics(&self, font: FontId, code: u8) -> Option<CharMetrics> {
        self.font_char_metrics(font, code)
    }
    fn math_metrics_source(&self, _font: FontId) -> MathMetricsSource<'_> {
        MathMetricsSource::ClassicTfmExact
    }
}

#[cfg(test)]
mod tests;
