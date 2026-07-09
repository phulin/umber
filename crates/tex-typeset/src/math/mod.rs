//! Pure Appendix G math-list to horizontal-list conversion.

mod convert;
mod delimiters;
mod fractions;
mod model;
mod operators;
mod params;
mod radicals;
mod scripts;
mod spacing;
mod style;

use tex_fonts::metrics::ExtensibleRecipe as MetricExtensibleRecipe;
use tex_fonts::{LigKernChar, LigKernCommand};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::ids::{FontId, GlueId};
use tex_state::math::MathFontSize;
#[cfg(test)]
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

use crate::TypesetState;

pub use convert::mlist_to_hlist;
pub(crate) use convert::{
    Context, FetchedChar, add, char_box, clean_box, fetch, make_character_nucleus, source_node, sub,
};
pub use delimiters::{left_right_delimiter_target, var_delimiter};
pub use model::{BoxAxis, FrozenHList, MathBox, MathGlueKind, MathNode};
pub(crate) use model::{boxed_node, hlist_extents, hpack, node_is_char, vpack};
pub use params::{ExtensionParams, MathParamState, MathParams, SizeParams, SymbolParams};
pub use spacing::{SpacingKind, inter_noad_spacing, math_glue, math_kern};
pub use style::{Style, StyleFamily};

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
}

impl MathTypesetState for Universe {
    fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        Universe::math_family_font(self, size, family)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        Universe::font_parameter(self, font, number)
    }

    fn font_next_larger(&self, font: FontId, code: u8) -> Option<u8> {
        Universe::font_next_larger(self, font, code)
    }

    fn font_extensible_recipe(&self, font: FontId, code: u8) -> Option<MetricExtensibleRecipe> {
        Universe::extensible_recipe(self, font, code)
    }

    fn lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        Universe::lig_kern_command(self, font, left, right)
    }

    fn font_skew_char(&self, font: FontId) -> i32 {
        Universe::font_skew_char(self, font)
    }
}

impl MathParamState for Universe {
    fn int_param(&self, param: IntParam) -> i32 {
        Universe::int_param(self, param)
    }

    fn dimen_param(&self, param: DimenParam) -> Scaled {
        Universe::dimen_param(self, param)
    }

    fn glue_param(&self, param: GlueParam) -> GlueId {
        Universe::glue_param(self, param)
    }
}

#[cfg(test)]
mod tests;
