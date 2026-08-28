//! Pure-kernel adapter over one already-admitted execution borrow.

use tex_fonts::metrics::ExtensibleRecipe;
use tex_fonts::{CharMetrics, LigKernChar, LigKernCommand, MathMetricsSource};
use tex_state::CommandContext;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::font::{FontExpansion, PdfFontCode};
use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::math::MathFontSize;
use tex_state::node::Node;
use tex_state::node_arena::{PageListId, PageNodeSequenceId};
use tex_state::scaled::Scaled;
use tex_typeset::TypesetState;
use tex_typeset::expansion::FontExpansionSpec;
use tex_typeset::math::{MathParamState, MathTypesetState};

/// Immutable pure-typesetting view over the episode's admitted state.
pub(crate) struct TypesetContext<'a, 'state, G> {
    state: &'a CommandContext<'state, G>,
}

impl<'a, 'state, G> TypesetContext<'a, 'state, G> {
    pub(crate) const fn new(state: &'a CommandContext<'state, G>) -> Self {
        Self { state }
    }
}

impl<G> TypesetState for TypesetContext<'_, '_, G> {
    fn page_nodes(&self, list: PageListId) -> &[Node] {
        self.state
            .page_nodes(list)
            .expect("typesetting list belongs to the admitted page arena")
    }

    fn page_node_sequence(
        &self,
        sequence: PageNodeSequenceId,
    ) -> Option<tex_state::node_arena::ArenaNodeSequence<'_, tex_state::node_arena::PageLifetime>>
    {
        self.state.page_node_sequence(sequence).ok()
    }

    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<CharMetrics> {
        self.state.font_char_metrics(font, code)
    }

    fn font_character_metrics(&self, font: FontId, character: char) -> Option<CharMetrics> {
        self.state.font_character_metrics(font, character)
    }

    fn font_uses_tfm_metrics(&self, font: FontId) -> bool {
        self.state.font_uses_tfm_metrics(font)
    }

    fn font_widths(&self, font: FontId) -> &[Scaled; 256] {
        self.state.font_widths(font)
    }

    fn font_characters(&self, font: FontId) -> &[Option<CharMetrics>] {
        self.state.font_characters(font)
    }

    fn font_parameter_value(&self, font: FontId, number: u32) -> Scaled {
        self.state.font_parameter(font, number)
    }

    fn pdf_font_code(&self, table: PdfFontCode, font: FontId, code: u8) -> i32 {
        self.state.pdf_font_code(table, font, code)
    }

    fn font_kern(&self, font: FontId, left: u8, right: u8) -> Option<Scaled> {
        match self.state.font_lig_kern_command(
            font,
            LigKernChar::Char(left),
            LigKernChar::Char(right),
        ) {
            Some(LigKernCommand::Kern(amount)) => Some(amount),
            _ => None,
        }
    }

    fn font_expansion_spec(&self, font: FontId) -> Option<FontExpansionSpec> {
        let FontExpansion {
            stretch,
            shrink,
            step,
            auto_expand,
        } = self.state.font_expansion(font)?;
        Some(
            FontExpansionSpec::new(
                i32::from(stretch),
                i32::from(shrink),
                i32::from(step),
                auto_expand,
            )
            .expect("live font expansion settings are validated"),
        )
    }
}

impl<G> MathTypesetState for TypesetContext<'_, '_, G> {
    fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        self.state.math_family_font(size, family)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.state.classic_math_parameter(font, number)
    }

    fn font_next_larger(&self, font: FontId, code: u8) -> Option<u8> {
        self.state.font_next_larger(font, code)
    }

    fn font_extensible_recipe(&self, font: FontId, code: u8) -> Option<ExtensibleRecipe> {
        self.state.font_extensible_recipe(font, code)
    }

    fn lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        self.state.font_lig_kern_command(font, left, right)
    }

    fn font_skew_char(&self, font: FontId) -> i32 {
        self.state.font_skew_char(font)
    }

    fn classic_math_char_metrics(&self, font: FontId, code: u8) -> Option<CharMetrics> {
        self.state.font_char_metrics(font, code)
    }

    fn math_metrics_source(&self, font: FontId) -> MathMetricsSource<'_> {
        self.state.font_math_metrics_source(font)
    }
}

impl<G> MathParamState for TypesetContext<'_, '_, G> {
    fn int_param(&self, parameter: IntParam) -> i32 {
        self.state.int_param(parameter)
    }

    fn dimen_param(&self, parameter: DimenParam) -> Scaled {
        self.state.dimen_param(parameter)
    }

    fn glue_param(&self, parameter: GlueParam) -> GlueSpec {
        self.state
            .glue_param(parameter)
            .map(|id| self.state.glue(id))
            .unwrap_or(GlueSpec::ZERO)
    }
}
