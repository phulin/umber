//! Already-admitted, interpretation-neutral command-state borrow.

use crate::InteractionMode;
use crate::definition_arena::{DefinitionId, DefinitionView};
use crate::dependency::{DependencyKey, DependencyRuntime, DependencyValue, TrackedRegionBarrier};
use crate::durable_arena::{DurableAllocationError, GlueId, ProvenanceId, TokenListId};
use crate::env::banks::IntParam;
use crate::env::{AssignmentScope, CodeTableKind, DenseState, StateError};
use crate::font::FontStore;
use crate::glue::GlueSpec;
use crate::hyphenation::{ExceptionSpec, HyphenationTable, PatternSpec};
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

/// A font identifier supplied either as a qualified retained spelling or as
/// an admitted command symbol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontIdentifier {
    Symbol(Symbol),
    Qualified(SymbolId),
}

impl From<Symbol> for FontIdentifier {
    fn from(value: Symbol) -> Self {
        Self::Symbol(value)
    }
}

impl From<SymbolId> for FontIdentifier {
    fn from(value: SymbolId) -> Self {
        Self::Qualified(value)
    }
}

impl PenaltyArrayKind {
    const fn storage(self) -> crate::env::banks::TokParam {
        match self {
            Self::InterLine => crate::env::banks::TokParam::INTER_LINE_PENALTIES_INTERNAL,
            Self::Club => crate::env::banks::TokParam::CLUB_PENALTIES_INTERNAL,
            Self::Widow => crate::env::banks::TokParam::WIDOW_PENALTIES_INTERNAL,
            Self::DisplayWidow => crate::env::banks::TokParam::DISPLAY_WIDOW_PENALTIES_INTERNAL,
        }
    }

    /// Identifies the private token-parameter cell which backs one e-TeX
    /// penalty array.
    ///
    /// The cell identity is needed by the synchronous group-restoration
    /// renderer after the journal has restored its live value. The payload
    /// remains private durable storage; callers receive only the semantic
    /// array kind.
    #[must_use]
    pub const fn from_storage_parameter(parameter: crate::env::banks::TokParam) -> Option<Self> {
        match parameter.raw() {
            raw if raw == Self::InterLine.storage().raw() => Some(Self::InterLine),
            raw if raw == Self::Club.storage().raw() => Some(Self::Club),
            raw if raw == Self::Widow.storage().raw() => Some(Self::Widow),
            raw if raw == Self::DisplayWidow.storage().raw() => Some(Self::DisplayWidow),
            _ => None,
        }
    }
}

/// One detached indent/width pair in TeX's current `\parshape` value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParagraphShapeLine {
    pub indent: Scaled,
    pub width: Scaled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareMagDiagnostic {
    IllegalMagnification { attempted: i32 },
    IncompatibleMagnification { attempted: i32, retained: i32 },
}

/// Detached TeX-shaped resource counters for terminal job reporting.
///
/// This value contains no store, generation, or host handle. Execution-owned
/// stack maxima may be filled by the caller after detachment because those
/// stacks do not belong to `tex-state`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineUsageStatistics {
    pub strings: usize,
    pub string_capacity: usize,
    pub string_characters: usize,
    pub string_character_capacity: usize,
    pub memory_words: usize,
    pub memory_word_capacity: usize,
    pub control_sequences: usize,
    pub font_info_words: usize,
    pub fonts: usize,
    pub hyphenation_exceptions: usize,
    pub hyphenation_exception_capacity: usize,
    pub input_stack: usize,
    pub nest_stack: usize,
    pub parameter_stack: usize,
    pub buffer_stack: usize,
    pub save_stack: usize,
}

/// One explicit retained-string accounting delta.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedStringAllocation {
    pub strings: usize,
    pub characters: usize,
}

