//! Typed test fixture for pure typesetting kernels.
//!
//! This is deliberately not a runtime facade. It owns only the copied values
//! and page-arena coordinates needed by one pure-kernel test.

use std::collections::HashMap;

use tex_fonts::metrics::ExtensibleRecipe;
use tex_fonts::{CharMetrics, LigKernChar, LigKernCommand, LoadedFont, MathMetricsSource};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, PARAMETER_COUNT};
use tex_state::font::{FontExpansion, NULL_FONT, PdfFontCode};
use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::math::{MATH_FAMILY_COUNT, MathFontSize};
use tex_state::node::Node;
use tex_state::node_arena::{NodeArenaError, NodeCursor, PageListId, PageNodeArena};
use tex_state::scaled::Scaled;

use crate::TypesetState;
use crate::expansion::{FontExpansionError, FontExpansionSpec};
use crate::math::{MathParamState, MathTypesetState};

#[derive(Clone, Copy, Debug)]
pub(crate) struct TestGlueId(usize);

struct TestFont {
    loaded: LoadedFont,
    parameters: Vec<Scaled>,
    skew_char: i32,
    expansion: Option<FontExpansionSpec>,
}

impl TestFont {
    fn new(loaded: LoadedFont) -> Self {
        Self {
            parameters: loaded.parameters().to_vec(),
            loaded,
            skew_char: -1,
            expansion: None,
        }
    }
}

/// Value-only state and typed page owner used by crate-internal kernel tests.
pub(crate) struct TestState {
    pages: PageNodeArena,
    fonts: Vec<TestFont>,
    math_families: [FontId; 3 * MATH_FAMILY_COUNT as usize],
    integers: [i32; PARAMETER_COUNT],
    dimensions: [Scaled; PARAMETER_COUNT],
    glues: Vec<GlueSpec>,
    glue_parameters: [GlueSpec; PARAMETER_COUNT],
    pdf_codes: HashMap<(PdfFontCode, FontId, u8), i32>,
    box_registers: [Option<PageListId>; 256],
}

impl Default for TestState {
    fn default() -> Self {
        Self::new()
    }
}

impl TestState {
    pub(crate) fn new() -> Self {
        let null = LoadedFont::new(
            "nullfont",
            "nullfont",
            [0; 8],
            0,
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            vec![Scaled::from_raw(0); 7],
            tex_fonts::FontMetrics::default(),
        );
        Self {
            pages: PageNodeArena::new(),
            fonts: vec![TestFont::new(null)],
            math_families: [NULL_FONT; 3 * MATH_FAMILY_COUNT as usize],
            integers: [0; PARAMETER_COUNT],
            dimensions: [Scaled::from_raw(0); PARAMETER_COUNT],
            glues: Vec::new(),
            glue_parameters: [GlueSpec::ZERO; PARAMETER_COUNT],
            pdf_codes: HashMap::new(),
            box_registers: [None; 256],
        }
    }

    pub(crate) fn publish_page_nodes(&mut self, nodes: &[Node]) -> PageListId {
        self.pages
            .publish_owned(nodes.to_vec())
            .expect("test nodes contain only fixture-owned children")
    }

    pub(crate) fn publish_page_node_range(
        &mut self,
        nodes: Vec<Node>,
    ) -> tex_state::node_arena::PageNodeRange {
        self.pages
            .publish_range(nodes)
            .expect("test nodes contain only fixture-owned children")
    }

    pub(crate) fn compose_page_node_sequences(
        &mut self,
        inputs: &[tex_state::node_arena::PageNodeSequenceId],
    ) -> tex_state::node_arena::PageNodeSequenceId {
        self.pages
            .compose_sequences(inputs)
            .expect("test sequences belong to fixture page arena")
    }

