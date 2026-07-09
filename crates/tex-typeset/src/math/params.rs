use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::GlueSpec;
use tex_state::math::MathFontSize;
use tex_state::scaled::Scaled;

use super::MathTypesetState;

/// Math-symbol font parameters for one TeX math size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SymbolParams {
    pub math_x_height: Scaled,
    pub math_quad: Scaled,
    pub num1: Scaled,
    pub num2: Scaled,
    pub num3: Scaled,
    pub denom1: Scaled,
    pub denom2: Scaled,
    pub sup1: Scaled,
    pub sup2: Scaled,
    pub sup3: Scaled,
    pub sub1: Scaled,
    pub sub2: Scaled,
    pub sup_drop: Scaled,
    pub sub_drop: Scaled,
    pub delim1: Scaled,
    pub delim2: Scaled,
    pub axis_height: Scaled,
}

/// Math-extension font parameters for one TeX math size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtensionParams {
    pub default_rule_thickness: Scaled,
    pub big_op_spacing1: Scaled,
    pub big_op_spacing2: Scaled,
    pub big_op_spacing3: Scaled,
    pub big_op_spacing4: Scaled,
    pub big_op_spacing5: Scaled,
}

/// Plain value snapshot needed by Appendix G conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathParams {
    pub text: SizeParams,
    pub script: SizeParams,
    pub script_script: SizeParams,
    pub delimiter_factor: i32,
    pub delimiter_shortfall: Scaled,
    pub null_delimiter_space: Scaled,
    pub script_space: Scaled,
    pub thin_mu_skip: GlueSpec,
    pub med_mu_skip: GlueSpec,
    pub thick_mu_skip: GlueSpec,
}

/// Per-size math parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SizeParams {
    pub symbols: SymbolParams,
    pub extension: ExtensionParams,
}

impl MathParams {
    #[must_use]
    pub fn read(state: &impl MathTypesetState) -> Self {
        // AppG rule 17
        Self {
            text: SizeParams::read(state, MathFontSize::Text),
            script: SizeParams::read(state, MathFontSize::Script),
            script_script: SizeParams::read(state, MathFontSize::ScriptScript),
            delimiter_factor: state.int_param(IntParam::DELIMITER_FACTOR),
            delimiter_shortfall: state.dimen_param(DimenParam::DELIMITER_SHORTFALL),
            null_delimiter_space: state.dimen_param(DimenParam::NULL_DELIMITER_SPACE),
            script_space: state.dimen_param(DimenParam::new(12)),
            thin_mu_skip: state.glue(state.glue_param(GlueParam::new(15))),
            med_mu_skip: state.glue(state.glue_param(GlueParam::new(16))),
            thick_mu_skip: state.glue(state.glue_param(GlueParam::new(17))),
        }
    }

    #[must_use]
    pub const fn for_size(self, size: MathFontSize) -> SizeParams {
        match size {
            MathFontSize::Text => self.text,
            MathFontSize::Script => self.script,
            MathFontSize::ScriptScript => self.script_script,
        }
    }
}

impl SizeParams {
    #[must_use]
    pub fn read(state: &impl MathTypesetState, size: MathFontSize) -> Self {
        let symbols = state.math_family_font(size, 2);
        let extension = state.math_family_font(size, 3);
        Self {
            symbols: SymbolParams {
                math_x_height: state.font_parameter(symbols, 5),
                math_quad: state.font_parameter(symbols, 6),
                num1: state.font_parameter(symbols, 8),
                num2: state.font_parameter(symbols, 9),
                num3: state.font_parameter(symbols, 10),
                denom1: state.font_parameter(symbols, 11),
                denom2: state.font_parameter(symbols, 12),
                sup1: state.font_parameter(symbols, 13),
                sup2: state.font_parameter(symbols, 14),
                sup3: state.font_parameter(symbols, 15),
                sub1: state.font_parameter(symbols, 16),
                sub2: state.font_parameter(symbols, 17),
                sup_drop: state.font_parameter(symbols, 18),
                sub_drop: state.font_parameter(symbols, 19),
                delim1: state.font_parameter(symbols, 20),
                delim2: state.font_parameter(symbols, 21),
                axis_height: state.font_parameter(symbols, 22),
            },
            extension: ExtensionParams {
                default_rule_thickness: state.font_parameter(extension, 8),
                big_op_spacing1: state.font_parameter(extension, 9),
                big_op_spacing2: state.font_parameter(extension, 10),
                big_op_spacing3: state.font_parameter(extension, 11),
                big_op_spacing4: state.font_parameter(extension, 12),
                big_op_spacing5: state.font_parameter(extension, 13),
            },
        }
    }
}
