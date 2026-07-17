use tex_fonts::{MathConstant, MathMetricsSource};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::GlueSpec;
use tex_state::ids::GlueId;
use tex_state::math::MathFontSize;
use tex_state::scaled::Scaled;

use super::MathTypesetState;

/// Mutable state reads needed only while taking an Appendix G parameter snapshot.
pub trait MathParamState: MathTypesetState {
    fn int_param(&self, param: IntParam) -> i32;
    fn dimen_param(&self, param: DimenParam) -> Scaled;
    fn glue_param(&self, param: GlueParam) -> GlueId;
}

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
    pub subscript_top_max: Option<Scaled>,
    pub superscript_bottom_min: Option<Scaled>,
    pub sub_superscript_gap_min: Option<Scaled>,
    pub superscript_bottom_max_with_subscript: Option<Scaled>,
    pub space_after_script: Option<Scaled>,
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
    pub fraction_numerator_gap_min: Option<Scaled>,
    pub fraction_numerator_display_gap_min: Option<Scaled>,
    pub fraction_denominator_gap_min: Option<Scaled>,
    pub fraction_denominator_display_gap_min: Option<Scaled>,
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
    pub bin_op_penalty: i32,
    pub rel_penalty: i32,
}

/// Per-size math parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SizeParams {
    pub symbols: SymbolParams,
    pub extension: ExtensionParams,
}

impl MathParams {
    #[must_use]
    pub fn read(state: &impl MathParamState) -> Self {
        Self {
            // AppG rules 9-18 use the family-2 and family-3 math fontdimens.
            text: SizeParams::read(state, MathFontSize::Text),
            script: SizeParams::read(state, MathFontSize::Script),
            script_script: SizeParams::read(state, MathFontSize::ScriptScript),
            // AppG rule 19.
            delimiter_factor: state.int_param(IntParam::DELIMITER_FACTOR),
            delimiter_shortfall: state.dimen_param(DimenParam::DELIMITER_SHORTFALL),
            // AppG rules 15 and 19.
            null_delimiter_space: state.dimen_param(DimenParam::NULL_DELIMITER_SPACE),
            // AppG rule 18.
            script_space: state.dimen_param(DimenParam::new(12)),
            // AppG rule 20.
            thin_mu_skip: state.glue(state.glue_param(GlueParam::new(15))),
            med_mu_skip: state.glue(state.glue_param(GlueParam::new(16))),
            thick_mu_skip: state.glue(state.glue_param(GlueParam::new(17))),
            // AppG rule 21.
            bin_op_penalty: state.int_param(IntParam::BIN_OP_PENALTY),
            rel_penalty: state.int_param(IntParam::REL_PENALTY),
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
        if let MathMetricsSource::OpenType(math) = state.math_metrics_source(symbols) {
            let c = |constant| math.constant(constant);
            return Self {
                symbols: SymbolParams {
                    math_x_height: c(MathConstant::AccentBaseHeight),
                    math_quad: state.font_parameter(symbols, 6),
                    num1: c(MathConstant::FractionNumeratorDisplayStyleShiftUp),
                    num2: c(MathConstant::FractionNumeratorShiftUp),
                    num3: c(MathConstant::FractionNumeratorShiftUp),
                    denom1: c(MathConstant::FractionDenominatorDisplayStyleShiftDown),
                    denom2: c(MathConstant::FractionDenominatorShiftDown),
                    sup1: c(MathConstant::SuperscriptShiftUp),
                    sup2: c(MathConstant::SuperscriptShiftUp),
                    sup3: c(MathConstant::SuperscriptShiftUpCramped),
                    sub1: c(MathConstant::SubscriptShiftDown),
                    sub2: c(MathConstant::SubscriptShiftDown),
                    sup_drop: c(MathConstant::SuperscriptBaselineDropMax),
                    sub_drop: c(MathConstant::SubscriptBaselineDropMin),
                    delim1: Scaled::from_raw(0),
                    delim2: Scaled::from_raw(0),
                    axis_height: c(MathConstant::AxisHeight),
                    subscript_top_max: Some(c(MathConstant::SubscriptTopMax)),
                    superscript_bottom_min: Some(c(MathConstant::SuperscriptBottomMin)),
                    sub_superscript_gap_min: Some(c(MathConstant::SubSuperscriptGapMin)),
                    superscript_bottom_max_with_subscript: Some(c(
                        MathConstant::SuperscriptBottomMaxWithSubscript,
                    )),
                    space_after_script: Some(c(MathConstant::SpaceAfterScript)),
                },
                extension: ExtensionParams {
                    default_rule_thickness: c(MathConstant::FractionRuleThickness),
                    big_op_spacing1: c(MathConstant::UpperLimitGapMin),
                    big_op_spacing2: c(MathConstant::LowerLimitGapMin),
                    big_op_spacing3: c(MathConstant::UpperLimitBaselineRiseMin),
                    big_op_spacing4: c(MathConstant::LowerLimitBaselineDropMin),
                    big_op_spacing5: Scaled::from_raw(0),
                    fraction_numerator_gap_min: Some(c(MathConstant::FractionNumeratorGapMin)),
                    fraction_numerator_display_gap_min: Some(c(
                        MathConstant::FractionNumeratorDisplayStyleGapMin,
                    )),
                    fraction_denominator_gap_min: Some(c(MathConstant::FractionDenominatorGapMin)),
                    fraction_denominator_display_gap_min: Some(c(
                        MathConstant::FractionDenominatorDisplayStyleGapMin,
                    )),
                },
            };
        }
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
                subscript_top_max: None,
                superscript_bottom_min: None,
                sub_superscript_gap_min: None,
                superscript_bottom_max_with_subscript: None,
                space_after_script: None,
            },
            extension: ExtensionParams {
                default_rule_thickness: state.font_parameter(extension, 8),
                big_op_spacing1: state.font_parameter(extension, 9),
                big_op_spacing2: state.font_parameter(extension, 10),
                big_op_spacing3: state.font_parameter(extension, 11),
                big_op_spacing4: state.font_parameter(extension, 12),
                big_op_spacing5: state.font_parameter(extension, 13),
                fraction_numerator_gap_min: None,
                fraction_numerator_display_gap_min: None,
                fraction_denominator_gap_min: None,
                fraction_denominator_display_gap_min: None,
            },
        }
    }
}