    pub(crate) fn page_node_list(
        &self,
        list: PageListId,
    ) -> Result<NodeCursor<'_>, NodeArenaError> {
        self.pages
            .node_cursor(list)
            .map_err(|_| NodeArenaError::InvalidList)
    }

    pub(crate) fn intern_font(&mut self, font: LoadedFont) -> FontId {
        let raw = u32::try_from(self.fonts.len()).expect("test font count fits u32");
        let id = FontId::testing_new(raw);
        self.fonts.push(TestFont::new(font));
        id
    }

    pub(crate) fn intern_glue(&mut self, glue: GlueSpec) -> TestGlueId {
        let id = TestGlueId(self.glues.len());
        self.glues.push(glue);
        id
    }

    pub(crate) fn set_math_family_font(
        &mut self,
        size: MathFontSize,
        family: u8,
        font: FontId,
        _global: bool,
    ) {
        let index =
            usize::from(size.index()) * usize::from(MATH_FAMILY_COUNT) + usize::from(family);
        self.math_families[index] = font;
    }

    pub(crate) fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        let index =
            usize::from(size.index()) * usize::from(MATH_FAMILY_COUNT) + usize::from(family);
        self.math_families[index]
    }

    pub(crate) fn set_font_dimen(
        &mut self,
        font: FontId,
        number: u32,
        value: Scaled,
    ) -> Result<(), usize> {
        let Some(index) = number
            .checked_sub(1)
            .and_then(|value| usize::try_from(value).ok())
        else {
            return Err(0);
        };
        let parameters = &mut self.font_mut(font).parameters;
        if index >= parameters.len() {
            parameters.resize(index + 1, Scaled::from_raw(0));
        }
        parameters[index] = value;
        Ok(())
    }

    pub(crate) fn set_font_skew_char(&mut self, font: FontId, value: i32) {
        self.font_mut(font).skew_char = value;
    }

    pub(crate) fn configure_font_expansion(
        &mut self,
        font: FontId,
        expansion: FontExpansion,
    ) -> Result<(), FontExpansionError> {
        let spec = FontExpansionSpec::new(
            i32::from(expansion.stretch),
            i32::from(expansion.shrink),
            i32::from(expansion.step),
            expansion.auto_expand,
        )?;
        self.font_mut(font).expansion = Some(spec);
        Ok(())
    }

    pub(crate) fn set_pdf_font_code(
        &mut self,
        table: PdfFontCode,
        font: FontId,
        code: u8,
        value: i32,
    ) {
        self.pdf_codes.insert((table, font, code), value);
    }

    pub(crate) fn set_int_param(&mut self, parameter: IntParam, value: i32) {
        self.integers[usize::from(parameter.raw())] = value;
    }

    pub(crate) fn set_dimen_param(&mut self, parameter: DimenParam, value: Scaled) {
        self.dimensions[usize::from(parameter.raw())] = value;
    }

    pub(crate) fn set_glue_param(&mut self, parameter: GlueParam, glue: TestGlueId) {
        self.glue_parameters[usize::from(parameter.raw())] = self.glues[glue.0];
    }

    pub(crate) fn assign_page_box_local(&mut self, index: u16, list: PageListId) {
        self.box_registers[usize::from(index)] = Some(list);
    }

    pub(crate) fn copy_box_to_page(&mut self, index: u16) -> Option<PageListId> {
        let source = self.box_registers[usize::from(index)]?;
        let nodes = self
            .pages
            .node_cursor(source)
            .ok()?
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        self.pages.publish_owned(nodes).ok()
    }

    fn font(&self, id: FontId) -> &TestFont {
        &self.fonts[usize::try_from(id.raw()).expect("test font id fits usize")]
    }

    fn font_mut(&mut self, id: FontId) -> &mut TestFont {
        &mut self.fonts[usize::try_from(id.raw()).expect("test font id fits usize")]
    }

    fn default_pdf_font_code(&self, table: PdfFontCode, font: FontId, code: u8) -> i32 {
        match table {
            PdfFontCode::Ef => 1000,
            PdfFontCode::Tag => self
                .font(font)
                .loaded
                .character_metrics(char::from(code))
                .map_or(0, |metrics| match metrics.tag {
                    tex_fonts::metrics::CharTag::None => 0,
                    tex_fonts::metrics::CharTag::LigKern { .. } => 1,
                    tex_fonts::metrics::CharTag::NextLarger(_) => 2,
                    tex_fonts::metrics::CharTag::Extensible(_) => 4,
                }),
            _ => 0,
        }
    }
}