impl RetainedStringAllocation {
    #[must_use]
    pub const fn one(value: &str) -> Self {
        Self {
            strings: 1,
            characters: value.len(),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct EngineUsageRuntime {
    retained_strings: usize,
    retained_characters: usize,
}

impl EngineUsageRuntime {
    fn record_retained_strings(&mut self, allocation: RetainedStringAllocation) {
        self.retained_strings = self.retained_strings.saturating_add(allocation.strings);
        self.retained_characters = self
            .retained_characters
            .saturating_add(allocation.characters);
    }
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
    fonts: &'a mut FontStore,
    page_nodes: &'a mut PageNodeArena,
    page: &'a mut PageBuilderState,
    pdf: &'a mut PdfState<G>,
    sources: &'a mut SourceMap,
    hyphenation: &'a mut HyphenationTable,
    interaction_mode: &'a mut InteractionMode,
    prepared_mag: &'a mut Option<i32>,
    error_context_widths: crate::print::ErrorContextWidths,
    engine_usage: &'a mut EngineUsageRuntime,
    font_info_capacity: usize,
}

pub(super) struct CommandContextParts<'a, G> {
    pub interner: &'a mut Interner,
    pub admitted: AdmittedStateMut<'a, G>,
    pub primitive_names: &'a [String],
    pub primitive_meanings: &'a [MeaningWord<G>],
    pub world: &'a mut World,
    pub dependencies: &'a mut DependencyRuntime,
    pub fonts: &'a mut FontStore,
    pub page_nodes: &'a mut PageNodeArena,
    pub page: &'a mut PageBuilderState,
    pub pdf: &'a mut PdfState<G>,
    pub sources: &'a mut SourceMap,
    pub hyphenation: &'a mut HyphenationTable,
    pub interaction_mode: &'a mut InteractionMode,
    pub prepared_mag: &'a mut Option<i32>,
    pub error_context_widths: crate::print::ErrorContextWidths,
    pub engine_usage: &'a mut EngineUsageRuntime,
    pub font_info_capacity: usize,
}

impl<'a, G> CommandContext<'a, G> {
    pub(super) fn new(parts: CommandContextParts<'a, G>) -> Self {
        let CommandContextParts {
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
            prepared_mag,
            error_context_widths,
            engine_usage,
            font_info_capacity,
        } = parts;
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
            prepared_mag,
            error_context_widths,
            engine_usage,
            font_info_capacity,
        }
    }

    /// Detaches the state-owned portion of TeX82's terminal usage report.
    #[must_use]
    pub fn detach_engine_usage_statistics(&self) -> EngineUsageStatistics {
        let fonts = self.fonts.len();
        let hyphenation = self.hyphenation.exception_usage();
        EngineUsageStatistics {
            strings: self.engine_usage.retained_strings,
            string_capacity: 15_000_usize.saturating_sub(1_027),
            string_characters: self.engine_usage.retained_characters,
            string_character_capacity: 125_000_usize.saturating_sub(106_808),
            memory_words: 0,
            memory_word_capacity: 250_000,
            control_sequences: self.interner.multiletter_len(),
            font_info_words: self.admitted.state_ref().font_parameter_words(),
            fonts: fonts.saturating_sub(1),
            hyphenation_exceptions: hyphenation.occupied,
            hyphenation_exception_capacity: hyphenation.capacity,
            input_stack: 0,
            nest_stack: 0,
            parameter_stack: 0,
            buffer_stack: 0,
            save_stack: 0,
        }
    }

    /// Records only the scalar accounting effect of strings retained by an
    /// execution or host owner. No string bytes enter semantic state.
    pub fn record_retained_strings(&mut self, allocation: RetainedStringAllocation) {
        self.engine_usage.record_retained_strings(allocation);
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

    /// Assigns one already-resolved static or generation-local macro meaning.
    pub fn assign_resolved_meaning(
        &mut self,
        symbol: Symbol,
        meaning: ResolvedMeaning<G>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        let word = match meaning {
            ResolvedMeaning::Static(Meaning::Font(font)) => {
                self.validate_font_root(font)?;
                MeaningWord::from_static(Meaning::Font(font))
            }
            ResolvedMeaning::Static(meaning) => MeaningWord::from_static(meaning),
            ResolvedMeaning::Macro { flags, definition } => {
                MeaningWord::macro_definition(flags, definition)
            }
        };
        self.admitted.state().assign_meaning(symbol, word, scope)
    }

    fn validate_font_root(&self, font: crate::ids::FontId) -> Result<(), StateError> {
        self.fonts
            .contains(font)
            .then_some(())
            .ok_or(StateError::ForeignSession)
    }

    fn assert_live_node_font_roots(&self, node: &crate::node::Node) {
        node.visit_fonts(|font| {
            assert!(
                self.fonts.contains(font),
                "durable node contains a font outside the admitted timeline"
            );
        });
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

    /// Interns TeX82's `active_base + c` definition cell on first use.
    ///
    /// Raw delivery can resolve an already-defined active character without
    /// allocation. Definition targets are different: `get_r_token` must make
    /// the cell addressable even when the active character has never had a
    /// meaning before.
    pub fn intern_active_character(&mut self, ch: char) -> Symbol {
        let id = self.interner.intern_active(ch);
        self.intern_symbol(id)
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

    pub fn allocate_glue(&mut self, value: GlueSpec) -> Result<GlueId<G>, DurableAllocationError> {
        self.admitted.allocate_glue(value)
    }

    pub fn allocate_token_list(
        &mut self,
        words: &[TokenWord],
    ) -> Result<TokenListId<G>, DurableAllocationError> {
        self.admitted.allocate_token_list(words)
    }

    #[inline(always)]
    pub fn provenance(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.admitted.provenance(id)
    }

    /// Admits one compact command origin into the generation-typed
    /// provenance domain, then detaches all presentation before returning.
    /// The returned value contains no coordinate, source-map row, or backing
    /// owner and may safely survive rollback of the originating command.
    pub fn detach_diagnostic_origin(
        &mut self,
        origin: crate::token::OriginId,
        request: crate::DiagnosticOriginRequest<'_>,
    ) -> Result<crate::DetachedOriginDiagnostic, DurableAllocationError> {
        let coordinate = match origin.decode() {
            crate::token::OriginEncoding::Arena(index) => {
                match self.admitted.provenance_coordinate_at(index) {
                    Some(coordinate) => coordinate,
                    None => self
                        .admitted
                        .allocate_provenance(OriginRecord::UnknownBootstrap)?,
                }
            }
            crate::token::OriginEncoding::DirectSource(position) => {
                let region = self.sources.region_for_backed_position(position);
                let span = region
                    .and_then(|region| {
                        let next = position.raw().checked_add(1)?;
                        (next <= region.anchor().raw())
                            .then(|| crate::source_map::SourcePos::from_raw_for_store(next))
                    })
                    .and_then(|hi| self.sources.span(position, hi).ok());
                self.admitted.allocate_provenance(
                    span.map_or(OriginRecord::UnknownBootstrap, OriginRecord::SourceSpan),
                )?
            }
            crate::token::OriginEncoding::Unknown
            | crate::token::OriginEncoding::NoExpandFallback => self
                .admitted
                .allocate_provenance(OriginRecord::UnknownBootstrap)?,
        };
        let record = self.admitted.provenance(coordinate);
        let (resolved, generated_origin) = self.detach_origin_source(record, request.demand);
        Ok(
            crate::provenance_resolver::ProvenanceResolver::<G>::admitted(request.demand)
                .detach_admitted_origin(request.message, record, resolved, generated_origin),
        )
    }

    /// Detaches the immutable source recipe needed by a committed output
    /// artifact. No source-map coordinate or backing owner escapes admission.
    #[must_use]
    pub fn detach_artifact_source_recipe(
        &self,
        origin: crate::token::OriginId,
    ) -> Option<crate::world::ArtifactSourceRecipe> {
        let record = match origin.decode() {
            crate::token::OriginEncoding::Arena(index) => self
                .admitted
                .provenance_coordinate_at(index)
                .map(|coordinate| self.admitted.provenance(coordinate))?,
            crate::token::OriginEncoding::DirectSource(position) => {
                let region = self.sources.region_for_backed_position(position)?;
                let end = position.raw().checked_add(1)?.min(region.anchor().raw());
                OriginRecord::SourceSpan(
                    self.sources
                        .span(
                            position,
                            crate::source_map::SourcePos::from_raw_for_store(end),
                        )
                        .ok()?,
                )
            }
            crate::token::OriginEncoding::Unknown
            | crate::token::OriginEncoding::NoExpandFallback => return None,
        };
        let (registration, start, end) = match record {
            OriginRecord::Source(source) => {
                let registration = self.sources.registration_for_source(source.source())?;
                let start = source.byte_offset();
                (registration, start, start.saturating_add(1))
            }
            OriginRecord::SourceSpan(span) => {
                let registration = self.sources.registration_for_span(span)?;
                let region = registration.region();
                (
                    registration,
                    span.lo().raw().checked_sub(region.start.raw())?,
                    span.hi().raw().checked_sub(region.start.raw())?,
                )
            }
            _ => return None,
        };
        let (content, logical_path) = match registration.descriptor() {
            crate::source_map::SourceDescriptor::World { input_record, .. } => {
                let input = self.world.input_record(*input_record)?;
                (input.hash(), input.path().to_string_lossy().into_owned())
            }
            crate::source_map::SourceDescriptor::Generated(source) => (
                source.hash(),
                source.logical_path().unwrap_or("<generated>").to_owned(),
            ),
        };
        Some(crate::world::ArtifactSourceRecipe {
            content,
            logical_path,
            start,
            end,
        })
    }

    fn detach_origin_source(
        &self,
        record: OriginRecord,
        demand: crate::ColdProvenanceDemand,
    ) -> (
        Option<crate::ResolvedSourceLocation>,
        Option<crate::DetachedGeneratedSourceSpan>,
    ) {
        let (registration, start, end) = match record {
            OriginRecord::SourceSpan(span) => {
                let Some(registration) = self.sources.registration_for_span(span) else {
                    return (None, None);
                };
                let region = registration.region();
                let Some(start) = span.lo().raw().checked_sub(region.start.raw()) else {
                    return (None, None);
                };
                let Some(end) = span.hi().raw().checked_sub(region.start.raw()) else {
                    return (None, None);
                };
                (registration, start, end)
            }
            OriginRecord::Source(source) => {
                let Some(registration) = self.sources.registration_for_source(source.source())
                else {
                    return (None, None);
                };
                let start = source.byte_offset();
                (registration, start, start.saturating_add(1))
            }
            _ => return (None, None),
        };
        let (logical_path, bytes, generated) = match registration.descriptor() {
            crate::source_map::SourceDescriptor::World { input_record, .. } => {
                let Some(record) = self.world.input_record(*input_record) else {
                    return (None, None);
                };
                let Some(bytes) = self.world.input_content(record.hash()) else {
                    return (None, None);
                };
                (
                    record.path().to_string_lossy().into_owned(),
                    bytes.to_vec(),
                    false,
                )
            }
            crate::source_map::SourceDescriptor::Generated(source) => (
                source.logical_path().unwrap_or("<generated>").to_owned(),
                source.bytes().to_vec(),
                true,
            ),
        };
        let span = crate::DetachedGeneratedSourceSpan {
            logical_path,
            bytes,
            start,
            end,
        };
        let resolved = crate::provenance_resolver::ProvenanceResolver::<G>::admitted(demand)
            .resolve_generated(&span);
        (resolved, generated.then_some(span))
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

    pub fn assign_glue_parameter(
        &mut self,
        parameter: crate::env::banks::GlueParam,
        value: Option<GlueId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted
            .state()
            .assign_glue_parameter(parameter, value, scope)
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

    pub fn assign_page_box(
        &mut self,
        index: u16,
        value: Option<PageListId>,
        scope: AssignmentScope,
    ) -> Result<(), crate::NodePromotionError> {
        let durable = value
            .map(|root| {
                self.admitted
                    .promote_page_nodes(self.page_nodes, &[root])
                    .map(|roots| roots[0])
            })
            .transpose()?;
        self.admitted
            .state()
            .assign_box_register(index, durable, scope)
            .map_err(|_| crate::NodePromotionError::Values(crate::PromotionError::AllocationFailed))
    }

    pub fn assign_page_box_global(
        &mut self,
        index: u16,
        value: PageListId,
    ) -> Result<(), crate::NodePromotionError> {
        self.assign_page_box(index, Some(value), AssignmentScope::Global)
    }

    pub fn replace_page_box(
        &mut self,
        index: u16,
        value: PageListId,
    ) -> Result<(), crate::NodePromotionError> {
        let durable = self
            .admitted
            .promote_page_nodes(self.page_nodes, &[value])?[0];
        self.admitted
            .state()
            .replace_box_register(index, Some(durable))
            .map_err(|_| crate::NodePromotionError::Values(crate::PromotionError::AllocationFailed))
    }

    pub fn copy_box_to_page(&mut self, index: u16) -> Option<PageListId> {
        let root = self.box_register(index).ok().flatten()?;
        Some(
            self.admitted
                .copy_nodes_into_page(&[root], self.page_nodes)
                .expect("durable box closure belongs to the admitted generation")[0],
        )
    }

    pub fn take_box_to_page(&mut self, index: u16) -> Option<PageListId> {
        let copied = self.copy_box_to_page(index);
        if copied.is_some() {
            self.admitted
                .state()
                .replace_box_register(index, None)
                .expect("box register index is admitted");
        }
        copied
    }

    pub fn clear_box_preserving_level(&mut self, index: u16) {
        self.admitted
            .state()
            .replace_box_register(index, None)
            .expect("box register index is admitted");
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

    pub fn assign_code(
        &mut self,
        kind: CodeTableKind,
        scalar: char,
        value: i64,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted
            .state()
            .assign_code(kind, scalar, value, scope)
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

    pub fn assign_dimen_param(
        &mut self,
        parameter: crate::env::banks::DimenParam,
        value: Scaled,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.admitted
            .state()
            .assign_dimension_parameter(parameter, value, scope)
    }

    pub fn assign_current_font(
        &mut self,
        value: crate::ids::FontId,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.validate_font_root(value)?;
        self.admitted.state().assign_current_font(value, scope)
    }

    pub fn assign_math_family_font(
        &mut self,
        size: crate::math::MathFontSize,
        family: u8,
        value: crate::ids::FontId,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.validate_font_root(value)?;
        let index = u8::try_from(size.index())
            .expect("math font size is bounded")
            .saturating_mul(16)
            .saturating_add(family);
        self.admitted
            .state()
            .assign_math_family_font(index, value, scope)
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

    /// Borrows the exact open-group frames for command-layer coordination.
    ///
    /// The slice is valid only for this admitted command episode. Persistent
    /// command state copies the compact frames it needs and never retains a
    /// reference into the mutable state bank.
    #[must_use]
    pub fn group_frames(&self) -> &[crate::GroupFrame] {
        self.admitted.state_ref().group_frames()
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
    pub fn execution_group_depth(&self) -> usize {
        self.admitted.state_ref().group_frames().len()
    }

    #[must_use]
    pub fn innermost_group_kind(&self) -> Option<crate::GroupKind> {
        self.admitted
            .state_ref()
            .group_frames()
            .last()
            .map(|frame| frame.kind())
    }

    pub fn begin_group(
        &mut self,
        kind: crate::GroupKind,
        entered_line: u32,
    ) -> Result<crate::GroupFrame, StateError> {
        self.admitted.state().begin_group(kind, entered_line)
    }

    pub fn end_group(
        &mut self,
        kind: crate::GroupKind,
    ) -> Result<crate::GroupRestorationReceipt<G>, StateError> {
        let receipt = self.admitted.state().end_group(kind)?;
        // Closing a save level replays an ordered environment-journal suffix.
        // That timeline mutation cannot be validated from the individual live
        // post-images alone, so an in-flight memo episode must fail closed.
        self.dependencies
            .poison(TrackedRegionBarrier::EnvironmentTimelineChange);
        Ok(receipt)
    }

    /// Opens §245's diagnostic channel with the print controls captured after
    /// one ordered §283 restoration decision.
    ///
    /// The restoration receipt itself owns no printer or World borrow. The
    /// executor consumes it synchronously and opens this short-lived channel
    /// only while publishing the matching detached entry.
    pub fn begin_group_restoration_diagnostic<'effects>(
        &self,
        effects: &'effects mut crate::diagnostic::DiagnosticEffects,
        trace: crate::GroupRestorationTraceState,
    ) -> crate::diagnostic::Diagnostic<'effects> {
        crate::diagnostic::Diagnostic::from_parts(
            effects,
            *self.interaction_mode,
            self.error_context_widths.max_print_line(),
            trace.tracing_online(),
            trace.newline_char(),
            trace.escape_char(),
        )
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

    /// Renders TeX82 §252's compact box-register assignment value.
    #[must_use]
    pub fn box_assignment_trace_text(&self, value: Option<PageListId>) -> String {
        let Some(value) = value else {
            return "void".to_owned();
        };
        let list = self
            .page_node_list(value)
            .expect("box assignment root belongs to the admitted page arena");
        let (kind, node) = match (list.len(), list.nodes().first()) {
            (1, Some(crate::node::Node::HList(node))) => ("hbox", node),
            (1, Some(crate::node::Node::VList(node))) => ("vbox", node),
            _ => return "void".to_owned(),
        };
        let abbreviated_children = if self
            .page_node_list(node.children)
            .is_ok_and(|children| !children.is_empty())
        {
            " []"
        } else {
            ""
        };
        let glue_setting = box_glue_setting_text(node);
        let shift = if node.shift.raw() != 0 {
            format!(", shifted {}", crate::scaled::print_scaled(node.shift))
        } else {
            String::new()
        };
        let subtype = match node.box_lr {
            crate::node::BoxLr::Normal => "",
            crate::node::BoxLr::Reversed => ", reversed",
            crate::node::BoxLr::DList => ", display",
        };
        format!(
            "\\{kind}({}+{})x{}{glue_setting}{shift}{subtype}{abbreviated_children}",
            crate::scaled::print_scaled(node.height),
            crate::scaled::print_scaled(node.depth),
            crate::scaled::print_scaled(node.width)
        )
    }

    /// Renders one command meaning through the admitted state without
    /// reopening the aggregate generation barrier.
    #[must_use]
    pub fn bounded_meaning_text(&self, token: crate::token::Token, breadth: usize) -> String {
        crate::token_show::bounded_meaning_text_with(self, token, breadth)
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

    /// Detaches the complete artifact-facing metadata for one live font.
    #[must_use]
    pub fn font_artifact_recipe(&self, id: crate::ids::FontId) -> crate::FontArtifactRecipe {
        self.fonts.artifact_recipe(id)
    }

    /// Detaches the complete immutable font recipe prefix for one admitted
    /// episode. Callers retain no font ID or generation handle.
    pub fn font_artifact_recipes(&self) -> Vec<crate::FontArtifactRecipe> {
        (0..self.fonts.len())
            .map(|slot| {
                let id = self
                    .fonts
                    .id_at(u32::try_from(slot).expect("font store is bounded by u32"))
                    .expect("immutable font prefix is dense");
                self.fonts.artifact_recipe(id)
            })
            .collect()
    }

    #[must_use]
    pub fn font_construction(&self, id: crate::ids::FontId) -> tex_fonts::FontConstruction {
        self.fonts.get(id).construction().clone()
    }

    #[must_use]
    pub fn font_supports_math(&self, id: crate::ids::FontId) -> bool {
        self.fonts.get(id).supports_math()
    }

    #[must_use]
    pub fn font_parameter_count(&self, id: crate::ids::FontId) -> usize {
        self.admitted
            .state_ref()
            .font_parameter_count(id)
            .expect("live font has runtime parameter state") as usize
    }

    /// Resolves a generated font's semantic source inside this admitted
    /// episode.  The returned runtime id never enters the detached recipe.
    #[must_use]
    pub fn font_id_for_source_identity(
        &self,
        identity: tex_fonts::FontSourceIdentity,
    ) -> Option<crate::ids::FontId> {
        self.fonts.by_source_identity(identity)
    }

    #[must_use]
    pub fn font_false_boundary_char(&self, id: crate::ids::FontId) -> Option<u8> {
        self.fonts.get(id).metrics().false_boundary_char()
    }

    /// Interns one already-validated in-memory font for an admitted executor
    /// operation. Runtime resource loading should map capacity failure before
    /// entering the mutation episode.
    pub fn intern_font(&mut self, font: tex_fonts::LoadedFont) -> crate::ids::FontId {
        let allocates = self.fonts.would_allocate(&font);
        let default_hyphen_char = self.int_param(IntParam::DEFAULT_HYPHEN_CHAR);
        let default_skew_char = self.int_param(IntParam::DEFAULT_SKEW_CHAR);
        let prepared = allocates.then(|| {
            self.admitted
                .state()
                .prepare_font_runtime(font.parameters(), default_hyphen_char, default_skew_char)
                .expect("validated font runtime state exceeds memory")
        });
        let id = self
            .fonts
            .intern(font)
            .expect("validated font exceeds the live font store capacity");
        if let Some(prepared) = prepared {
            self.admitted
                .state()
                .install_font_runtime(id, prepared)
                .expect("fresh font runtime row follows the font store");
        }
        id
    }

    pub fn set_font_identifier_symbol(
        &mut self,
        font: crate::ids::FontId,
        identifier: impl Into<FontIdentifier>,
    ) {
        let symbol = match identifier.into() {
            FontIdentifier::Symbol(symbol) => self
                .interner
                .qualify_local(symbol)
                .expect("font identifier belongs to the admitted session"),
            FontIdentifier::Qualified(symbol) => symbol,
        };
        let kind = self
            .interner
            .kind_id(symbol)
            .expect("font identifier belongs to the admitted session");
        let name = self
            .interner
            .resolve_id(symbol)
            .expect("font identifier belongs to the admitted session")
            .to_owned();
        let complete = crate::font::complete_font_hash_fragment(
            *self.fonts.hash_fragment(font),
            Some((kind, &name)),
        );
        self.fonts.set_identifier(font, symbol, complete);
    }

    pub fn try_intern_font_with_identifier(
        &mut self,
        font: tex_fonts::LoadedFont,
        identifier: impl Into<FontIdentifier>,
    ) -> Result<crate::ids::FontId, crate::font::FontStoreCapacityError> {
        let allocates = self.fonts.would_allocate(&font);
        let default_hyphen_char = self.int_param(IntParam::DEFAULT_HYPHEN_CHAR);
        let default_skew_char = self.int_param(IntParam::DEFAULT_SKEW_CHAR);
        let prepared = allocates
            .then(|| {
                self.admitted.state().prepare_font_runtime(
                    font.parameters(),
                    default_hyphen_char,
                    default_skew_char,
                )
            })
            .transpose()
            .map_err(|_| crate::font::FontStoreCapacityError)?;
        let id = self.fonts.intern(font)?;
        if let Some(prepared) = prepared {
            self.admitted
                .state()
                .install_font_runtime(id, prepared)
                .map_err(|_| crate::font::FontStoreCapacityError)?;
        }
        self.set_font_identifier_symbol(id, identifier);
        Ok(id)
    }

    pub fn try_copy_font_with_identifier(
        &mut self,
        source: crate::ids::FontId,
        identifier: impl Into<FontIdentifier>,
    ) -> Result<crate::ids::FontId, crate::font::FontStoreCapacityError> {
        let parameters = (1..=self.font_parameter_count(source))
            .map(|number| self.font_dimen(source, number as u32))
            .collect();
        let font = self.fonts.get(source).copied(parameters);
        let id = self.try_intern_derived_font(font, source, true, false, false)?;
        self.set_font_identifier_symbol(id, identifier);
        Ok(id)
    }

    pub fn try_letterspace_font_with_identifier(
        &mut self,
        source: crate::ids::FontId,
        identifier: impl Into<FontIdentifier>,
        amount: i16,
        no_ligatures: bool,
    ) -> Result<crate::ids::FontId, crate::font::FontStoreCapacityError> {
        let current_quad = self.font_dimen(source, 6);
        let font = self
            .fonts
            .get(source)
            .letterspaced(current_quad, amount, no_ligatures)
            .expect("bounded live TeX font widths support letterspacing");
        let id = self.try_intern_derived_font(font, source, false, false, no_ligatures)?;
        self.set_font_identifier_symbol(id, identifier);
        Ok(id)
    }

    pub fn configure_font_expansion(
        &mut self,
        font: crate::ids::FontId,
        expansion: crate::font::FontExpansion,
    ) -> Result<bool, crate::font::FontExpansionConfigError> {
        self.fonts.set_expansion(font, expansion)
    }

    pub fn try_expanded_font(
        &mut self,
        source: crate::ids::FontId,
        ratio: i16,
    ) -> Result<crate::ids::FontId, crate::font::FontStoreCapacityError> {
        if ratio == 0 {
            return Ok(source);
        }
        let generated = self.fonts.get(source).expanded(ratio);
        if let Some(existing) = self.fonts.by_source_identity(generated.source_identity()) {
            return Ok(existing);
        }
        self.try_intern_derived_font(generated, source, true, true, false)
    }

    fn try_intern_derived_font(
        &mut self,
        font: tex_fonts::LoadedFont,
        source: crate::ids::FontId,
        preserve_character_settings: bool,
        preserve_pdf_settings: bool,
        disable_ligatures: bool,
    ) -> Result<crate::ids::FontId, crate::font::FontStoreCapacityError> {
        let allocates = self.fonts.would_allocate(&font);
        let default_hyphen_char = self.int_param(IntParam::DEFAULT_HYPHEN_CHAR);
        let default_skew_char = self.int_param(IntParam::DEFAULT_SKEW_CHAR);
        let prepared = allocates
            .then(|| {
                self.admitted.state().prepare_derived_font_runtime(
                    crate::env::DerivedFontRuntimeRequest {
                        source,
                        parameters: font.parameters(),
                        preserve_character_settings,
                        preserve_pdf_settings,
                        disable_ligatures,
                        default_hyphen_char,
                        default_skew_char,
                    },
                )
            })
            .transpose()
            .map_err(|_| crate::font::FontStoreCapacityError)?;
        let id = self.fonts.intern(font)?;
        if let Some(prepared) = prepared {
            self.admitted
                .state()
                .install_font_runtime(id, prepared)
                .map_err(|_| crate::font::FontStoreCapacityError)?;
        }
        Ok(id)
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
    pub fn font_is_left_to_right_shaping(&self, id: crate::ids::FontId) -> bool {
        self.fonts
            .get(id)
            .opentype()
            .is_some_and(|font| font.direction == tex_fonts::WritingDirection::LeftToRight)
    }

    #[must_use]
    pub fn font_mapped_text(&self, id: crate::ids::FontId, character: char) -> Option<&str> {
        self.fonts.get(id).mapped_text(character)
    }

    #[must_use]
    pub fn shape_font_run(
        &self,
        id: crate::ids::FontId,
        request: tex_fonts::ShapingRequest<'_>,
    ) -> Option<tex_fonts::ShapedRun> {
        self.fonts.get(id).shape_run(request)
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
            .unwrap_or_else(|| self.font_parameter_count(id))
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
        (self.pdf_font_code(crate::PdfFontCode::Tag, id, code) & 4 != 0)
            .then(|| self.fonts.get(id).metrics().extensible_recipe(code))
            .flatten()
    }

    #[must_use]
    pub fn font_lig_kern_command(
        &self,
        id: crate::ids::FontId,
        left: crate::font::LigKernChar,
        right: crate::font::LigKernChar,
    ) -> Option<crate::font::LigKernCommand> {
        if let crate::font::LigKernChar::Char(code) = left
            && self.pdf_font_code(crate::PdfFontCode::Tag, id, code) & 1 == 0
        {
            return None;
        }
        let command = self.fonts.get(id).metrics().lig_kern_command(left, right);
        if self.pdf_font_ligatures_disabled(id) {
            return command
                .filter(|command| matches!(command, crate::font::LigKernCommand::Kern(_)));
        }
        command
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
        self.admitted
            .state_ref()
            .font_dimen(id, number)
            .unwrap_or_else(|_| Scaled::from_raw(0))
    }

    #[must_use]
    pub fn font_dimen_readable(&self, id: crate::ids::FontId, number: u32) -> bool {
        number != 0 && (number as usize) <= self.font_parameter_count(id)
    }

    #[must_use]
    pub fn font_dimen_writable(&self, id: crate::ids::FontId, number: u32) -> bool {
        self.font_dimen_readable(id, number)
            || (number != 0
                && usize::try_from(id.raw())
                    .ok()
                    .and_then(|raw| raw.checked_add(1))
                    == Some(self.fonts.len())
                && usize::try_from(number).ok().is_some_and(|number| {
                    let current = self.font_parameter_count(id);
                    let growth = number.saturating_sub(current);
                    growth
                        <= self
                            .font_info_capacity
                            .saturating_sub(self.admitted.state_ref().font_parameter_words())
                }))
    }

    #[must_use]
    pub fn font_hyphen_char(&self, id: crate::ids::FontId) -> i32 {
        self.admitted
            .state_ref()
            .font_hyphen_char(id)
            .expect("live font has runtime hyphen state")
    }

    #[must_use]
    pub fn font_skew_char(&self, id: crate::ids::FontId) -> i32 {
        self.admitted
            .state_ref()
            .font_skew_char(id)
            .expect("live font has runtime skew state")
    }

    pub fn set_font_dimen(
        &mut self,
        id: crate::ids::FontId,
        number: u32,
        value: Scaled,
    ) -> Result<(), usize> {
        if !self.font_dimen_writable(id, number) {
            return Err(self.font_info_capacity);
        }
        self.admitted
            .state()
            .assign_font_dimen(id, number, value, AssignmentScope::Global)
            .map_err(|_| self.font_info_capacity)
    }

    pub fn set_font_hyphen_char(&mut self, id: crate::ids::FontId, value: i32) {
        self.admitted
            .state()
            .assign_font_hyphen_char(id, value, AssignmentScope::Global)
            .expect("live font has runtime hyphen state");
    }

    pub fn set_font_skew_char(&mut self, id: crate::ids::FontId, value: i32) {
        self.admitted
            .state()
            .assign_font_skew_char(id, value, AssignmentScope::Global)
            .expect("live font has runtime skew state");
    }

    #[must_use]
    pub fn pdf_font_code(
        &self,
        table: crate::PdfFontCode,
        font: crate::ids::FontId,
        code: u8,
    ) -> i32 {
        self.admitted
            .state_ref()
            .pdf_font_code(font, table, code)
            .unwrap_or_else(|_| self.default_pdf_font_code(table, font, code))
    }

    pub fn set_pdf_font_code(
        &mut self,
        table: crate::PdfFontCode,
        font: crate::ids::FontId,
        code: u8,
        value: i32,
    ) {
        let defaults =
            core::array::from_fn(|code| self.default_pdf_font_code(table, font, code as u8));
        self.admitted
            .state()
            .prepare_pdf_font_code_table(font, table, defaults)
            .expect("live font admits its PDF code table");
        let value = match table {
            crate::PdfFontCode::Lp
            | crate::PdfFontCode::Rp
            | crate::PdfFontCode::Knbs
            | crate::PdfFontCode::Stbs
            | crate::PdfFontCode::Shbs
            | crate::PdfFontCode::Knbc
            | crate::PdfFontCode::Knac => value.clamp(-1000, 1000),
            crate::PdfFontCode::Ef => value.clamp(0, 1000),
            crate::PdfFontCode::Tag => {
                let current = self.pdf_font_code(table, font, code);
                if value >= 0 {
                    current
                } else {
                    current & !(-value).min(7)
                }
            }
        };
        self.admitted
            .state()
            .assign_pdf_font_code(font, table, code, value, AssignmentScope::Global)
            .expect("prepared PDF font code cell is admitted");
    }

    pub fn disable_pdf_font_ligatures(&mut self, font: crate::ids::FontId) {
        self.admitted
            .state()
            .assign_pdf_font_ligatures_disabled(font, true, AssignmentScope::Global)
            .expect("live font has runtime ligature state");
    }

    #[must_use]
    pub fn pdf_font_ligatures_disabled(&self, font: crate::ids::FontId) -> bool {
        self.admitted
            .state_ref()
            .pdf_font_ligatures_disabled(font)
            .expect("live font has runtime ligature state")
    }

    fn default_pdf_font_code(
        &self,
        table: crate::PdfFontCode,
        font: crate::ids::FontId,
        code: u8,
    ) -> i32 {
        match table {
            crate::PdfFontCode::Ef => 1000,
            crate::PdfFontCode::Tag => self
                .fonts
                .get(font)
                .character_metrics(char::from(code))
                .map_or(0, |metrics| match metrics.tag {
                    crate::font::CharTag::None => 0,
                    crate::font::CharTag::LigKern { .. } => 1,
                    crate::font::CharTag::NextLarger(_) => 2,
                    crate::font::CharTag::Extensible(_) => 4,
                }),
            _ => 0,
        }
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

    /// Validates and freezes TeX82 §288's job-level magnification.
    pub fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        let attempted = self.int_param(IntParam::MAG);
        let (effective, diagnostic) = if let Some(retained) = *self.prepared_mag
            && attempted != retained
        {
            (
                retained,
                Some(PrepareMagDiagnostic::IncompatibleMagnification {
                    attempted,
                    retained,
                }),
            )
        } else if !(1..=32_768).contains(&attempted) {
            (
                1_000,
                Some(PrepareMagDiagnostic::IllegalMagnification { attempted }),
            )
        } else {
            (attempted, None)
        };
        if effective != attempted {
            self.assign_int_param(IntParam::MAG, effective, AssignmentScope::Global)
                .expect("the magnification parameter is admitted");
        }
        *self.prepared_mag = Some(effective);
        (effective, diagnostic)
    }

    #[must_use]
    pub fn internal_integer(&self, integer: crate::meaning::InternalInteger) -> Option<i32> {
        use crate::meaning::InternalInteger;
        Some(match integer {
            // TeX82 §§422--424 read `badness_code` from `last_badness`;
            // §§644/660 and §§668/674 update that same cell while
            // packing horizontal and vertical lists respectively.
            InternalInteger::Badness => self.int_param(IntParam::LAST_BADNESS),
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
    pub fn pdf_font_configuration(&self) -> crate::PdfFontConfiguration {
        crate::PdfFontConfiguration {
            adjust_spacing: self.int_param(IntParam::PDF_ADJUST_SPACING),
            protrude_chars: self.int_param(IntParam::PDF_PROTRUDE_CHARS),
            tracing_fonts: self.int_param(IntParam::PDF_TRACING_FONTS),
            adjust_interword_glue: self.int_param(IntParam::PDF_ADJUST_INTERWORD_GLUE),
            prepend_kern: self.int_param(IntParam::PDF_PREPEND_KERN),
            append_kern: self.int_param(IntParam::PDF_APPEND_KERN),
            generate_to_unicode: self.int_param(IntParam::PDF_GEN_TO_UNICODE),
            pk_resolution: self.int_param(IntParam::PDF_PK_RESOLUTION),
            omit_charset: self.int_param(IntParam::PDF_OMIT_CHARSET),
        }
    }

    pub fn set_last_badness(&mut self, value: i32) {
        self.assign_int_param(IntParam::LAST_BADNESS, value, AssignmentScope::Global)
            .expect("last-badness parameter is admitted");
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

    /// Advances and records an executor-owned semantic projection in the
    /// active dependency episode.
    pub fn observe_changed_command_projection(
        &mut self,
        key: DependencyKey,
        value: DependencyValue,
    ) {
        self.dependencies.mark_changed(key);
        self.dependencies.track(key);
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

    pub fn set_interaction_mode(&mut self, mode: InteractionMode) {
        *self.interaction_mode = mode;
    }

    pub fn clear_error_count(&mut self) {
        self.world.error_channel_mut().clear_error_count();
    }

    pub fn take_long_help_seen(&mut self, mark: bool) -> bool {
        self.world.error_channel_mut().take_long_help_seen(mark)
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
            .register_without_line_starts(source, descriptor)
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

    pub fn source_range_origin(
        &mut self,
        source: crate::input::SourceId,
        start: u64,
        end: u64,
    ) -> crate::token::OriginId {
        let Some(span) = self
            .sources
            .registered_source(source)
            .and_then(|registered| registered.span(start, end).ok())
        else {
            return crate::token::OriginId::UNKNOWN;
        };
        let Ok(coordinate) = self
            .admitted
            .allocate_provenance(OriginRecord::SourceSpan(span))
        else {
            // Provenance is diagnostic-only: an exhausted sidecar must not
            // change TeX semantics or make an otherwise valid token fail.
            return crate::token::OriginId::UNKNOWN;
        };
        crate::token::OriginId::arena(coordinate.format_index())
            .unwrap_or(crate::token::OriginId::UNKNOWN)
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

    pub fn close_hyphenation_patterns(&mut self) {
        self.hyphenation.close_patterns();
    }

    pub fn add_hyphenation_pattern_for_language(
        &mut self,
        language: u8,
        pattern: PatternSpec,
    ) -> Result<bool, crate::hyphenation::HyphenationCapacityError> {
        self.hyphenation.add_pattern_for_language(language, pattern)
    }

    pub fn add_hyphenation_exception_for_language(
        &mut self,
        language: u8,
        exception: ExceptionSpec,
    ) {
        self.hyphenation
            .add_exception_for_language(language, exception);
    }

    pub fn save_hyphenation_codes(
        &mut self,
        language: u8,
        codes: impl IntoIterator<Item = (char, char)>,
    ) {
        self.hyphenation.save_hyphen_codes(language, codes);
    }

    #[must_use]
    pub fn hyphen_positions_for_language(
        &self,
        language: u8,
        word: &str,
        left_min: usize,
        right_min: usize,
    ) -> Vec<usize> {
        self.hyphenation
            .hyphen_positions_for_language(language, word, left_min, right_min)
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
    pub fn pdf_external_image_record(
        &self,
        id: crate::PdfExternalImageId,
    ) -> Option<crate::PdfExternalImageRecord> {
        self.pdf.external_image_record(id)
    }

    pub fn allocate_pdf_external_image(
        &mut self,
        source: crate::PdfExternalImageSource,
        dimensions: crate::PdfExternalImageDimensions,
        color_space_object: i32,
    ) -> Result<crate::PdfExternalImageRecord, crate::PdfObjectCapacityError> {
        self.pdf
            .allocate_external_image(source, dimensions, color_space_object)
    }

    pub fn reserve_pdf_annotation(
        &mut self,
    ) -> Result<crate::PdfAnnotationRecord<G>, crate::PdfObjectCapacityError> {
        self.pdf.reserve_annotation()
    }

    pub fn initialize_pdf_annotation(
        &mut self,
        object: u32,
        data: crate::PdfAnnotationData<G>,
    ) -> Result<crate::PdfAnnotationRecord<G>, crate::PdfAnnotationInitializeError> {
        let semantic_id = self.token_semantic_id(data.entries);
        self.pdf.initialize_annotation(object, data, semantic_id)
    }

    pub fn create_pdf_annotation(
        &mut self,
        data: crate::PdfAnnotationData<G>,
    ) -> Result<crate::PdfAnnotationRecord<G>, crate::PdfObjectCapacityError> {
        let object = self.pdf.reserve_annotation()?.object();
        self.initialize_pdf_annotation(object, data)
            .map_err(|_| crate::PdfObjectCapacityError)
    }

    pub fn create_pdf_link(
        &mut self,
        dimensions: crate::PdfAnnotationDimensions,
        attributes: TokenListId<G>,
        action: crate::PdfActionSpec<G>,
        nesting_depth: usize,
    ) -> Result<crate::PdfLinkRecord<G>, crate::PdfObjectCapacityError> {
        let attributes_semantic_id = self.token_semantic_id(attributes);
        let action_semantic_id = action.fingerprint(|tokens| self.token_semantic_id(tokens));
        self.pdf.create_link(
            dimensions,
            attributes,
            action,
            attributes_semantic_id,
            action_semantic_id,
            u32::try_from(nesting_depth).unwrap_or(u32::MAX),
        )
    }

    pub fn end_pdf_link(&mut self) -> Option<crate::PdfOpenLink<G>> {
        self.pdf.end_link()
    }

    pub fn create_pdf_outline(
        &mut self,
        attributes: TokenListId<G>,
        action: crate::PdfActionSpec<G>,
        count: i32,
        title: TokenListId<G>,
    ) -> Result<crate::PdfOutlineRecord<G>, crate::PdfObjectCapacityError> {
        let semantic_ids = [
            self.token_semantic_id(attributes),
            action.fingerprint(|tokens| self.token_semantic_id(tokens)),
            self.token_semantic_id(title),
        ];
        self.pdf
            .create_outline(attributes, action, count, title, semantic_ids)
    }

    #[must_use]
    pub fn pdf_destination(
        &self,
        identity: &crate::PdfDestinationIdentity,
        structure: bool,
    ) -> Option<crate::PdfDestinationRecord> {
        self.pdf.destination(identity, structure).cloned()
    }

    pub fn reserve_pdf_destination(
        &mut self,
        identity: crate::PdfDestinationIdentity,
        structure: bool,
    ) -> Result<crate::PdfDestinationRecord, crate::PdfObjectCapacityError> {
        self.pdf.reserve_destination(identity, structure)
    }

    pub fn reserve_pdf_thread(
        &mut self,
        identity: crate::PdfDestinationIdentity,
    ) -> Result<crate::PdfThreadRecord, crate::PdfObjectCapacityError> {
        self.pdf.reserve_thread(identity)
    }

    #[must_use]
    pub fn pdf_raw_object(&self, object: u32) -> Option<crate::PdfRawObjectRecord<G>> {
        self.pdf
            .raw_object(crate::PdfRawObjectId::from_allocated(object))
    }

    pub fn reserve_pdf_raw_object(
        &mut self,
    ) -> Result<crate::PdfRawObjectId, crate::PdfObjectCapacityError> {
        self.pdf.reserve_raw_object()
    }

    pub fn initialize_pdf_raw_object(
        &mut self,
        id: crate::PdfRawObjectId,
        stream: bool,
        stream_attr: Option<TokenListId<G>>,
        file: bool,
        data: TokenListId<G>,
        immediate: bool,
    ) -> Result<(), crate::PdfRawObjectInitializeError> {
        let stream_attr = stream_attr.map(|tokens| self.pdf_token_parameter(tokens));
        let data = self.pdf_token_parameter(data);
        self.pdf.initialize_raw_object(
            id,
            crate::PdfRawObjectData::new(stream, stream_attr, file, data),
            immediate,
        )
    }

    pub fn set_pdf_space_font_name(&mut self, name: Vec<u8>) {
        self.pdf.set_space_font_name(name);
    }

    pub fn set_pdf_return_value(&mut self, value: i32) {
        self.pdf.set_return_value(value);
    }

    pub fn has_pdf_color_stack(&mut self, id: u32) -> bool {
        self.pdf.has_color_stack(id)
    }

    pub fn push_pdf_font_map(&mut self, operation: crate::PdfFontMapOperation) {
        self.pdf.push_font_map(operation);
    }

    #[must_use]
    pub fn pdf_font_map_duplicate_names(&self) -> Vec<Vec<u8>> {
        self.pdf.font_map_duplicate_names()
    }

    pub fn set_pdf_font_attribute(&mut self, font: crate::ids::FontId, bytes: Vec<u8>) {
        self.validate_font_root(font)
            .expect("PDF font attribute retains a live admitted font");
        self.pdf.set_font_attribute(font, bytes);
    }

    pub fn include_pdf_font_chars(&mut self, font: crate::ids::FontId, chars: Vec<u8>) {
        self.validate_font_root(font)
            .expect("PDF character inclusion retains a live admitted font");
        self.pdf.include_font_chars(font, chars);
    }

    pub fn disable_pdf_builtin_to_unicode(&mut self, font: crate::ids::FontId) {
        self.validate_font_root(font)
            .expect("PDF font configuration retains a live admitted font");
        self.pdf.disable_builtin_to_unicode(font);
    }

    pub fn set_pdf_glyph_to_unicode(&mut self, mapping: crate::PdfGlyphToUnicode) {
        self.pdf.set_glyph_to_unicode(mapping);
    }

    #[must_use]
    pub fn pdf_form_resource(&self, object: u32) -> Option<u32> {
        self.pdf.form(object).map(|form| form.resource())
    }

    pub fn ensure_pdf_font_resource(
        &mut self,
        font: crate::ids::FontId,
    ) -> Result<crate::PdfFontResourceRecord, crate::PdfObjectCapacityError> {
        self.validate_font_root(font)
            .map_err(|_| crate::PdfObjectCapacityError)?;
        let recipe = self.fonts.artifact_recipe(font);
        let identity = tex_fonts::PdfFontResourceIdentity::new(
            recipe.tfm_content_hash,
            recipe.opentype.map(|opentype| opentype.program_identity),
        );
        self.pdf
            .ensure_font_resource(font, recipe.semantic_identity, identity)
    }

    pub fn reference_pdf_raw_object(
        &mut self,
        raw: u32,
    ) -> Result<(), crate::PdfRawObjectInitializeError> {
        self.pdf
            .reference_raw_object(crate::PdfRawObjectId::from_allocated(raw))
    }

    #[must_use]
    pub fn pdf_form(&self, object: u32) -> Option<crate::PdfFormRecord<G>> {
        self.pdf.form(object)
    }

    pub fn reserve_pdf_form(&mut self) -> Result<(u32, u32), crate::PdfObjectCapacityError> {
        self.pdf.reserve_form()
    }

    pub fn initialize_pdf_form(
        &mut self,
        identity: (u32, u32),
        box_list: PageListId,
        dimensions: (Scaled, Scaled, Scaled),
        attr: Option<TokenListId<G>>,
        resources: Option<TokenListId<G>>,
        immediate: bool,
    ) -> Result<crate::PdfFormRecord<G>, crate::PdfObjectCapacityError> {
        let semantic_id = page_list_semantic_id(
            self.page_nodes,
            self.fonts,
            self.admitted.state_ref(),
            box_list,
        );
        let box_list = self
            .admitted
            .promote_page_nodes(self.page_nodes, &[box_list])
            .map_err(|_| crate::PdfObjectCapacityError)?[0];
        let attr = attr.map(|tokens| self.pdf_token_parameter(tokens));
        let resources = resources.map(|tokens| self.pdf_token_parameter(tokens));
        self.pdf.initialize_form(
            identity,
            box_list,
            semantic_id,
            dimensions,
            (attr, resources),
            immediate,
        )
    }

    pub fn append_pdf_document_fragment(
        &mut self,
        kind: crate::PdfDocumentFragmentKind,
        tokens: TokenListId<G>,
    ) {
        let parameter = self.pdf_token_parameter(tokens);
        self.pdf.append_document_fragment(kind, parameter);
    }

    #[must_use]
    pub fn pdf_catalog_open_action(&self) -> Option<crate::PdfActionRecord<G>> {
        self.pdf.catalog_open_action()
    }

    pub fn set_pdf_catalog_open_action_with_targets(
        &mut self,
        action: crate::PdfActionSpec<G>,
        destination: Option<crate::PdfDestinationIdentity>,
        structure: Option<crate::PdfDestinationIdentity>,
        thread: Option<crate::PdfDestinationIdentity>,
    ) -> Result<crate::PdfActionRecord<G>, crate::PdfObjectCapacityError> {
        let fingerprint = action.fingerprint(|tokens| self.token_semantic_id(tokens));
        self.pdf
            .set_catalog_open_action(action, fingerprint, destination, structure, thread)
    }

    fn token_semantic_id(&self, tokens: TokenListId<G>) -> crate::state_hash::StateHashFragment {
        let words = self.token_list(tokens);
        crate::state_hash::StateHashFragment::from_exact_builder(0x7064_665f_746f_6b70, |hasher| {
            hasher.usize(words.len());
            for word in words {
                hasher.u32(word.raw());
            }
        })
    }

    fn pdf_token_parameter(&self, tokens: TokenListId<G>) -> crate::pdf::PdfTokenParameter<G> {
        crate::pdf::PdfTokenParameter {
            tokens,
            semantic_id: self.token_semantic_id(tokens),
        }
    }

    pub fn define_pdf_destination(
        &mut self,
        identity: crate::PdfDestinationIdentity,
        structure_target: Option<u32>,
    ) -> Result<crate::PdfDestinationDefinition, crate::PdfObjectCapacityError> {
        self.pdf.define_destination(identity, structure_target)
    }

    pub fn append_pdf_thread_bead(
        &mut self,
        identity: crate::PdfDestinationIdentity,
    ) -> Result<(crate::PdfThreadRecord, crate::PdfThreadBeadRecord), crate::PdfObjectCapacityError>
    {
        self.pdf.append_thread_bead(identity)
    }

    #[must_use]
    pub fn pdf_page_object(&self, page: u32) -> Option<u32> {
        page.checked_sub(1)
            .and_then(|index| self.pdf.pages().get(index as usize))
            .map(crate::PdfPageRecord::page_object)
    }

    #[must_use]
    pub fn pdf_page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Resolves the complete terminal PDF ledger while this generation is
    /// admitted. The result owns every token spelling, resource payload, and
    /// font identity needed after the generation is retired.
    pub fn detach_pdf_completion(
        &self,
    ) -> Result<crate::DetachedPdfCompletion, crate::PdfCompletionError> {
        let pages_entries = self
            .token_parameter(crate::env::banks::TokParam::PDF_PAGES_ATTR)
            .ok()
            .flatten()
            .map(|tokens| self.pdf_completion_token_bytes(tokens))
            .unwrap_or_default();
        let scalars = crate::pdf::completion::PdfCompletionScalars {
            font_configuration: self.pdf_font_configuration(),
            pages_entries,
            include_info_dictionary: self.int_param(IntParam::PDF_OMIT_INFO_DICT) == 0,
            include_dates: self.int_param(IntParam::PDF_INFO_OMIT_DATE) == 0,
            suppress_ptex_info: self.int_param(IntParam::PDF_SUPPRESS_PTEX_INFO),
            ptex_use_underscore: self.int_param(IntParam::PDF_PTEX_USE_UNDERSCORE) > 0,
            form_omit_procset: self.int_param(IntParam::PDF_OMIT_PROCSET),
            suppress_page_group_warning: self.int_param(IntParam::PDF_SUPPRESS_WARNING_PAGE_GROUP)
                != 0,
            clock: self.world.job_clock(),
        };
        crate::pdf::completion::detach(
            self.pdf,
            scalars,
            |tokens| Ok(self.pdf_completion_token_bytes(tokens)),
            |font| self.fonts.artifact_recipe(font),
            |font, code| self.fonts.get(font).metrics().character(code),
            |font, number| self.font_parameter(font, number),
            |hash| {
                self.world
                    .read_artifact(hash)
                    .map_err(|error| error.to_string())
            },
        )
    }

    fn pdf_completion_token_bytes(&self, tokens: TokenListId<G>) -> Vec<u8> {
        let mut text = String::new();
        for word in self.token_list(tokens) {
            self.append_token_string_text(word.semantic_token(), &mut text);
        }
        text.into_bytes()
    }

    /// Detaches pdfTeX's unresolved navigation diagnostics before the
    /// admission is released for terminal publication.
    #[must_use]
    pub fn detach_pdf_navigation_warnings(&self) -> Vec<crate::PdfNavigationWarning> {
        self.pdf.unresolved_navigation_warnings()
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

    pub fn apply_pdf_color_stack(
        &mut self,
        id: u32,
        target: crate::PdfColorStackTarget,
        action: &crate::PdfColorStackAction,
    ) -> Result<crate::PdfColorStackEmission, crate::PdfColorStackApplyError> {
        self.pdf.apply_color_stack(id, target, action)
    }

    pub fn pdf_page_color_stack_restorations(&mut self) -> Vec<crate::PdfColorStackEmission> {
        self.pdf.page_color_stack_restorations()
    }

    #[must_use]
    pub fn pdf_snap_reference(&self) -> (Scaled, Scaled) {
        self.pdf.snap_reference()
    }

    pub fn publish_pdf_traversal_positions(
        &mut self,
        last_position: Option<(Scaled, Scaled)>,
        snap_reference: (Scaled, Scaled),
    ) {
        self.pdf
            .publish_traversal_positions(last_position, snap_reference);
    }

    #[must_use]
    pub fn pdf_font_resource(
        &self,
        font: crate::ids::FontId,
    ) -> Option<crate::PdfFontResourceRecord> {
        self.pdf.font_resource(font)
    }

    pub fn set_pdf_form_artifact(&mut self, object: u32, artifact: crate::PdfFormArtifact) {
        self.pdf.set_form_artifact(object, artifact);
    }

    #[must_use]
    pub fn pdf_form_artifact(&self, object: u32) -> Option<crate::PdfFormArtifact> {
        self.pdf.form_artifact(object).cloned()
    }

    #[must_use]
    pub fn pdf_form_color_rollback(&self) -> crate::PdfFormColorRollback {
        self.pdf.form_color_rollback()
    }

    pub fn rollback_pdf_form_colors(&mut self, rollback: crate::PdfFormColorRollback) {
        self.pdf.rollback_form_colors(rollback);
    }

    pub fn open_output_stream(&mut self, slot: crate::world::StreamSlot, path: std::path::PathBuf) {
        self.world.open_out(slot, path);
    }

    pub fn close_output_stream(&mut self, slot: crate::world::StreamSlot) -> bool {
        self.world.close_out(slot)
    }

    #[must_use]
    pub fn output_stream_is_open(&self, slot: crate::world::StreamSlot) -> bool {
        self.world.write_stream_is_open(slot)
    }

    pub fn set_last_stream_open_context(&mut self, context: impl Into<String>) {
        self.world.set_last_stream_open_context(context);
    }

    /// Publishes one complete page-lifetime list inside this admitted episode.
    pub fn publish_page_nodes(&mut self, nodes: Vec<crate::node::Node>) -> PageListId {
        for node in &nodes {
            self.assert_live_node_font_roots(node);
        }
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

    /// Borrows the live page-builder sequence for diagnostic rendering only.
    pub fn current_page_nodes(&self) -> impl DoubleEndedIterator<Item = &crate::node::Node> {
        self.page.current_page()
    }

    #[must_use]
    pub fn page_dimension(&self, dimension: crate::page::PageDimension) -> Scaled {
        self.page.dimension(dimension, false)
    }

    #[must_use]
    pub fn page_dimension_with_output_routine(
        &self,
        dimension: crate::page::PageDimension,
        output_routine_active: bool,
    ) -> Scaled {
        self.page.dimension(dimension, output_routine_active)
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

    pub fn start_new_page(&mut self) {
        self.page.start_new_page();
    }

    #[must_use]
    pub fn page_contributions(&self) -> &std::collections::VecDeque<crate::node::Node> {
        self.page.contribution()
    }

    pub fn append_page_contribution(&mut self, node: crate::node::Node) {
        self.assert_live_node_font_roots(&node);
        self.page.push_contribution(node);
    }

    pub fn prepend_page_contribution(&mut self, node: crate::node::Node) {
        self.assert_live_node_font_roots(&node);
        self.page.prepend_contribution(node);
    }

    pub fn prepend_page_contributions(&mut self, nodes: Vec<crate::node::Node>) {
        for node in &nodes {
            self.assert_live_node_font_roots(node);
        }
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
        self.assert_live_node_font_roots(&node);
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
        self.assert_live_node_font_roots(&node);
        self.page.push_page_discard(node);
    }

    pub fn take_page_discards(&mut self) -> Vec<crate::node::Node> {
        self.page.take_page_discards()
    }

    pub fn clear_page_discards(&mut self) {
        self.page.clear_page_discards();
    }

    pub fn set_split_discards(&mut self, nodes: Vec<crate::node::Node>) {
        for node in &nodes {
            self.assert_live_node_font_roots(node);
        }
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

    pub fn set_page_mark(
        &mut self,
        mark: crate::page::PageMark,
        value: crate::node::NodeTokenList,
    ) {
        self.page.set_mark(mark, value);
        self.dependencies
            .mark_changed(DependencyKey::PageMark(mark.index()));
        self.dependencies
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class: 0,
            });
    }

    pub fn clear_page_mark(&mut self, mark: crate::page::PageMark) {
        self.page.clear_mark(mark);
        self.dependencies
            .mark_changed(DependencyKey::PageMark(mark.index()));
        self.dependencies
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class: 0,
            });
    }

    pub fn set_page_mark_class(
        &mut self,
        mark: crate::page::PageMark,
        class: u16,
        value: crate::node::NodeTokenList,
    ) {
        self.page.set_mark_class(mark, class, value);
        self.dependencies
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class,
            });
        if class == 0 {
            self.dependencies
                .mark_changed(DependencyKey::PageMark(mark.index()));
        }
    }

    pub fn clear_page_mark_class(&mut self, mark: crate::page::PageMark, class: u16) {
        self.page.clear_mark_class(mark, class);
        self.dependencies
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class,
            });
        if class == 0 {
            self.dependencies
                .mark_changed(DependencyKey::PageMark(mark.index()));
        }
    }

    pub fn page_mark_classes(&self) -> impl Iterator<Item = u16> + '_ {
        self.page.mark_class_ids()
    }

    #[must_use]
    pub fn paragraph_shape(&self) -> Vec<ParagraphShapeLine> {
        let Some(root) = self
            .token_parameter(crate::env::banks::TokParam::PAR_SHAPE_INTERNAL)
            .expect("paragraph-shape parameter is admitted")
        else {
            return Vec::new();
        };
        let words = self.token_list(root);
        assert_eq!(words.len() % 8, 0, "paragraph-shape payload is truncated");
        words
            .chunks_exact(8)
            .map(|chunk| {
                let mut bytes = [0_u8; 8];
                for (byte, word) in bytes.iter_mut().zip(chunk) {
                    *byte = match word.semantic_token() {
                        crate::token::Token::Char {
                            ch,
                            cat: crate::token::Catcode::Invalid,
                        } if u8::try_from(ch as u32).is_ok() => ch as u8,
                        _ => panic!("paragraph-shape payload contains a non-byte token"),
                    };
                }
                ParagraphShapeLine {
                    indent: Scaled::from_raw(i32::from_le_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3],
                    ])),
                    width: Scaled::from_raw(i32::from_le_bytes([
                        bytes[4], bytes[5], bytes[6], bytes[7],
                    ])),
                }
            })
            .collect()
    }

    #[must_use]
    pub fn paragraph_shape_len(&self) -> usize {
        self.token_parameter(crate::env::banks::TokParam::PAR_SHAPE_INTERNAL)
            .expect("paragraph-shape parameter is admitted")
            .map_or(0, |root| {
                let len = self.token_list(root).len();
                assert_eq!(len % 8, 0, "paragraph-shape payload is truncated");
                len / 8
            })
    }

    /// Projects the logical TeX `\parshape` length from its internal scoped
    /// storage cell for detached assignment/restoration diagnostics.
    ///
    /// The internal token-parameter coordinate and byte encoding remain
    /// private to `tex-state`; callers receive only the user-visible logical
    /// value when the supplied cell is the paragraph-shape cell.
    #[must_use]
    pub fn restored_paragraph_shape_len(
        &self,
        parameter: crate::env::banks::TokParam,
        root: Option<TokenListId<G>>,
    ) -> Option<usize> {
        if parameter != crate::env::banks::TokParam::PAR_SHAPE_INTERNAL {
            return None;
        }
        Some(root.map_or(0, |root| {
            let len = self.token_list(root).len();
            assert_eq!(len % 8, 0, "paragraph-shape payload is truncated");
            len / 8
        }))
    }

    #[must_use]
    pub fn paragraph_shape_dimension(&self, line: i32, width: bool) -> Scaled {
        if line <= 0 {
            return Scaled::from_raw(0);
        }
        let shape = self.paragraph_shape();
        if shape.is_empty() {
            return Scaled::from_raw(0);
        }
        let line = (line as usize).min(shape.len()) - 1;
        if width {
            shape[line].width
        } else {
            shape[line].indent
        }
    }

    pub fn assign_paragraph_shape(
        &mut self,
        lines: &[ParagraphShapeLine],
        scope: AssignmentScope,
    ) -> Result<(), DurableAllocationError> {
        let parameter = crate::env::banks::TokParam::PAR_SHAPE_INTERNAL;
        if lines.is_empty()
            && scope == AssignmentScope::Local
            && self
                .token_parameter(parameter)
                .expect("paragraph-shape parameter is admitted")
                .is_none()
        {
            return Ok(());
        }
        let root = if lines.is_empty() {
            None
        } else {
            let words = lines
                .iter()
                .flat_map(|line| {
                    line.indent
                        .raw()
                        .to_le_bytes()
                        .into_iter()
                        .chain(line.width.raw().to_le_bytes())
                        .map(|byte| {
                            TokenWord::pack(crate::token::Token::Char {
                                ch: char::from(byte),
                                cat: crate::token::Catcode::Invalid,
                            })
                        })
                })
                .collect::<Vec<_>>();
            Some(self.allocate_token_list(&words)?)
        };
        self.assign_token_parameter(parameter, root, scope)
            .expect("paragraph-shape parameter is admitted");
        Ok(())
    }

    #[must_use]
    pub fn penalty_array(&self, kind: PenaltyArrayKind) -> Vec<i32> {
        let Some(root) = self
            .token_parameter(kind.storage())
            .expect("penalty-array parameter is admitted")
        else {
            return Vec::new();
        };
        let words = self.token_list(root);
        assert_eq!(words.len() % 4, 0, "penalty-array payload is truncated");
        words
            .chunks_exact(4)
            .map(|chunk| {
                let mut bytes = [0_u8; 4];
                for (byte, word) in bytes.iter_mut().zip(chunk) {
                    *byte = match word.semantic_token() {
                        crate::token::Token::Char {
                            ch,
                            cat: crate::token::Catcode::Invalid,
                        } if u8::try_from(ch as u32).is_ok() => ch as u8,
                        _ => panic!("penalty-array payload contains a non-byte token"),
                    };
                }
                i32::from_le_bytes(bytes)
            })
            .collect()
    }

    #[must_use]
    pub fn penalty_array_value(&self, kind: PenaltyArrayKind, index: i32) -> i32 {
        let values = self.penalty_array(kind);
        if index <= 0 || values.is_empty() {
            return if index == 0 { values.len() as i32 } else { 0 };
        }
        values[(index as usize).min(values.len()) - 1]
    }

    pub fn assign_penalty_array(
        &mut self,
        kind: PenaltyArrayKind,
        values: &[i32],
        scope: AssignmentScope,
    ) -> Result<(), DurableAllocationError> {
        let parameter = kind.storage();
        if values.is_empty()
            && scope == AssignmentScope::Local
            && self
                .token_parameter(parameter)
                .expect("penalty-array parameter is admitted")
                .is_none()
        {
            return Ok(());
        }
        let root = if values.is_empty() {
            None
        } else {
            let words = values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .map(|byte| {
                    TokenWord::pack(crate::token::Token::Char {
                        ch: char::from(byte),
                        cat: crate::token::Catcode::Invalid,
                    })
                })
                .collect::<Vec<_>>();
            Some(self.allocate_token_list(&words)?)
        };
        self.assign_token_parameter(parameter, root, scope)
            .expect("penalty-array parameter is admitted");
        Ok(())
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

    pub fn begin_diagnostic<'effects>(
        &self,
        effects: &'effects mut crate::diagnostic::DiagnosticEffects,
    ) -> crate::diagnostic::Diagnostic<'effects> {
        let tracing_online = self.int_param(IntParam::TRACING_ONLINE);
        let newline = self.int_param(IntParam::NEWLINE_CHAR);
        let escape = self.int_param(IntParam::ESCAPE_CHAR);
        crate::diagnostic::Diagnostic::from_parts(
            effects,
            *self.interaction_mode,
            self.error_context_widths.max_print_line(),
            tracing_online,
            newline,
            escape,
        )
    }

    /// Appends one ordinary selector-routed print as an operation-local,
    /// rollback-safe effect. Publication remains outside admission so the
    /// World evaluates its then-current per-sink partial lines atomically.
    pub fn append_ordinary_print_effect(
        &self,
        effects: &mut crate::diagnostic::DiagnosticEffects,
        text: String,
    ) {
        effects.push_ordinary_rendered(
            *self.interaction_mode,
            self.error_context_widths.max_print_line(),
            text,
        );
    }

    /// Opens §245's diagnostic channel with terminal-and-log routing.
    ///
    /// e-TeX 2.6 change 17.516 temporarily forces `tracing_online` while
    /// reporting level-two missing characters. This is a print-channel
    /// override, not an eqtb assignment.
    pub fn begin_online_diagnostic<'effects>(
        &self,
        effects: &'effects mut crate::diagnostic::DiagnosticEffects,
    ) -> crate::diagnostic::Diagnostic<'effects> {
        let newline = self.int_param(IntParam::NEWLINE_CHAR);
        let escape = self.int_param(IntParam::ESCAPE_CHAR);
        crate::diagnostic::Diagnostic::from_parts(
            effects,
            *self.interaction_mode,
            self.error_context_widths.max_print_line(),
            1,
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

    /// Publishes detached diagnostics that canonically precede a synchronous
    /// live World-facing print sequence.
    ///
    /// Recoverable error reporting still owns live interaction/history state
    /// in [`crate::World`]. A caller that has already rendered tracing or
    /// restoration output into an operation-local collector must cross this
    /// explicit bridge before opening [`crate::print::ErrorReport`] or a
    /// legacy synchronous diagnostic headline, or that live output would
    /// overtake the earlier program. No World handle or partial-line state
    /// escapes this admission.
    pub fn publish_diagnostic_effects_before_synchronous_print(
        &mut self,
        effects: &mut crate::diagnostic::DiagnosticEffects,
    ) {
        self.world
            .publish_diagnostic_effects(std::mem::take(effects));
    }

    pub fn error_report(&mut self) -> crate::print::ErrorReport<'_, G> {
        let newline = self.int_param(IntParam::NEWLINE_CHAR);
        let escape = self.int_param(IntParam::ESCAPE_CHAR);
        crate::print::ErrorReport::bare_from_parts(
            self.world,
            self.interaction_mode,
            self.error_context_widths,
            newline,
            escape,
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

    pub fn known_control_sequence(&mut self, name: &str) -> Option<Symbol> {
        let symbol = self.symbol(name)?;
        self.admitted
            .state()
            .admit_symbol(symbol)
            .expect("session-known symbol fits the current meaning bank");
        Some(symbol)
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

    pub fn intern(&mut self, name: &str) -> Result<SymbolId, crate::interner::InternerError> {
        let id = self.interner.intern(name)?;
        self.admitted
            .state()
            .admit_symbol(id.symbol())
            .expect("interned symbol fits the meaning bank");
        Ok(id)
    }

    pub fn intern_retained_pool_string(
        &mut self,
        value: &str,
    ) -> Result<SymbolId, crate::interner::InternerError> {
        let id = self.intern(value)?;
        self.record_retained_strings(RetainedStringAllocation::one(value));
        Ok(id)
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
        let symbol = self
            .symbol(name)
            .unwrap_or_else(|| self.intern_control_sequence(name));
        if matches!(
            self.meaning(symbol),
            ResolvedMeaning::Static(Meaning::Undefined)
        ) {
            self.admitted
                .state()
                .assign_meaning(
                    symbol,
                    MeaningWord::from_static(Meaning::Relax),
                    crate::AssignmentScope::Local,
                )
                .expect("undefined control sequence is admitted");
        }
        symbol
    }

    pub fn set_provisional_meaning(&mut self, symbol: Symbol, meaning: Meaning, global: bool) {
        if let Meaning::Font(font) = meaning {
            self.validate_font_root(font)
                .expect("provisional font meaning retains a live admitted font");
        }
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

fn box_glue_setting_text<List>(node: &crate::node::BoxNode<List>) -> String {
    if node.glue_sign == crate::node::Sign::Normal || node.glue_set.is_zero() {
        return String::new();
    }
    let sign = match node.glue_sign {
        crate::node::Sign::Normal => unreachable!("normal glue was handled above"),
        crate::node::Sign::Stretching => "",
        crate::node::Sign::Shrinking => " -",
    };
    let numerator = i64::from(node.glue_set.numerator()) * i64::from(Scaled::UNITY);
    let denominator = i64::from(node.glue_set.denominator());
    let raw = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        -((-numerator + denominator / 2) / denominator)
    };
    let ratio =
        crate::scaled::print_scaled(Scaled::from_raw(i32::try_from(raw).unwrap_or(i32::MAX)));
    let order = match node.glue_order {
        crate::glue::Order::Normal => "",
        crate::glue::Order::Fil => "fil",
        crate::glue::Order::Fill => "fill",
        crate::glue::Order::Filll => "filll",
    };
    format!(", glue set{sign} {ratio}{order}")
}

fn page_list_semantic_id<G>(
    page_nodes: &PageNodeArena,
    fonts: &FontStore,
    state: &DenseState<G>,
    root: PageListId,
) -> crate::state_hash::StateHashFragment {
    struct PageSemanticHasher<'a, G> {
        page_nodes: &'a PageNodeArena,
        fonts: &'a FontStore,
        state: &'a DenseState<G>,
        hasher: crate::state_hash::StateHasher,
    }

    impl<G> PageSemanticHasher<'_, G> {
        fn list(&mut self, root: PageListId) {
            let list = self
                .page_nodes
                .get(root)
                .expect("PDF form root belongs to the live page arena");
            self.hasher.usize(list.nodes().len());
            for node in list.nodes() {
                self.node(node);
            }
        }

        fn font(&mut self, font: crate::ids::FontId) {
            let recipe = self.fonts.artifact_recipe(font);
            self.hasher.str(&format!("{recipe:?}"));
            self.state
                .hash_font_runtime(font, self.fonts.get(font), &mut self.hasher)
                .expect("live PDF form font has runtime state");
        }

        fn node(&mut self, node: &crate::node::Node) {
            node.visit_semantic_node_lists(|child| {
                self.hasher.tag(0xf0);
                self.list(*child);
            });
            let mut value = node.clone();
            value.visit_node_lists_mut(|child| *child = PageListId::empty());
            match &mut value {
                crate::node::Node::Char { font, .. } => {
                    self.font(*font);
                    *font = crate::font::NULL_FONT;
                }
                crate::node::Node::Lig { font, .. } => {
                    self.font(*font);
                    *font = crate::font::NULL_FONT;
                }
                crate::node::Node::MarginKern { font, .. } => {
                    self.font(*font);
                    *font = crate::font::NULL_FONT;
                }
                _ => {}
            }
            value.erase_diagnostic_sidecars();
            self.hasher.str(&format!("{value:?}"));
        }
    }

    let mut projection = PageSemanticHasher {
        page_nodes,
        fonts,
        state,
        hasher: crate::state_hash::StateHasher::new_exact(0x7064_665f_666f_726d),
    };
    projection.list(root);
    projection.hasher.finish_fragment()
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
