//! Already-admitted, interpretation-neutral command-state borrow.

use crate::InteractionMode;
use crate::definition_arena::{DefinitionId, DefinitionView};
use crate::dependency::{DependencyKey, DependencyRuntime, DependencyValue, TrackedRegionBarrier};
use crate::durable_arena::{GlueId, ProvenanceId, TokenListId};
use crate::env::banks::IntParam;
use crate::env::{AssignmentScope, CodeTableKind, DenseState, StateError};
use crate::font::FontStore;
use crate::glue::GlueSpec;
use crate::hyphenation::HyphenationTable;
use crate::interner::{ControlSequenceKind, Interner, InternerAccessError, Symbol, SymbolId};
use crate::meaning::{Meaning, MeaningWord, ResolvedMeaning};
use crate::node_arena::{
    DurableListId, NodeArenaError, NodeList, PageLifetime, PageListId, PageNodeArena,
};
use crate::page::PageBuilderState;
use crate::pdf::PdfState;
use crate::provenance::OriginRecord;
use crate::scaled::Scaled;
use crate::source_map::SourceMap;
use crate::stores::AdmittedStateMut;
use crate::token::TokenWord;
use crate::world::{JobClock, World};

/// The two line sources reachable by command delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandLineSource<'a> {
    Terminal { prompt: &'a str },
    Stream(crate::world::StreamSlot),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandBoxKind {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoxDimension {
    Width,
    Height,
    Depth,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PenaltyArrayKind {
    InterLine,
    Club,
    Widow,
    DisplayWidow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareMagDiagnostic {
    IllegalMagnification { attempted: i32 },
    IncompatibleMagnification { attempted: i32, retained: i32 },
}

/// One command episode's borrowed session and generation.
///
/// Admission validates coarse owners once. Meaning reads and definition
/// resolution then index the dense bank and definition arena directly.
pub struct CommandContext<'a, G> {
    interner: &'a mut Interner,
    admitted: AdmittedStateMut<'a, G>,
    primitive_names: &'a [String],
    primitive_meanings: &'a [MeaningWord<G>],
    world: &'a mut World,
    dependencies: &'a mut DependencyRuntime,
    fonts: &'a FontStore,
    page_nodes: &'a mut PageNodeArena,
    page: &'a mut PageBuilderState,
    pdf: &'a mut PdfState<G>,
    sources: &'a mut SourceMap,
    hyphenation: &'a HyphenationTable,
    interaction_mode: &'a mut InteractionMode,
    error_context_widths: crate::print::ErrorContextWidths,
}

impl<'a, G> CommandContext<'a, G> {
    pub(super) const fn new(
        interner: &'a mut Interner,
        admitted: AdmittedStateMut<'a, G>,
        primitive_names: &'a [String],
        primitive_meanings: &'a [MeaningWord<G>],
        world: &'a mut World,
        dependencies: &'a mut DependencyRuntime,
        fonts: &'a FontStore,
        page_nodes: &'a mut PageNodeArena,
        page: &'a mut PageBuilderState,
        pdf: &'a mut PdfState<G>,
        sources: &'a mut SourceMap,
        hyphenation: &'a HyphenationTable,
        interaction_mode: &'a mut InteractionMode,
        error_context_widths: crate::print::ErrorContextWidths,
    ) -> Self {
        Self {
            interner,
            admitted,
            primitive_names,
            primitive_meanings,
            world,
            dependencies,
            fonts,
            page_nodes,
            page,
            pdf,
            sources,
            hyphenation,
            interaction_mode,
            error_context_widths,
        }
    }

    pub fn resolve_symbol(&self, symbol: SymbolId) -> Result<&str, InternerAccessError> {
        self.interner.resolve_id(symbol)
    }

    #[inline(always)]
    pub fn meaning(&self, symbol: Symbol) -> ResolvedMeaning<G> {
        self.interner
            .resolve_local(symbol)
            .expect("command symbols belong to the admitted session");
        self.admitted
            .state_ref()
            .meaning(symbol)
            .expect("command symbols are admitted")
    }

    #[inline(always)]
    pub fn meaning_id(&self, symbol: SymbolId) -> Result<ResolvedMeaning<G>, StateError> {
        // `resolve_id` is the admission check. The compact slot is then a
        // direct index for the lifetime of this context.
        self.interner
            .resolve_id(symbol)
            .map_err(|_| StateError::ForeignSession)?;
        self.admitted.state_ref().meaning(symbol.symbol())
    }

    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.interner
            .resolve_local(symbol)
            .expect("command symbols belong to the admitted session")
    }

    pub fn control_sequence_kind(&self, symbol: Symbol) -> ControlSequenceKind {
        self.interner
            .qualify_local(symbol)
            .and_then(|id| self.interner.kind_id(id).ok())
            .expect("command symbols belong to the admitted session")
    }

    #[must_use]
    pub fn active_character_symbol(&self, ch: char) -> Option<Symbol> {
        self.interner.active(ch).map(SymbolId::symbol)
    }

    #[must_use]
    pub fn frozen_primitive_meaning(&self, token: crate::token::Token) -> Option<Meaning> {
        match self.frozen_primitive_resolved(token)? {
            ResolvedMeaning::Static(meaning) => Some(meaning),
            ResolvedMeaning::Macro { .. } => None,
        }
    }

    #[must_use]
    pub fn frozen_primitive_resolved(
        &self,
        token: crate::token::Token,
    ) -> Option<ResolvedMeaning<G>> {
        let crate::token::Token::Frozen(frozen) = token else {
            return None;
        };
        self.primitive_meanings
            .get(usize::from(frozen.primitive_index()?))
            .copied()
            .map(MeaningWord::resolve)
    }

    #[must_use]
    pub fn primitive_name(&self, meaning: Meaning) -> Option<&str> {
        self.primitive_meanings
            .iter()
            .position(|candidate| {
                matches!(candidate.resolve(), ResolvedMeaning::Static(value) if value == meaning)
            })
            .map(|index| self.primitive_names[index].as_str())
    }

    #[must_use]
    pub fn primitive_resolved(&self, name: &str) -> Option<ResolvedMeaning<G>> {
        self.primitive_names
            .iter()
            .position(|candidate| candidate == name)
            .map(|index| self.primitive_meanings[index].resolve())
    }

    #[inline(always)]
    pub fn definition(&self, id: DefinitionId<G>) -> DefinitionView<'_, G> {
        self.admitted.definition(id)
    }

    #[inline(always)]
    pub fn token_list(&self, id: TokenListId<G>) -> &[TokenWord] {
        self.admitted.token_list(id)
    }

    #[inline(always)]
    pub fn glue(&self, id: GlueId<G>) -> GlueSpec {
        self.admitted.glue(id)
    }

    #[inline(always)]
    pub fn provenance(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.admitted.provenance(id)
    }

    #[inline(always)]
    pub fn count(&self, index: u16) -> Result<i32, StateError> {
        self.admitted.state_ref().count(index)
    }

    pub fn assign_count(
        &mut self,
        index: u16,
        value: i32,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted.state().assign_count(index, value, scope)
    }

    #[inline(always)]
    pub fn dimension(&self, index: u16) -> Result<Scaled, StateError> {
        self.admitted.state_ref().dimension(index)
    }

    pub fn assign_dimension(
        &mut self,
        index: u16,
        value: Scaled,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted.state().assign_dimension(index, value, scope)
    }

    #[inline(always)]
    pub fn int_param(&self, parameter: IntParam) -> i32 {
        self.admitted
            .state_ref()
            .integer_parameter(parameter)
            .expect("command parameters are admitted")
    }

    pub fn assign_int_param(
        &mut self,
        parameter: IntParam,
        value: i32,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted
            .state()
            .assign_integer_parameter(parameter, value, scope)
    }

    #[inline(always)]
    pub fn token_register(&self, index: u16) -> Result<Option<TokenListId<G>>, StateError> {
        self.admitted.state_ref().token_register(index)
    }

    pub fn assign_token_register(
        &mut self,
        index: u16,
        value: Option<TokenListId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted
            .state()
            .assign_token_register(index, value, scope)
    }

    #[inline(always)]
    pub fn token_parameter(
        &self,
        parameter: crate::env::banks::TokParam,
    ) -> Result<Option<TokenListId<G>>, StateError> {
        self.admitted.state_ref().token_parameter(parameter)
    }

    pub fn assign_token_parameter(
        &mut self,
        parameter: crate::env::banks::TokParam,
        value: Option<TokenListId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted
            .state()
            .assign_token_parameter(parameter, value, scope)
    }

    #[inline(always)]
    pub fn glue_register(&self, index: u16) -> Result<Option<GlueId<G>>, StateError> {
        self.admitted.state_ref().glue_register(index)
    }

    pub fn assign_glue_register(
        &mut self,
        index: u16,
        value: Option<GlueId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted
            .state()
            .assign_glue_register(index, value, scope)
    }

    pub fn assign_mu_glue_register(
        &mut self,
        index: u16,
        value: Option<GlueId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted
            .state()
            .assign_mu_glue_register(index, value, scope)
    }

    #[inline(always)]
    pub fn box_register(&self, index: u16) -> Result<Option<DurableListId<G>>, StateError> {
        self.admitted.state_ref().box_register(index)
    }

    pub fn assign_box_register(
        &mut self,
        index: u16,
        value: Option<DurableListId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted
            .state()
            .assign_box_register(index, value, scope)
    }

    #[inline(always)]
    pub fn node_list(
        &self,
        id: DurableListId<G>,
    ) -> Result<NodeList<'_, G, GlueId<G>, TokenListId<G>>, NodeArenaError> {
        self.admitted.node_list(id)
    }

    #[inline(always)]
    pub fn code(&self, kind: CodeTableKind, scalar: char) -> Result<i64, StateError> {
        self.admitted.state_ref().code(kind, scalar)
    }

    #[must_use]
    pub fn lccode(&self, scalar: char) -> crate::code_tables::LcCode {
        u32::try_from(self.code(CodeTableKind::Lccode, scalar).unwrap_or(0)).unwrap_or(0)
    }

    #[must_use]
    pub fn uccode(&self, scalar: char) -> crate::code_tables::UcCode {
        u32::try_from(self.code(CodeTableKind::Uccode, scalar).unwrap_or(0)).unwrap_or(0)
    }

    #[must_use]
    pub fn sfcode(&self, scalar: char) -> crate::code_tables::SfCode {
        u16::try_from(self.code(CodeTableKind::Sfcode, scalar).unwrap_or(0)).unwrap_or(0)
    }

    #[must_use]
    pub fn mathcode(&self, scalar: char) -> crate::code_tables::MathCode {
        u32::try_from(self.code(CodeTableKind::Mathcode, scalar).unwrap_or(0)).unwrap_or(0)
    }

    #[must_use]
    pub fn delcode(&self, scalar: char) -> crate::code_tables::DelCode {
        i32::try_from(self.code(CodeTableKind::Delcode, scalar).unwrap_or(-1)).unwrap_or(-1)
    }

    #[must_use]
    pub fn dimen(&self, index: u16) -> Scaled {
        self.dimension(index)
            .unwrap_or_else(|_| Scaled::from_raw(0))
    }

    #[must_use]
    pub fn dimen_param(&self, parameter: crate::env::banks::DimenParam) -> Scaled {
        self.admitted
            .state_ref()
            .dimension_parameter(parameter)
            .unwrap_or_else(|_| Scaled::from_raw(0))
    }

    #[must_use]
    pub fn current_font(&self) -> crate::ids::FontId {
        self.admitted.state_ref().current_font()
    }

    #[must_use]
    pub fn math_family_font(
        &self,
        size: crate::math::MathFontSize,
        family: u8,
    ) -> crate::ids::FontId {
        let size = match size {
            crate::math::MathFontSize::Text => 0,
            crate::math::MathFontSize::Script => 1,
            crate::math::MathFontSize::ScriptScript => 2,
        };
        let index = size * 16 + family;
        self.admitted
            .state_ref()
            .math_family_font(index)
            .unwrap_or(crate::font::NULL_FONT)
    }

    #[must_use]
    pub fn group_lineages(&self) -> Vec<u64> {
        self.admitted
            .state_ref()
            .group_frames()
            .iter()
            .map(|frame| frame.lineage())
            .collect()
    }

    #[must_use]
    pub fn group_frames_from(&self, start: usize) -> Vec<(usize, &'static str, u32)> {
        self.admitted
            .state_ref()
            .group_frames()
            .iter()
            .copied()
            .enumerate()
            .skip(start)
            .map(|(index, frame)| (index + 1, frame.kind().group_text(), frame.entered_line()))
            .collect()
    }

    #[must_use]
    pub fn current_group_values(&self) -> (i32, i32) {
        let frames = self.admitted.state_ref().group_frames();
        let level = i32::try_from(frames.len()).unwrap_or(i32::MAX);
        let kind = frames.last().map_or(0, |frame| frame.kind().etex_code());
        (level, kind)
    }

    #[must_use]
    pub fn box_kind(&self, index: u16) -> Option<CommandBoxKind> {
        let id = self.box_register(index).ok().flatten()?;
        let list = self.node_list(id).ok()?;
        match (list.len(), list.nodes().first()) {
            (1, Some(crate::node::Node::HList(_))) => Some(CommandBoxKind::Horizontal),
            (1, Some(crate::node::Node::VList(_))) => Some(CommandBoxKind::Vertical),
            _ => None,
        }
    }

    #[must_use]
    pub fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        let id = self.box_register(index).ok().flatten()?;
        let list = self.node_list(id).ok()?;
        let node = match (list.len(), list.nodes().first()) {
            (1, Some(crate::node::Node::HList(node) | crate::node::Node::VList(node))) => node,
            _ => return None,
        };
        Some(match dimension {
            BoxDimension::Width => node.width,
            BoxDimension::Height => node.height,
            BoxDimension::Depth => node.depth,
        })
    }

    #[must_use]
    pub fn box_margin_kern(&self, index: u16, side: crate::node::MarginKernSide) -> Option<Scaled> {
        let id = self.box_register(index).ok().flatten()?;
        let list = self.node_list(id).ok()?;
        let children = match (list.len(), list.nodes().first()) {
            (1, Some(crate::node::Node::HList(node))) => list.child(node.children).ok()?,
            _ => return None,
        };
        let candidate = match side {
            crate::node::MarginKernSide::Left => children.nodes().first(),
            crate::node::MarginKernSide::Right => children.nodes().last(),
        };
        Some(match candidate {
            Some(crate::node::Node::MarginKern {
                amount,
                side: candidate_side,
                ..
            }) if *candidate_side == side => *amount,
            _ => Scaled::from_raw(0),
        })
    }

    #[must_use]
    pub fn font_name(&self, id: crate::ids::FontId) -> String {
        self.fonts.get(id).name().to_owned()
    }

    #[must_use]
    pub fn font_external_name(&self, id: crate::ids::FontId) -> &str {
        self.fonts.get(id).name()
    }

    #[must_use]
    pub fn font_size(&self, id: crate::ids::FontId) -> Scaled {
        self.fonts.get(id).size()
    }

    #[must_use]
    pub fn tracked_font_size(&self, id: crate::ids::FontId) -> Scaled {
        self.font_size(id)
    }

    #[must_use]
    pub fn font_design_size(&self, id: crate::ids::FontId) -> Scaled {
        self.fonts.get(id).design_size()
    }

    #[must_use]
    pub fn font_identifier_symbol(&self, id: crate::ids::FontId) -> Option<Symbol> {
        self.fonts.identifier(id).map(SymbolId::symbol)
    }

    #[must_use]
    pub fn font_char_metrics(
        &self,
        id: crate::ids::FontId,
        code: u8,
    ) -> Option<crate::font::CharMetrics> {
        self.fonts.get(id).metrics().character(code)
    }

    #[must_use]
    pub fn font_character_metrics(
        &self,
        id: crate::ids::FontId,
        character: char,
    ) -> Option<crate::font::CharMetrics> {
        self.fonts.get(id).character_metrics(character)
    }

    #[must_use]
    pub fn font_character_exists(&self, id: crate::ids::FontId, character: char) -> bool {
        self.fonts.get(id).character_exists(character)
    }

    #[must_use]
    pub fn font_uses_tfm_metrics(&self, id: crate::ids::FontId) -> bool {
        self.fonts.get(id).uses_tfm_metrics()
    }

    #[must_use]
    pub fn font_widths(&self, id: crate::ids::FontId) -> &[Scaled; 256] {
        self.fonts.get(id).metrics().widths()
    }

    #[must_use]
    pub fn font_characters(&self, id: crate::ids::FontId) -> &[Option<crate::font::CharMetrics>] {
        self.fonts.get(id).metrics().characters()
    }

    #[must_use]
    pub fn font_parameter(&self, id: crate::ids::FontId, number: u32) -> Scaled {
        self.font_dimen(id, number)
    }

    #[must_use]
    pub fn classic_math_parameter(&self, id: crate::ids::FontId, number: u16) -> Scaled {
        self.fonts
            .get(id)
            .classic_math_parameter_override(number)
            .unwrap_or_else(|| self.font_dimen(id, u32::from(number)))
    }

    #[must_use]
    pub fn classic_math_parameter_count(&self, id: crate::ids::FontId) -> usize {
        self.fonts
            .get(id)
            .classic_math_parameter_count_override()
            .unwrap_or_else(|| self.fonts.get(id).parameters().len())
    }

    #[must_use]
    pub fn font_next_larger(&self, id: crate::ids::FontId, code: u8) -> Option<u8> {
        self.fonts.get(id).metrics().next_larger(code)
    }

    #[must_use]
    pub fn font_extensible_recipe(
        &self,
        id: crate::ids::FontId,
        code: u8,
    ) -> Option<crate::font::ExtensibleRecipe> {
        self.fonts.get(id).metrics().extensible_recipe(code)
    }

    #[must_use]
    pub fn font_lig_kern_command(
        &self,
        id: crate::ids::FontId,
        left: crate::font::LigKernChar,
        right: crate::font::LigKernChar,
    ) -> Option<crate::font::LigKernCommand> {
        self.fonts.get(id).metrics().lig_kern_command(left, right)
    }

    #[must_use]
    pub fn font_math_metrics_source(
        &self,
        id: crate::ids::FontId,
    ) -> tex_fonts::MathMetricsSource<'_> {
        self.fonts.get(id).math_metrics_source()
    }

    #[must_use]
    pub fn font_expansion(&self, id: crate::ids::FontId) -> Option<crate::font::FontExpansion> {
        self.fonts.expansion(id)
    }

    #[must_use]
    pub fn font_dimen(&self, id: crate::ids::FontId, number: u32) -> Scaled {
        number
            .checked_sub(1)
            .and_then(|index| self.fonts.get(id).parameters().get(index as usize))
            .copied()
            .unwrap_or_else(|| Scaled::from_raw(0))
    }

    #[must_use]
    pub fn font_dimen_readable(&self, id: crate::ids::FontId, number: u32) -> bool {
        number != 0 && (number as usize) <= self.fonts.get(id).parameters().len()
    }

    #[must_use]
    pub fn font_dimen_writable(&self, id: crate::ids::FontId, number: u32) -> bool {
        self.font_dimen_readable(id, number)
    }

    #[must_use]
    pub fn font_hyphen_char(&self, _id: crate::ids::FontId) -> i32 {
        self.int_param(IntParam::DEFAULT_HYPHEN_CHAR)
    }

    #[must_use]
    pub fn font_skew_char(&self, _id: crate::ids::FontId) -> i32 {
        self.int_param(IntParam::DEFAULT_SKEW_CHAR)
    }

    #[must_use]
    pub fn current_font_parameter(&self, number: u32) -> Scaled {
        self.font_dimen(self.current_font(), number)
    }

    #[must_use]
    pub fn muskip(&self, index: u16) -> Option<GlueId<G>> {
        self.admitted
            .state_ref()
            .mu_glue_register(index)
            .ok()
            .flatten()
    }

    #[must_use]
    pub fn glue_param(&self, parameter: crate::env::banks::GlueParam) -> Option<GlueId<G>> {
        self.admitted
            .state_ref()
            .glue_parameter(parameter)
            .ok()
            .flatten()
    }

    pub fn define_preamble_tabskip(&mut self, value: GlueSpec, global: bool) {
        let id = self
            .admitted
            .allocate_glue(value)
            .expect("preamble tabskip fits durable glue storage");
        self.admitted
            .state()
            .assign_glue_parameter(
                crate::env::banks::GlueParam::TAB_SKIP,
                Some(id),
                if global {
                    crate::AssignmentScope::Global
                } else {
                    crate::AssignmentScope::Local
                },
            )
            .expect("tabskip parameter is admitted");
    }

    pub fn prepare_mag(&self) -> (i32, Option<PrepareMagDiagnostic>) {
        let mag = self.int_param(IntParam::MAG);
        if !(1..=32_768).contains(&mag) {
            (
                1_000,
                Some(PrepareMagDiagnostic::IllegalMagnification { attempted: mag }),
            )
        } else {
            (mag, None)
        }
    }

    #[must_use]
    pub fn internal_integer(&self, integer: crate::meaning::InternalInteger) -> Option<i32> {
        use crate::meaning::InternalInteger;
        Some(match integer {
            InternalInteger::Badness => 0,
            InternalInteger::InputLineNumber => return None,
            InternalInteger::ETeXVersion => 2,
            InternalInteger::PdfTeXVersion => 140,
            InternalInteger::PdfElapsedTime => self.world.pdf_elapsed_time(),
            InternalInteger::PdfRandomSeed => self.world.pdf_random_seed(),
            InternalInteger::PdfShellEscape => match self.world.shell_escape_policy() {
                crate::world::ShellEscapePolicy::Disabled => 0,
                crate::world::ShellEscapePolicy::Enabled => 1,
                crate::world::ShellEscapePolicy::Restricted => 2,
            },
            InternalInteger::PdfLastObject => self.pdf.last_raw_object() as i32,
            InternalInteger::PdfLastAnnot => self.pdf.last_annotation() as i32,
            InternalInteger::PdfLastLink => self.pdf.last_link() as i32,
            InternalInteger::PdfLastXPos => self.pdf.last_position().0.raw(),
            InternalInteger::PdfLastYPos => self.pdf.last_position().1.raw(),
            InternalInteger::PdfLastXForm => self.pdf.last_form() as i32,
            InternalInteger::PdfLastXImage => self
                .pdf
                .last_external_image()
                .map_or(0, |record| record.id().raw() as i32),
            InternalInteger::PdfReturnValue => self.pdf.return_value(),
            InternalInteger::PdfLastXImagePages => self
                .pdf
                .last_external_image()
                .map_or(0, |record| record.metadata().page_count() as i32),
            InternalInteger::PdfLastXImageColorDepth => self
                .pdf
                .last_external_image()
                .map_or(0, |record| i32::from(record.metadata().color_depth())),
            InternalInteger::CurrentGroupLevel
            | InternalInteger::CurrentGroupType
            | InternalInteger::CurrentIfLevel
            | InternalInteger::CurrentIfType
            | InternalInteger::CurrentIfBranch => return None,
            InternalInteger::LastNodeType => self.page.last_node_type(),
        })
    }

    #[must_use]
    pub fn pdf_font_code(
        &self,
        _table: crate::font::PdfFontCode,
        _font: crate::ids::FontId,
        _code: u8,
    ) -> i32 {
        0
    }

    #[must_use]
    pub(crate) const fn state(&self) -> &DenseState<G> {
        self.admitted.state_ref()
    }

    #[must_use]
    pub fn untracked_int_param(&self, parameter: IntParam) -> i32 {
        self.int_param(parameter)
    }

    #[must_use]
    pub fn untracked_catcode(&self, ch: char) -> crate::token::Catcode {
        self.catcode(ch)
    }

    pub fn observe_command_projection(&mut self, key: DependencyKey, value: DependencyValue) {
        self.dependencies.record(key, value);
    }

    #[must_use]
    pub fn tracked_region_is_active(&self) -> bool {
        self.dependencies.is_recording()
    }

    pub fn unsupported_command_state(&mut self) {
        self.dependencies
            .poison(TrackedRegionBarrier::UnsupportedCommandState);
    }

    pub fn unsupported_host_capability(&mut self) {
        self.dependencies
            .poison(TrackedRegionBarrier::UnsupportedHostCapability);
    }

    #[must_use]
    pub fn job_clock(&mut self) -> JobClock {
        self.world.job_clock()
    }

    #[must_use]
    pub fn interaction_permits_terminal_input(&self) -> bool {
        matches!(
            *self.interaction_mode,
            InteractionMode::Scroll | InteractionMode::ErrorStop
        )
    }

    #[must_use]
    pub fn interaction_mode_value(&self) -> i32 {
        match *self.interaction_mode {
            InteractionMode::Batch => 0,
            InteractionMode::Nonstop => 1,
            InteractionMode::Scroll => 2,
            InteractionMode::ErrorStop => 3,
        }
    }

    #[must_use]
    pub fn read_stream_at_eof(&self, slot: crate::world::StreamSlot) -> bool {
        self.world.input_stream_eof(slot)
    }

    pub fn input_ln(&mut self, source: CommandLineSource<'_>) -> Option<String> {
        match source {
            CommandLineSource::Terminal { prompt } => {
                if !prompt.is_empty() {
                    self.printer().print(prompt);
                }
                let line = self.world.read_terminal_line().ok().flatten()?;
                self.world.echo_terminal_input(&line);
                Some(line)
            }
            CommandLineSource::Stream(slot) => self.world.read_stream_line(slot).ok().flatten(),
        }
    }

    pub fn record_warning_history(&mut self) {
        self.world.error_channel_mut().record_warning_history();
    }

    pub fn print_file_open(&mut self, name: &str) {
        let mut printer = self.printer();
        let term_offset = printer.terminal_offset();
        if term_offset + name.chars().count() > printer.max_print_line() - 2 {
            printer.print_ln();
        } else if term_offset > 0 || printer.log_offset() > 0 {
            printer.print_char(' ');
        }
        printer.print_char('(').print(name);
        self.world.file_framing_mut().open();
    }

    pub fn print_file_close(&mut self) {
        self.printer().print_char(')');
        self.world.file_framing_mut().close();
    }

    pub fn make_string_pool_string(&mut self, _value: &str) {
        self.unsupported_command_state();
    }

    pub fn slow_make_string_pool_string(&mut self, _value: &str) {
        self.unsupported_command_state();
    }

    pub fn register_source(
        &mut self,
        source: crate::input::SourceId,
        descriptor: crate::source_map::SourceDescriptor,
    ) -> Result<crate::source_map::SourcePos, crate::source_map::SourceMapError> {
        self.sources
            .register_with_line_starts(source, descriptor, std::sync::Arc::from([0_usize]))
    }

    #[must_use]
    pub fn source_token_origin(
        &self,
        source: crate::input::SourceId,
        start: u64,
        end: u64,
    ) -> crate::token::OriginId {
        self.sources
            .registered_source(source)
            .and_then(|registered| registered.direct_origin(start, end))
            .unwrap_or(crate::token::OriginId::UNKNOWN)
    }

    #[must_use]
    pub fn source_range_origin(
        &self,
        source: crate::input::SourceId,
        start: u64,
        end: u64,
    ) -> crate::token::OriginId {
        self.source_token_origin(source, start, end)
    }

    #[must_use]
    pub fn hyphenation_patterns_open(&self) -> bool {
        self.hyphenation.patterns_open()
    }

    #[must_use]
    pub fn saved_hyphenation_code(&self, language: u8, ch: char) -> Option<Option<char>> {
        self.hyphenation.saved_hyphen_code(language, ch)
    }

    #[must_use]
    pub fn contains_hyphenation_pattern_for_language(
        &self,
        language: u8,
        letters: &[char],
    ) -> bool {
        self.hyphenation
            .contains_pattern_for_language(language, letters)
    }

    pub fn pdf_uniform_deviate(&mut self, bound: i32) -> i32 {
        self.world.pdf_uniform_deviate(bound)
    }

    pub fn pdf_normal_deviate(&mut self) -> i32 {
        self.world.pdf_normal_deviate()
    }

    #[must_use]
    pub fn pdf_external_image(
        &self,
        id: crate::PdfExternalImageId,
    ) -> Option<crate::PdfExternalImageMetadata> {
        self.pdf.external_image(id)
    }

    #[must_use]
    pub fn pdf_form_resource(&self, object: u32) -> Option<u32> {
        self.pdf.form(object).map(|form| form.resource())
    }

    #[must_use]
    pub fn pdf_page_object(&self, page: u32) -> Option<u32> {
        page.checked_sub(1)
            .and_then(|index| self.pdf.pages().get(index as usize))
            .map(crate::PdfPageRecord::page_object)
    }

    pub fn set_pdf_match_state(
        &mut self,
        haystack: Vec<u8>,
        captures: Vec<Option<(u32, u32)>>,
        slots: u32,
        matched: bool,
    ) {
        self.pdf.set_match(haystack, captures, slots, matched);
    }

    #[must_use]
    pub fn pdf_match_capture(&self, index: u32) -> Option<(u32, &[u8])> {
        self.pdf.match_capture(index)
    }

    pub fn allocate_pdf_color_stack(
        &mut self,
        mode: crate::PdfColorStackMode,
        restore_at_page_start: bool,
        initial: Vec<u8>,
    ) -> Result<u32, crate::PdfColorStackCapacityError> {
        self.pdf
            .allocate_color_stack(mode, restore_at_page_start, initial)
    }

    /// Publishes one complete page-lifetime list inside this admitted episode.
    pub fn publish_page_nodes(&mut self, nodes: Vec<crate::node::Node>) -> PageListId {
        self.page_nodes
            .publish(nodes)
            .expect("page construction contains only live page-arena children")
    }

    /// Resolves one page-lifetime list while the admitted context is live.
    pub fn page_node_list(
        &self,
        list: PageListId,
    ) -> Result<NodeList<'_, PageLifetime>, NodeArenaError> {
        self.page_nodes.get(list)
    }

    /// Resolves the node slice consumed by pure typesetting kernels.
    pub fn page_nodes(&self, list: PageListId) -> Result<&[crate::node::Node], NodeArenaError> {
        Ok(self.page_nodes.get(list)?.nodes())
    }

    #[must_use]
    pub fn page_dimension(&self, dimension: crate::page::PageDimension) -> Scaled {
        self.page.dimension(dimension, false)
    }

    pub fn set_page_dimension(&mut self, dimension: crate::page::PageDimension, value: Scaled) {
        self.page.set_dimension(dimension, value);
    }

    #[must_use]
    pub fn page_integer(&self, integer: crate::page::PageInteger) -> i32 {
        self.page.integer(integer)
    }

    pub fn set_page_integer(&mut self, integer: crate::page::PageInteger, value: i32) {
        self.page.set_integer(integer, value);
    }

    #[must_use]
    pub fn page_contents(&self) -> crate::page::PageContents {
        self.page.contents()
    }

    pub fn set_page_contents(&mut self, contents: crate::page::PageContents) {
        self.page.set_contents(contents);
    }

    #[must_use]
    pub fn page_max_depth(&self) -> Scaled {
        self.page.page_max_depth()
    }

    #[must_use]
    pub fn insert_penalties(&self) -> i32 {
        self.page.insert_penalties()
    }

    #[must_use]
    pub fn least_page_cost(&self) -> i32 {
        self.page.least_page_cost()
    }

    pub fn freeze_page_specs(
        &mut self,
        contents: crate::page::PageContents,
        vsize: Scaled,
        max_depth: Scaled,
    ) {
        self.page.freeze_specs(contents, vsize, max_depth);
    }

    pub fn record_best_page_break(&mut self, index: usize, best_size: Scaled, cost: i32) {
        self.page.record_best_break(index, best_size, cost);
    }

    pub fn record_page_fire_up(&mut self, trigger_index: usize) {
        self.page.record_fire_up(trigger_index);
    }

    #[must_use]
    pub fn page_fire_up(&self) -> Option<crate::page::PageFireUp> {
        self.page.fire_up()
    }

    pub fn start_page_after_output(&mut self) {
        self.page.start_page_after_output();
    }

    #[must_use]
    pub fn page_contributions(&self) -> &std::collections::VecDeque<crate::node::Node> {
        self.page.contribution()
    }

    pub fn append_page_contribution(&mut self, node: crate::node::Node) {
        self.page.push_contribution(node);
    }

    pub fn prepend_page_contribution(&mut self, node: crate::node::Node) {
        self.page.prepend_contribution(node);
    }

    pub fn prepend_page_contributions(&mut self, nodes: Vec<crate::node::Node>) {
        self.page.prepend_contributions(nodes);
    }

    pub fn remove_page_contribution_range(
        &mut self,
        range: std::ops::RangeInclusive<usize>,
    ) -> Vec<crate::node::Node> {
        self.page.remove_contribution_range(range)
    }

    #[must_use]
    pub fn page_contribution_front(&self) -> Option<&crate::node::Node> {
        self.page.contribution_front()
    }

    #[must_use]
    pub fn page_contribution_second(&self) -> Option<&crate::node::Node> {
        self.page.contribution_second()
    }

    pub fn pop_page_contribution_front(&mut self) -> Option<crate::node::Node> {
        self.page.pop_contribution_front()
    }

    #[must_use]
    pub fn current_page_len(&self) -> usize {
        self.page.current_page_len()
    }

    #[must_use]
    pub fn current_page_tail(&self) -> Option<&crate::node::Node> {
        self.page.current_page_tail()
    }

    pub fn push_current_page_node(&mut self, node: crate::node::Node) {
        self.page.push_current_page(node);
    }

    pub fn take_current_page_prefix(
        &mut self,
        split_index: usize,
    ) -> (Vec<crate::node::Node>, Vec<crate::node::Node>) {
        self.page.take_current_page_prefix(split_index)
    }

    pub fn update_page_last_from_node(&mut self, node: &crate::node::Node) {
        self.page.update_last_from_node(node);
    }

    #[must_use]
    pub fn page_has_last_glue(&self) -> bool {
        self.page.has_last_glue()
    }

    #[must_use]
    pub fn page_last_skip(&self) -> Option<GlueSpec> {
        self.page.last_skip_ref()
    }

    #[must_use]
    pub fn page_last_penalty(&self) -> i32 {
        self.page.last_penalty()
    }

    #[must_use]
    pub fn page_last_kern(&self) -> Scaled {
        self.page.last_kern()
    }

    #[must_use]
    pub fn page_last_node_type(&self) -> i32 {
        self.page.last_node_type()
    }

    pub fn push_page_discard(&mut self, node: crate::node::Node) {
        self.page.push_page_discard(node);
    }

    pub fn take_page_discards(&mut self) -> Vec<crate::node::Node> {
        self.page.take_page_discards()
    }

    pub fn clear_page_discards(&mut self) {
        self.page.clear_page_discards();
    }

    pub fn set_split_discards(&mut self, nodes: Vec<crate::node::Node>) {
        self.page.set_split_discards(nodes);
    }

    pub fn take_split_discards(&mut self) -> Vec<crate::node::Node> {
        self.page.take_split_discards()
    }

    pub fn clear_split_discards(&mut self) {
        self.page.clear_split_discards();
    }

    #[must_use]
    pub fn page_insertions(&self) -> &[crate::page::PageInsertion] {
        self.page.page_insertions()
    }

    #[must_use]
    pub fn page_insertion(&self, class: u16) -> Option<crate::page::PageInsertion> {
        self.page.page_insertion(class)
    }

    pub fn upsert_page_insertion(&mut self, insertion: crate::page::PageInsertion) {
        self.page.upsert_page_insertion(insertion);
    }

    #[must_use]
    pub fn page_mark(&self, mark: crate::page::PageMark) -> crate::node::NodeTokenList {
        self.page.mark(mark)
    }

    #[must_use]
    pub fn page_mark_value(
        &self,
        mark: crate::page::PageMark,
    ) -> Option<&crate::node::NodeTokenList> {
        self.page.mark_value(mark)
    }

    #[must_use]
    pub fn page_mark_class_value(
        &self,
        mark: crate::page::PageMark,
        class: u16,
    ) -> Option<&crate::node::NodeTokenList> {
        self.page.mark_class_value(mark, class)
    }

    #[must_use]
    pub fn paragraph_shape_len(&self) -> usize {
        0
    }

    #[must_use]
    pub fn paragraph_shape_dimension(&self, _line: i32, _width: bool) -> Scaled {
        Scaled::from_raw(0)
    }

    #[must_use]
    pub fn penalty_array_value(&self, kind: PenaltyArrayKind, index: i32) -> i32 {
        if index < 0 {
            return 0;
        }
        let parameter = match kind {
            PenaltyArrayKind::InterLine => IntParam::INTERLINE_PENALTY,
            PenaltyArrayKind::Club => IntParam::CLUB_PENALTY,
            PenaltyArrayKind::Widow => IntParam::WIDOW_PENALTY,
            PenaltyArrayKind::DisplayWidow => IntParam::DISPLAY_WIDOW_PENALTY,
        };
        if index == 0 {
            1
        } else {
            self.int_param(parameter)
        }
    }

    pub fn observe_command_rendering_dependencies(&mut self) {
        self.unsupported_command_state();
    }

    pub fn append_selector_string_text(&self, raw: &str, text: &mut String) {
        let newline = u32::try_from(self.int_param(IntParam::NEWLINE_CHAR))
            .ok()
            .filter(|&value| value < 256)
            .and_then(char::from_u32);
        for ch in raw.chars() {
            if Some(ch) == newline {
                text.push('\n');
            } else {
                crate::token_show::append_tex_print_char(ch, text);
            }
        }
    }

    pub fn append_token_show_text(&self, token: crate::token::Token, text: &mut String) {
        crate::token_show::append_token_show_text(self, token, text);
    }

    pub fn append_token_string_text(&self, token: crate::token::Token, text: &mut String) {
        crate::token_show::append_token_string_text(self, token, text);
    }

    pub fn append_token_selector_text(&self, token: crate::token::Token, text: &mut String) {
        let newline = u32::try_from(self.int_param(IntParam::NEWLINE_CHAR))
            .ok()
            .filter(|&value| value < 256)
            .and_then(char::from_u32);
        crate::token_show::append_token_selector_text(self, token, newline, text);
    }

    #[must_use]
    pub const fn error_context_widths(&self) -> crate::print::ErrorContextWidths {
        self.error_context_widths
    }

    pub fn begin_diagnostic(&mut self) -> crate::diagnostic::Diagnostic<'_, G> {
        let tracing_online = self.int_param(IntParam::TRACING_ONLINE);
        let newline = self.int_param(IntParam::NEWLINE_CHAR);
        let escape = self.int_param(IntParam::ESCAPE_CHAR);
        crate::diagnostic::Diagnostic::from_parts(
            self.world,
            self.interaction_mode,
            self.error_context_widths,
            tracing_online,
            newline,
            escape,
        )
    }

    pub fn printer(&mut self) -> crate::print::Printer<'_, G> {
        let newline = self.int_param(IntParam::NEWLINE_CHAR);
        let escape = self.int_param(IntParam::ESCAPE_CHAR);
        let selector = crate::print::Selector::for_interaction(*self.interaction_mode);
        crate::print::Printer::from_parts(
            self.world,
            self.interaction_mode,
            newline,
            escape,
            self.error_context_widths.max_print_line(),
            selector,
        )
    }

    pub fn print_err(&mut self, text: &str) -> crate::print::ErrorReport<'_, G> {
        let newline = self.int_param(IntParam::NEWLINE_CHAR);
        let escape = self.int_param(IntParam::ESCAPE_CHAR);
        crate::print::ErrorReport::begin_from_parts(
            self.world,
            self.interaction_mode,
            self.error_context_widths,
            newline,
            escape,
            text,
        )
    }

    pub fn resume_error_report(
        &mut self,
        deferred: crate::print::DeferredErrorReport,
    ) -> crate::print::ErrorReport<'_, G> {
        let newline = self.int_param(IntParam::NEWLINE_CHAR);
        let escape = self.int_param(IntParam::ESCAPE_CHAR);
        crate::print::ErrorReport::resume_from_parts(
            self.world,
            self.interaction_mode,
            self.error_context_widths,
            newline,
            escape,
            deferred,
        )
    }

    pub fn take_error_recovery_request(&mut self) -> Option<crate::print::ErrorRecoveryRequest> {
        self.world.error_channel_mut().take_recovery_request()
    }

    pub fn continue_error_stop_dialog(&mut self, context: &str) -> crate::print::ErrorOutcome {
        let newline = self.int_param(IntParam::NEWLINE_CHAR);
        let escape = self.int_param(IntParam::ESCAPE_CHAR);
        crate::print::ErrorReport::<G>::continue_from_parts(
            self.world,
            self.interaction_mode,
            self.error_context_widths,
            newline,
            escape,
            context,
        )
    }

    #[must_use]
    pub fn catcode(&self, ch: char) -> crate::token::Catcode {
        let raw = self
            .code(CodeTableKind::Catcode, ch)
            .unwrap_or(crate::token::Catcode::Other as i64);
        u8::try_from(raw)
            .ok()
            .and_then(crate::token::Catcode::from_raw)
            .unwrap_or(crate::token::Catcode::Other)
    }

    #[must_use]
    pub fn frozen_endv_token(&self) -> crate::token::Token {
        crate::token::Token::frozen_endv()
    }

    #[must_use]
    pub fn frozen_end_template_token(&self) -> crate::token::Token {
        crate::token::Token::frozen_end_template()
    }

    #[must_use]
    pub fn frozen_primitive_name(&self, token: crate::token::Token) -> Option<&str> {
        if token.is_frozen_end_template() || token.is_frozen_endv() {
            return Some("endtemplate");
        }
        if token.is_frozen_relax() {
            return Some("relax");
        }
        let crate::token::Token::Frozen(frozen) = token else {
            return None;
        };
        self.primitive_names
            .get(usize::from(frozen.primitive_index()?))
            .map(String::as_str)
    }

    #[must_use]
    pub fn primitive_token(&self, name: &str) -> Option<crate::token::Token> {
        let index = self
            .primitive_names
            .iter()
            .position(|candidate| candidate == name)?;
        Some(crate::token::Token::frozen_primitive(
            u16::try_from(index).ok()?,
        ))
    }

    #[must_use]
    pub fn symbol(&self, name: &str) -> Option<Symbol> {
        self.interner.known(name).map(SymbolId::symbol)
    }

    #[must_use]
    pub fn known_control_sequence(&self, name: &str) -> Option<Symbol> {
        self.symbol(name)
    }

    fn intern_symbol(&mut self, id: Result<SymbolId, crate::interner::InternerError>) -> Symbol {
        let id = id.expect("command control-sequence interning stays within session budget");
        self.admitted
            .state()
            .admit_symbol(id.symbol())
            .expect("interned symbol fits the meaning bank");
        id.symbol()
    }

    pub fn intern_control_sequence(&mut self, name: &str) -> Symbol {
        let id = self.interner.intern(name);
        self.intern_symbol(id)
    }

    pub fn intern_hash_control_sequence(&mut self, name: &str) -> Symbol {
        let id = self.interner.intern_hash(name);
        self.intern_symbol(id)
    }

    pub fn intern_internal_control_sequence(&mut self, name: &str) -> Symbol {
        let id = self.interner.intern_internal(name);
        self.intern_symbol(id)
    }

    pub fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        if let Some(symbol) = self.symbol(name) {
            return symbol;
        }
        let symbol = self.intern_control_sequence(name);
        self.admitted
            .state()
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::Relax),
                crate::AssignmentScope::Local,
            )
            .expect("new control sequence is admitted");
        symbol
    }

    pub fn set_provisional_meaning(&mut self, symbol: Symbol, meaning: Meaning, global: bool) {
        self.admitted
            .state()
            .assign_meaning(
                symbol,
                MeaningWord::from_static(meaning),
                if global {
                    crate::AssignmentScope::Global
                } else {
                    crate::AssignmentScope::Local
                },
            )
            .expect("provisional meaning targets admitted state");
    }
}

impl<G> crate::token_show::TokenDisplayState for CommandContext<'_, G> {
    fn display_resolve(&self, symbol: Symbol) -> Option<&str> {
        Some(self.resolve(symbol))
    }

    fn display_control_sequence_kind(&self, symbol: Symbol) -> Option<ControlSequenceKind> {
        Some(self.control_sequence_kind(symbol))
    }

    fn display_catcode(&self, ch: char) -> crate::token::Catcode {
        self.catcode(ch)
    }

    fn display_frozen_primitive_name(&self, token: crate::token::Token) -> Option<&str> {
        self.frozen_primitive_name(token)
    }

    fn display_escape_char(&self) -> i32 {
        self.int_param(IntParam::ESCAPE_CHAR)
    }
}