impl TypesetState for TestState {
    fn page_nodes(&self, list: PageListId) -> tex_state::node_arena::NodeCursor<'_> {
        self.pages
            .node_cursor(list)
            .expect("live test page coordinate")
    }

    fn page_node_sequence(
        &self,
        sequence: tex_state::node_arena::PageNodeSequenceId,
    ) -> Option<tex_state::node_arena::NodeCursor<'_>> {
        self.pages.node_cursor(sequence).ok()
    }

    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<CharMetrics> {
        self.font(font).loaded.metrics().character(code)
    }

    fn font_character_metrics(&self, font: FontId, character: char) -> Option<CharMetrics> {
        self.font(font).loaded.character_metrics(character)
    }

    fn font_uses_tfm_metrics(&self, font: FontId) -> bool {
        self.font(font).loaded.uses_tfm_metrics()
    }

    fn font_widths(&self, font: FontId) -> &[Scaled; 256] {
        self.font(font).loaded.metrics().widths()
    }

    fn font_characters(&self, font: FontId) -> &[Option<CharMetrics>] {
        self.font(font).loaded.metrics().characters()
    }

    fn font_parameter_value(&self, font: FontId, number: u32) -> Scaled {
        number
            .checked_sub(1)
            .and_then(|value| usize::try_from(value).ok())
            .and_then(|index| self.font(font).parameters.get(index))
            .copied()
            .unwrap_or_else(|| Scaled::from_raw(0))
    }

    fn pdf_font_code(&self, table: PdfFontCode, font: FontId, code: u8) -> i32 {
        self.pdf_codes
            .get(&(table, font, code))
            .copied()
            .unwrap_or_else(|| self.default_pdf_font_code(table, font, code))
    }

    fn font_kern(&self, font: FontId, left: u8, right: u8) -> Option<Scaled> {
        match self
            .font(font)
            .loaded
            .metrics()
            .lig_kern_command(LigKernChar::Char(left), LigKernChar::Char(right))
        {
            Some(LigKernCommand::Kern(amount)) => Some(amount),
            _ => None,
        }
    }

    fn font_expansion_spec(&self, font: FontId) -> Option<FontExpansionSpec> {
        self.font(font).expansion
    }
}

impl MathTypesetState for TestState {
    fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        self.math_family_font(size, family)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.font_parameter_value(font, u32::from(number))
    }

    fn font_next_larger(&self, font: FontId, code: u8) -> Option<u8> {
        self.font(font).loaded.metrics().next_larger(code)
    }

    fn font_extensible_recipe(&self, font: FontId, code: u8) -> Option<ExtensibleRecipe> {
        self.font(font).loaded.metrics().extensible_recipe(code)
    }

    fn lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        self.font(font)
            .loaded
            .metrics()
            .lig_kern_command(left, right)
    }

    fn font_skew_char(&self, font: FontId) -> i32 {
        self.font(font).skew_char
    }

    fn math_metrics_source(&self, font: FontId) -> MathMetricsSource<'_> {
        self.font(font).loaded.math_metrics_source()
    }
}

impl MathParamState for TestState {
    fn int_param(&self, parameter: IntParam) -> i32 {
        self.integers[usize::from(parameter.raw())]
    }

    fn dimen_param(&self, parameter: DimenParam) -> Scaled {
        self.dimensions[usize::from(parameter.raw())]
    }

    fn glue_param(&self, parameter: GlueParam) -> GlueSpec {
        self.glue_parameters[usize::from(parameter.raw())]
    }
}
