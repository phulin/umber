//! Aggregate state access reserved for the canonical command processor.
//!
//! This boundary is deliberately interpretation-neutral: it owns no command
//! state and decides no command behavior. Operations are added here only when
//! they represent typed reads or mutations of [`Universe`] state.

use crate::{
    ChangedAt, DependencyCodeTable, DependencyEngineField, DependencyFontField, DependencyKey,
    DependencyValue, DependencyWorldField, JobClock, TracedTokenList, TrackedRegionBarrier,
    Universe,
    cell::{BankTag, CellId},
    env::banks::{GlueParam, IntParam, TokParam},
    glue::GlueSpec,
    ids::{FontId, GlueId},
    ids::{MacroDefinitionId, TokenListId},
    interner::{Symbol, SymbolId},
    macro_store::{
        MacroDefinitionProvenance, MacroDefinitionRef, MacroDefinitionView, MacroParameterPattern,
    },
    math::MathFontSize,
    meaning::{InternalInteger, Meaning},
    page::{PageInteger, PageMark},
    provenance::SynthesizedOriginKind,
    source_map::{SourceDescriptor, SourceMapError, SourcePos},
    token::{Catcode, OriginId, Token, TracedTokenWord},
    token_store::{TokenListRef, TokenListView},
};

/// Borrow-scoped aggregate access to live TeX state.
///
/// Construct this through [`Universe::command_context`]. The private
/// `Universe` borrow prevents consumers from retaining the context in a
/// snapshot or bypassing the aggregate mutation boundary.
#[derive(Debug)]
pub struct CommandContext<'a> {
    universe: &'a mut Universe,
}

impl CommandContext<'_> {
    fn observe_dependency(&mut self, key: DependencyKey) {
        if !self.universe.dependency_region_is_active() {
            return;
        }
        self.universe.track_dependency(key);
        if let Some(value) = self.universe.semantic_dependency_value(key) {
            self.universe.record_dependency(key, value);
        } else {
            self.universe
                .poison_tracked_region(TrackedRegionBarrier::UnsupportedCommandState);
        }
    }

    fn observe_cell(&mut self, bank: BankTag, index: u32) {
        self.observe_dependency(DependencyKey::Cell(CellId::new(bank, index)));
    }

    fn observe_font(&mut self, font: FontId, field: DependencyFontField, index: u32) {
        self.observe_dependency(DependencyKey::Font {
            field,
            font: font.raw(),
            index,
        });
    }

    /// Records a command-owned canonical projection that is not stored by
    /// [`Universe`], such as the input or conditional stack.
    pub fn observe_command_projection(&mut self, key: DependencyKey, value: DependencyValue) {
        if !self.universe.dependency_region_is_active() {
            return;
        }
        self.universe.track_dependency(key);
        self.universe.record_dependency(key, value);
    }

    /// Whether the aggregate owner installed the one outer tracked region.
    #[must_use]
    pub fn tracked_region_is_active(&self) -> bool {
        self.universe.dependency_region_is_active()
    }

    /// Fails closed before an unsupported command-owned fact can influence
    /// the containing tracked region.
    #[inline(always)]
    pub fn unsupported_command_state(&mut self) {
        self.universe
            .poison_tracked_region(TrackedRegionBarrier::UnsupportedCommandState);
    }

    /// Fails closed before an unvirtualized host capability can influence the
    /// containing tracked region.
    #[inline(always)]
    pub fn unsupported_host_capability(&mut self) {
        self.universe
            .poison_tracked_region(TrackedRegionBarrier::UnsupportedHostCapability);
    }

    /// Reads the immutable job-start clock through its precise World
    /// dependency. Conversions such as pdfTeX's `\pdfcreationdate` depend on
    /// this value without gaining general host or clock access.
    #[must_use]
    pub fn job_clock(&mut self) -> JobClock {
        self.observe_dependency(DependencyKey::World {
            field: DependencyWorldField::JobClock,
            index: 0,
        });
        self.universe.world().job_clock()
    }

    /// Conservatively records the command renderer's shared parameter reads.
    /// Rendering helpers borrow this context immutably, so the processor owns
    /// this once-per-region aggregate admission before any helper can run.
    pub fn observe_command_rendering_dependencies(&mut self) {
        for param in [
            IntParam::ESCAPE_CHAR,
            IntParam::NEWLINE_CHAR,
            IntParam::END_LINE_CHAR,
            IntParam::TRACING_IFS,
            IntParam::GLOBAL_DEFS,
            IntParam::TRACING_COMMANDS,
            IntParam::PDF_OUTPUT,
            IntParam::TEX_XET_STATE,
        ] {
            self.observe_cell(BankTag::IntParam, u32::from(param.raw()));
        }
        self.observe_dependency(DependencyKey::CodeGeneration(DependencyCodeTable::Catcode));
    }

    /// Reads an integer parameter for diagnostic rendering outside semantic
    /// command evaluation.
    #[must_use]
    pub fn untracked_int_param(&self, param: IntParam) -> i32 {
        self.universe.int_param(param)
    }

    /// Reads a category code for diagnostic token rendering.
    #[must_use]
    pub fn untracked_catcode(&self, ch: char) -> Catcode {
        self.universe.catcode(ch)
    }
    /// Appends string-pool content using TeX82 §§59--60's active-selector
    /// `print` spelling, including `\newlinechar` handling.
    pub fn append_selector_string_text(&self, raw: &str, text: &mut String) {
        let newlinechar = u32::try_from(self.universe.int_param(IntParam::NEWLINE_CHAR))
            .ok()
            .filter(|&code| code <= u8::MAX.into())
            .and_then(char::from_u32);
        for ch in raw.chars() {
            if Some(ch) == newlinechar {
                text.push('\n');
            } else {
                crate::token_show::append_tex_print_char(ch, text);
            }
        }
    }

    /// Appends one token using TeX82 §262's `show_token_list` spelling.
    pub fn append_token_show_text(&self, token: Token, text: &mut String) {
        crate::token_show::append_token_show_text(self.universe, token, text);
    }

    /// Appends one token through TeX82's `new_string` selector spelling.
    pub fn append_token_string_text(&self, token: Token, text: &mut String) {
        crate::token_show::append_token_string_text(self.universe, token, text);
    }

    /// Appends one token using TeX82 §262's active-selector `show_token_list`
    /// spelling, including `print_cs` separators and `\newlinechar` handling.
    pub fn append_token_selector_text(&self, token: Token, text: &mut String) {
        let newlinechar = u32::try_from(self.universe.int_param(IntParam::NEWLINE_CHAR))
            .ok()
            .filter(|&code| code <= u8::MAX.into())
            .and_then(char::from_u32);
        crate::token_show::append_token_selector_text(self.universe, token, newlinechar, text);
    }

    /// Reads the process-selected TeX82 §79 pseudoprint widths.
    #[must_use]
    pub fn error_context_widths(&self) -> crate::print::ErrorContextWidths {
        self.universe.error_context_widths()
    }

    /// Reads TeX82 §960's live `trie_not_ready` state.
    #[must_use]
    pub fn hyphenation_patterns_open(&mut self) -> bool {
        self.unsupported_command_state();
        self.universe.hyphenation_patterns_open()
    }

    /// Reads TeX82 §963's existing-pattern test without moving insertion
    /// ownership out of the executor.
    #[must_use]
    pub fn contains_hyphenation_pattern_for_language(
        &mut self,
        language: u8,
        letters: &[char],
    ) -> bool {
        self.observe_dependency(DependencyKey::HyphenationPatterns(language));
        self.universe
            .contains_hyphenation_pattern_for_language(language, letters)
    }

    /// Opens tex.web §245's shared `\tracing*` diagnostic channel through the
    /// live command aggregate borrow.
    ///
    /// e-TeX's `\tracingifs` renders its `{...}` trace lines from inside the
    /// command core itself (tex.web part 28's `conditional`/`pass_text`),
    /// unlike the executor-driven `\tracingassigns`/`\tracinggroups` traces,
    /// so it needs this channel directly rather than a queued diagnostic.
    pub fn begin_diagnostic(&mut self) -> crate::diagnostic::Diagnostic<'_> {
        self.unsupported_command_state();
        self.universe.begin_diagnostic()
    }

    /// tex.web §75's ordinary print scope at the current interaction-implied
    /// selector, through the live command aggregate borrow.
    ///
    /// e-TeX's `\tracingnesting` warnings (`group_warning`/`if_warning`/
    /// `file_warning`) print directly at this ambient selector rather than
    /// through §245's `begin_diagnostic` redirect: unlike
    /// `\tracingassigns`/`\tracinggroups`/`\tracingifs`, they are not gated
    /// on `\tracingonline` and must reach the terminal whenever the ambient
    /// selector already does.
    pub fn printer(&mut self) -> crate::print::Printer<'_> {
        self.unsupported_command_state();
        self.universe.printer()
    }

    /// Opens tex.web §73's recoverable-error report through the live command
    /// aggregate borrow.
    ///
    /// The report remains tied to this context's borrow of [`Universe`], so a
    /// command processor can emit canonical scanner diagnostics without
    /// retaining the print channel in command state, snapshots, or formats.
    pub fn print_err(&mut self, text: &str) -> crate::print::ErrorReport<'_> {
        self.unsupported_command_state();
        self.universe.print_err(text)
    }

    /// tex.web §537's `(name`, through the live command aggregate borrow.
    pub fn print_file_open(&mut self, name: &str) {
        crate::file_framing::print_file_open(self.universe, name);
    }

    /// tex.web §362's bare `)`.
    ///
    /// §362 prints this from inside `get_next`, one statement ahead of
    /// `check_outer_validity`, so the command core has to be able to reach it
    /// without waiting for the engine driver; see [`crate::file_framing`].
    pub fn print_file_close(&mut self) {
        crate::file_framing::print_file_close(self.universe);
    }

    /// Resumes a scanner report around tex.web's intervening `back_error`.
    pub fn resume_error_report(
        &mut self,
        deferred: crate::print::DeferredErrorReport,
    ) -> crate::print::ErrorReport<'_> {
        self.unsupported_command_state();
        self.universe.resume_error_report(deferred)
    }

    /// Takes one ErrorStop request for mutation by the canonical input owner.
    pub fn take_error_recovery_request(&mut self) -> Option<crate::print::ErrorRecoveryRequest> {
        self.universe.take_error_recovery_request()
    }

    /// Re-enters the ErrorStop advice loop after token deletion.
    pub fn continue_error_stop_dialog(&mut self, context: &str) -> crate::print::ErrorOutcome {
        self.universe.continue_error_stop_dialog(context)
    }

    /// Registers immutable command input before its first source delivery.
    ///
    /// The command processor retains the backing and supplies the descriptor;
    /// this aggregate boundary owns timeline registration and makes repeated
    /// registration of the same source idempotent.
    pub fn register_source(
        &mut self,
        source: crate::SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<SourcePos, SourceMapError> {
        self.universe.register_source(source, descriptor)
    }

    /// Allocates an exact registered-source spelling range.
    #[must_use]
    pub fn source_range_origin(
        &mut self,
        source: crate::SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        self.universe
            .source_range_origin(source, byte_offset, byte_end)
    }

    #[must_use]
    pub fn source_range_origin_ref(
        &mut self,
        source: crate::SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> crate::provenance::OriginRef {
        self.universe
            .source_range_origin_ref(source, byte_offset, byte_end)
    }

    /// Allocates provenance for one ordinary backed source scalar.
    #[must_use]
    pub fn source_token_origin(
        &mut self,
        source: crate::SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        self.universe
            .source_token_origin(source, byte_offset, byte_end)
    }

    #[must_use]
    pub fn source_token_origin_ref(
        &mut self,
        source: crate::SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> crate::provenance::OriginRef {
        self.universe
            .source_token_origin_ref(source, byte_offset, byte_end)
    }

    /// Reads one catcode through the aggregate code-table boundary.
    #[must_use]
    pub fn catcode(&mut self, ch: char) -> Catcode {
        self.observe_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Catcode,
            scalar: ch.into(),
        });
        self.universe.catcode(ch)
    }

    /// Reads one lowercase code through the aggregate code-table boundary.
    #[must_use]
    pub fn lccode(&mut self, ch: char) -> crate::code_tables::LcCode {
        self.observe_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Lccode,
            scalar: ch.into(),
        });
        self.universe.lccode(ch)
    }

    /// Reads one saved hyphenation code, pdfTeX's `\savinghyphcodes` table.
    ///
    /// `None` means the language saved no table at all, so TeX82 §935/§961's
    /// plain `lc_code` applies; `Some(None)` is a saved code of zero.
    #[must_use]
    pub fn saved_hyphenation_code(&mut self, language: u8, ch: char) -> Option<Option<char>> {
        self.observe_dependency(DependencyKey::HyphenationCodes(language));
        self.universe.saved_hyphenation_code(language, ch)
    }

    /// Reads one uppercase code through the aggregate code-table boundary.
    #[must_use]
    pub fn uccode(&mut self, ch: char) -> crate::code_tables::UcCode {
        self.observe_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Uccode,
            scalar: ch.into(),
        });
        self.universe.uccode(ch)
    }

    /// Reads one space factor code through the aggregate code-table boundary.
    #[must_use]
    pub fn sfcode(&mut self, ch: char) -> crate::code_tables::SfCode {
        self.observe_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Sfcode,
            scalar: ch.into(),
        });
        self.universe.sfcode(ch)
    }

    /// Reads one math code through the aggregate code-table boundary.
    #[must_use]
    pub fn mathcode(&mut self, ch: char) -> crate::code_tables::MathCode {
        self.observe_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Mathcode,
            scalar: ch.into(),
        });
        self.universe.mathcode(ch)
    }

    /// Reads one delimiter code through the aggregate code-table boundary.
    #[must_use]
    pub fn delcode(&mut self, ch: char) -> crate::code_tables::DelCode {
        self.observe_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Delcode,
            scalar: ch.into(),
        });
        self.universe.delcode(ch)
    }

    /// Reads the number of lines in TeX's current `\parshape` value.
    #[must_use]
    pub fn paragraph_shape_len(&self) -> usize {
        self.universe.paragraph_shape_len()
    }

    /// Reads one component of TeX's current `\parshape` value, repeating its
    /// final line for positive indexes beyond the explicitly stored shape.
    #[must_use]
    pub fn paragraph_shape_dimension(&self, line: i32, width: bool) -> crate::scaled::Scaled {
        self.universe.paragraph_shape_dimension(line, width)
    }

    /// Reads one e-TeX penalty-array element using its numeric enquiry rule.
    #[must_use]
    pub fn penalty_array_value(&self, kind: crate::PenaltyArrayKind, index: i32) -> i32 {
        self.universe.penalty_array_value(kind, index)
    }

    /// Interns a fixed-namespace control-sequence spelling without assigning
    /// it a meaning. Section 259 hash entries use the explicit neighboring
    /// method so their allocator occupancy remains observable.
    #[must_use]
    pub fn intern_control_sequence(&mut self, name: &str) -> Symbol {
        self.universe.intern(name).symbol()
    }

    /// Interns a spelling through TeX82 §259's hash-table path.
    #[must_use]
    pub fn intern_hash_control_sequence(&mut self, name: &str) -> Symbol {
        self.universe.intern_hash_control_sequence(name).symbol()
    }

    /// Interns an inaccessible engine-owned fixed `eqtb` control sequence.
    #[must_use]
    pub fn intern_internal_control_sequence(&mut self, name: &str) -> Symbol {
        self.universe
            .intern_internal_control_sequence(name)
            .symbol()
    }

    /// Looks up an already-known control-sequence spelling without creating
    /// one.
    ///
    /// This is TeX82 §259's `id_lookup` with `no_new_control_sequence` set:
    /// an absent name resolves to §222's dummy location instead of becoming a
    /// new hash entry.
    #[must_use]
    pub fn known_control_sequence(&self, name: &str) -> Option<Symbol> {
        self.universe.symbol(name).map(|symbol| symbol.symbol())
    }

    /// Interns a name built by `\\csname` and installs TeX's `\\relax`
    /// meaning only when that name was previously undefined.
    #[must_use]
    pub fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        self.universe.intern_relaxed_control_sequence(name).symbol()
    }

    /// Returns the immutable semantic words of one stored token list.
    #[must_use]
    pub fn tokens(&self, id: TokenListId) -> TokenListView<'_> {
        self.universe.tokens(id)
    }

    /// Returns the copy-only identity for a live token list.
    #[must_use]
    pub fn token_list_ref(&self, id: TokenListId) -> TokenListRef {
        self.universe.token_list_ref(id)
    }

    /// Materializes one cold structural token origin from the provenance archive.
    pub fn materialize_token_word(
        &mut self,
        id: TokenListId,
        index: usize,
    ) -> Option<crate::token::RootedTracedTokenWord> {
        self.universe.materialize_token_word(id, index)
    }

    /// Returns TeX82's inaccessible frozen end-v sentinel for canonical
    /// alignment-template retirement. Input delivery owns when it is emitted;
    /// callers cannot intern or otherwise manufacture this token.
    #[must_use]
    pub fn frozen_endv_token(&self) -> Token {
        Token::frozen_endv()
    }

    /// Returns TeX82's inaccessible `frozen_end_template` token. Input
    /// delivery emits it when an exhausted v-template remains retained until
    /// typed `do_endv` retirement.
    #[must_use]
    pub fn frozen_end_template_token(&self) -> Token {
        Token::frozen_end_template()
    }

    /// Freezes a scanner-owned traced token sequence through the aggregate
    /// content and provenance stores.  Command scanners may allocate their
    /// immutable result, but retain no wider store or host capability.
    #[must_use]
    pub fn finish_traced_token_list(&mut self, tokens: &[TracedTokenWord]) -> TracedTokenList {
        self.universe.finish_traced_token_list(tokens)
    }

    /// Freezes an already rooted scanner accumulator without reconstructing
    /// ownership from compact ids.
    #[must_use]
    pub fn finish_rooted_traced_token_list(
        &mut self,
        tokens: &crate::token::RootedTracedTokenBuffer,
    ) -> TracedTokenList {
        self.universe.finish_rooted_traced_token_list(tokens)
    }

    /// Records a command-scanner token allocation at its completed boundary.
    pub fn observe_transient_token_words(&mut self, words: usize) {
        self.universe.observe_transient_token_words(words);
    }

    /// Reads the immutable spelling of a control sequence.
    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.universe.resolve(symbol)
    }

    /// Returns the TeX control-sequence namespace of an interned symbol.
    #[must_use]
    pub fn control_sequence_kind(&self, symbol: Symbol) -> crate::interner::ControlSequenceKind {
        self.universe.control_sequence_kind(symbol)
    }

    /// Finds an already-interned control-sequence spelling without creating
    /// a new entry in the command namespace.
    #[must_use]
    pub fn symbol(&self, name: &str) -> Option<Symbol> {
        self.universe.symbol(name).map(|symbol| symbol.symbol())
    }

    /// Returns the registered canonical spelling of one frozen primitive
    /// meaning, independent of a mutable control-sequence cell.
    #[must_use]
    pub fn primitive_name(&self, meaning: Meaning) -> Option<&str> {
        self.universe.primitive_name(meaning)
    }

    /// Returns the immutable original meaning registered for a primitive
    /// spelling, independent of the live control-sequence cell.
    #[must_use]
    pub fn primitive_meaning(&self, name: &str) -> Option<Meaning> {
        self.universe.primitive_meaning(name)
    }

    /// Reads one integer parameter for canonical expandable conversion.
    #[must_use]
    pub fn int_param(&mut self, param: IntParam) -> i32 {
        self.observe_cell(BankTag::IntParam, u32::from(param.raw()));
        self.universe.int_param(param)
    }

    /// Advances pdfTeX's checkpointed MetaPost-derived random stream and
    /// returns a uniform deviate for the signed bound.
    pub fn pdf_uniform_deviate(&mut self, bound: i32) -> i32 {
        self.universe.pdf_uniform_deviate(bound)
    }

    /// Advances pdfTeX's checkpointed MetaPost-derived random stream and
    /// returns one 16.16 normal deviate.
    pub fn pdf_normal_deviate(&mut self) -> i32 {
        self.universe.pdf_normal_deviate()
    }

    /// Allocates one checkpointed pdfTeX color stack and returns its numeric
    /// identity for the expandable `\pdfcolorstackinit` conversion.
    pub fn allocate_pdf_color_stack(
        &mut self,
        mode: crate::PdfColorStackMode,
        restore_at_page_start: bool,
        initial: Vec<u8>,
    ) -> Result<u32, crate::PdfColorStackCapacityError> {
        self.universe
            .allocate_pdf_color_stack(mode, restore_at_page_start, initial)
    }

    /// Replaces pdfTeX's ungrouped, checkpointed last-match state.
    pub fn set_pdf_match_state(
        &mut self,
        haystack: Vec<u8>,
        captures: Vec<Option<(u32, u32)>>,
        slot_count: u32,
        matched: bool,
    ) {
        self.universe
            .set_pdf_match_state(haystack, captures, slot_count, matched);
    }

    /// Reads one capture from pdfTeX's checkpointed last-match state.
    #[must_use]
    pub fn pdf_match_capture(&self, index: u32) -> Option<(u32, &[u8])> {
        self.universe.pdf_match_capture(index)
    }

    /// Resolves pdfTeX's independent resource identity for one form object.
    #[must_use]
    pub fn pdf_form_resource(&self, object: u32) -> Option<u32> {
        self.universe.pdf_form(object).map(|form| form.resource())
    }

    /// Reads one token parameter for direct `\\the` insertion.
    #[must_use]
    pub fn tok_param(&mut self, param: TokParam) -> TokenListId {
        self.observe_cell(BankTag::TokParam, u32::from(param.raw()));
        self.universe.tok_param(param)
    }

    /// Reads a token parameter without conflating null with an assigned empty list.
    #[must_use]
    pub fn tok_param_option(&mut self, param: TokParam) -> Option<TokenListId> {
        self.observe_cell(BankTag::TokParam, u32::from(param.raw()));
        self.universe.tok_param_option(param)
    }

    /// Reads one count register for canonical expandable conversion.
    #[must_use]
    pub fn count(&mut self, index: u16) -> i32 {
        self.observe_cell(BankTag::Count, u32::from(index));
        self.universe.count(index)
    }

    /// Reads one dimension register through the aggregate state boundary.
    #[must_use]
    pub fn dimen(&mut self, index: u16) -> crate::scaled::Scaled {
        self.observe_cell(BankTag::Dimen, u32::from(index));
        self.universe.dimen(index)
    }

    /// Reads one dimension parameter through the aggregate state boundary.
    #[must_use]
    pub fn dimen_param(&mut self, index: u16) -> crate::scaled::Scaled {
        self.observe_cell(BankTag::DimenParam, u32::from(index));
        self.universe
            .dimen_param(crate::env::banks::DimenParam::new(index))
    }

    /// Reads one page dimension through the aggregate state boundary.
    #[must_use]
    pub fn page_dimension(
        &mut self,
        dimension: crate::page::PageDimension,
    ) -> crate::scaled::Scaled {
        self.observe_dependency(DependencyKey::PageDimension(dimension.index()));
        self.universe.page_dimension(dimension)
    }

    /// Reads one page-builder integer through the aggregate boundary.
    #[must_use]
    pub fn page_integer(&mut self, integer: PageInteger) -> i32 {
        self.observe_dependency(DependencyKey::PageInteger(integer.index()));
        self.universe.page_integer(integer)
    }

    /// Reads one immutable glue specification through the aggregate boundary.
    #[must_use]
    pub fn glue(&self, id: GlueId) -> crate::glue::GlueSpec {
        self.universe.glue(id)
    }

    /// Resolves a copy-only glue reference while this context is borrowed.
    #[must_use]
    pub fn glue_spec(&self, root: crate::glue::GlueSpecRef) -> crate::glue::GlueSpec {
        self.universe.glue_spec(root)
    }

    /// Reads one skip register through the aggregate boundary.
    #[must_use]
    pub fn skip(&mut self, index: u16) -> GlueId {
        self.observe_cell(BankTag::Skip, u32::from(index));
        self.universe.skip(index)
    }

    /// Reads one mu-skip register through the aggregate boundary.
    #[must_use]
    pub fn muskip(&mut self, index: u16) -> GlueId {
        self.observe_cell(BankTag::Muskip, u32::from(index));
        self.universe.muskip(index)
    }

    /// Reads one glue parameter through the aggregate boundary.
    #[must_use]
    pub fn glue_param(&mut self, index: u16) -> GlueId {
        self.observe_cell(BankTag::GlueParam, u32::from(index));
        self.universe.glue_param(GlueParam::new(index))
    }

    /// Applies TeX82 §759's preamble-local `\tabskip` definition.
    pub fn define_preamble_tabskip(&mut self, value: GlueSpec, global: bool) -> GlueId {
        let value = self.universe.intern_glue(value);
        if global {
            self.universe
                .set_glue_param_global(GlueParam::TAB_SKIP, value.id());
        } else {
            self.universe
                .set_glue_param(GlueParam::TAB_SKIP, value.id());
        }
        value.id()
    }

    /// Reads an engine-owned internal integer through the aggregate boundary.
    #[must_use]
    pub fn internal_integer(&self, integer: InternalInteger) -> Option<i32> {
        let value = match integer {
            InternalInteger::Badness => self.universe.last_badness(),
            InternalInteger::ETeXVersion => 2,
            InternalInteger::PdfTeXVersion => 140,
            InternalInteger::PdfElapsedTime => self.universe.pdf_elapsed_time(),
            InternalInteger::PdfRandomSeed => self.universe.pdf_random_seed(),
            InternalInteger::PdfShellEscape => self.universe.pdf_shell_escape_status(),
            InternalInteger::PdfLastObject => self.universe.pdf_last_object() as i32,
            InternalInteger::PdfLastAnnot => self.universe.pdf_last_annotation() as i32,
            InternalInteger::PdfLastLink => self.universe.pdf_last_link() as i32,
            InternalInteger::PdfLastXForm => self.universe.pdf_last_form() as i32,
            InternalInteger::PdfLastXPos => self.universe.pdf_last_position().0.raw(),
            InternalInteger::PdfLastYPos => self.universe.pdf_last_position().1.raw(),
            InternalInteger::PdfLastXImage => self.universe.pdf_last_ximage() as i32,
            InternalInteger::PdfLastXImagePages => self.universe.pdf_last_ximage_pages() as i32,
            InternalInteger::PdfLastXImageColorDepth => {
                i32::from(self.universe.pdf_last_ximage_color_depth())
            }
            InternalInteger::PdfReturnValue => self.universe.pdf_return_value(),
            InternalInteger::LastNodeType => self.universe.page_last_node_type(),
            InternalInteger::InputLineNumber
            | InternalInteger::CurrentGroupLevel
            | InternalInteger::CurrentGroupType
            | InternalInteger::CurrentIfLevel
            | InternalInteger::CurrentIfType
            | InternalInteger::CurrentIfBranch => return None,
        };
        Some(value)
    }

    /// Returns the indirect object of a one-based page that has already
    /// shipped. Future and absent pages deliberately have no speculative
    /// identity at the command boundary.
    #[must_use]
    pub fn pdf_page_object(&self, page: u32) -> Option<u32> {
        page.checked_sub(1)
            .and_then(|index| self.universe.pdf_pages().get(index as usize))
            .map(|record| record.page_object())
    }

    /// Reads detached external-image metadata without exposing the PDF ledger
    /// or granting allocation authority to the command processor.
    #[must_use]
    pub fn pdf_external_image(
        &mut self,
        id: crate::PdfExternalImageId,
    ) -> Option<crate::PdfExternalImageMetadata> {
        self.universe.pdf_external_image(id)
    }

    /// Reads e-TeX's live group enquiries without exposing group ownership.
    #[must_use]
    pub fn current_group_values(&self) -> (i32, i32) {
        (
            i32::try_from(self.universe.group_depth()).unwrap_or(i32::MAX),
            self.universe
                .innermost_group_kind()
                .map_or(0, crate::GroupKind::etex_code),
        )
    }

    #[must_use]
    pub fn current_group_lineage(&self) -> Option<u64> {
        self.universe
            .group_frames()
            .next_back()
            .map(|frame| frame.lineage())
    }

    /// The outermost-first lineage of every live group boundary.
    ///
    /// e-TeX's `file_warning` saves `cur_boundary`, whose enclosing save-stack
    /// chain distinguishes a surviving boundary from same-depth replacements.
    #[must_use]
    pub fn group_lineages(&self) -> Vec<u64> {
        self.universe
            .group_frames()
            .map(|frame| frame.lineage())
            .collect()
    }

    /// Every open group at or above `min_depth` (1-based group level),
    /// outermost first, as `(level, group_text, entered_line)`: e-TeX 2.6
    /// [49.1293]'s `print_group` fields for `\tracingnesting`'s
    /// `file_warning`, which needs the same per-group display `\showgroups`
    /// does but is not itself gated behind `\showgroups`'s own diagnostic
    /// channel.
    #[must_use]
    pub fn group_frames_from(&self, min_depth: usize) -> Vec<(usize, &'static str, u32)> {
        self.universe
            .group_frames()
            .enumerate()
            .skip(min_depth)
            .map(|(index, frame)| (index + 1, frame.kind().group_text(), frame.entered_line()))
            .collect()
    }

    /// tex.web §245's other half: `if history=spotless then
    /// history:=warning_issued`, for a warning printed outside
    /// `begin_diagnostic` (`\tracingnesting`'s `group_warning`/`if_warning`/
    /// `file_warning`, which are not `stat`-gated and so do not open a
    /// diagnostic scope at all).
    pub fn record_warning_history(&mut self) {
        self.universe
            .world_mut()
            .error_channel_mut()
            .record_warning_history();
    }

    /// Classifies a box register without exposing node-store ownership.
    #[must_use]
    pub fn box_kind(&mut self, index: u16) -> Option<CommandBoxKind> {
        self.observe_cell(BankTag::Box, u32::from(index));
        let list = self.universe.box_reg_ref(index)?;
        match list.nodes().first()? {
            crate::node_arena::NodeRef::HList(_) => Some(CommandBoxKind::Horizontal),
            crate::node_arena::NodeRef::VList(_) => Some(CommandBoxKind::Vertical),
            _ => None,
        }
    }

    /// Reads TeX82 §501's `\ifeof` predicate, `read_open[n] = closed`.
    ///
    /// A stream that was never opened, one whose `\openin` found no file, and
    /// one whose final line has already been consumed are all closed, so this
    /// is a single boolean read rather than an exposure of stream ownership.
    /// The caller supplies a `scan_four_bit_int`-bounded slot (§433).
    #[must_use]
    pub fn read_stream_at_eof(&mut self, stream: crate::world::StreamSlot) -> bool {
        self.observe_dependency(DependencyKey::InputStream(stream.raw()));
        self.universe.world().input_stream_eof(stream)
    }

    /// Acquires one input line, exactly as tex.web §31's `input_ln` does.
    ///
    /// This is the command core's only line-acquisition operation. tex.web
    /// reads every line that is not already in a registered file through
    /// `input_ln`: §363's `firm_up_the_line` replacement, §360's `*` prompt,
    /// and §484-§486's `\read` lines all funnel into it, and §71's
    /// `prompt_input(#)` is defined as `print(#); term_input`, where
    /// `term_input` is `input_ln(term_in,true)` plus its transcript echo.
    /// Modelling that one procedure once is what keeps §363 and §483 from
    /// each growing a private line path.
    ///
    /// The returned line has its trailing blanks removed, per §31 ("either
    /// `last=first` ... or `buffer[last-1]<>" "`"). `None` is §31's `false`
    /// return: for [`CommandLineSource::Terminal`] the terminal is at end of
    /// file, which §71 turns into `fatal_error("End of file on the
    /// terminal!")`; for [`CommandLineSource::Stream`] the stream is not open
    /// at all, which is §483's `read_open[m]=closed` and selects §484's
    /// terminal branch instead. A stream whose last line has already been
    /// consumed returns `Some("")` and closes itself, which is §486's
    /// appended empty line, not an absence of one.
    ///
    /// Acquisition is bounded by construction: every line source is owned by
    /// [`Universe`], so the command core performs no I/O of its own, never
    /// opens a file or a stream, and observes only what the aggregate already
    /// holds or what the driver's own terminal reader hands back.
    pub fn input_ln(&mut self, source: CommandLineSource<'_>) -> Option<String> {
        match source {
            CommandLineSource::Terminal { .. } => self.unsupported_host_capability(),
            CommandLineSource::Stream(stream) => {
                self.observe_dependency(DependencyKey::InputStream(stream.raw()));
            }
        }
        let line = match source {
            CommandLineSource::Terminal { prompt } => {
                // §71: `prompt_input(#) == wake_up_terminal; print(#);
                // term_input`. The prompt is printed by this operation so
                // that the command core never acquires a print channel of
                // its own just to ask for a line.
                if !prompt.is_empty() {
                    self.universe
                        .world_mut()
                        .write_text(crate::world::PrintSink::TerminalAndLog, prompt);
                }
                let line = self.universe.world_mut().read_terminal_line();
                if let Ok(Some(line)) = &line {
                    self.universe.world_mut().echo_terminal_input(line);
                }
                line
            }
            CommandLineSource::Stream(stream) => self.universe.world_mut().read_stream_line(stream),
        };
        // §31 has no error return: a source that cannot deliver a line is
        // indistinguishable from one that has been entirely read.
        let mut line = line.ok().flatten()?;
        line.truncate(line.trim_end_matches(' ').len());
        let content = crate::world::ContentHash::from_bytes(line.as_bytes());
        self.observe_command_projection(
            DependencyKey::InputRecord(content),
            DependencyValue::Content(content),
        );
        self.observe_command_projection(
            DependencyKey::PhysicalLine {
                content,
                terminator: 0,
            },
            DependencyValue::Content(content),
        );
        Some(line)
    }

    /// Reads tex.web's `interaction>nonstop_mode` test.
    ///
    /// §363, §360, and §484 all gate terminal acquisition on it, and §71
    /// states outright that `prompt_input` "is never called when
    /// `interaction<scroll_mode`".
    #[must_use]
    pub fn interaction_permits_terminal_input(&mut self) -> bool {
        self.observe_dependency(DependencyKey::Engine(
            DependencyEngineField::InteractionMode,
        ));
        matches!(
            self.universe.interaction_mode(),
            crate::InteractionMode::Scroll | crate::InteractionMode::ErrorStop
        )
    }

    /// Returns e-TeX's numeric interaction-mode internal quantity.
    #[must_use]
    pub fn interaction_mode_value(&mut self) -> i32 {
        self.observe_dependency(DependencyKey::Engine(
            DependencyEngineField::InteractionMode,
        ));
        match self.universe.interaction_mode() {
            crate::InteractionMode::Batch => 0,
            crate::InteractionMode::Nonstop => 1,
            crate::InteractionMode::Scroll => 2,
            crate::InteractionMode::ErrorStop => 3,
        }
    }

    /// Commits completed TeX82 `make_string` allocations from the command
    /// core without exposing the pool ledger itself.
    pub fn record_string_pool_allocations(&mut self, strings: usize, characters: usize) {
        self.universe
            .record_string_pool_allocations(strings, characters);
    }

    /// Retains one Web2C string-pool value, recycling an equal prior string.
    pub fn slow_make_string_pool_string(&mut self, value: &str) {
        self.universe.slow_make_string_pool_string(value);
    }

    /// Retains one new string even when an equal value already exists.
    pub fn make_string_pool_string(&mut self, value: &str) {
        self.universe.make_string_pool_string(value);
    }

    /// Reads one box-register dimension through the aggregate state boundary.
    ///
    /// A void box has no dimension; TeX's internal-quantity scanner maps
    /// that case to zero at the command semantic layer.
    #[must_use]
    pub fn box_dimension(
        &mut self,
        index: u16,
        dimension: crate::BoxDimension,
    ) -> Option<crate::scaled::Scaled> {
        self.observe_cell(BankTag::Box, u32::from(index));
        self.universe.box_dimension(index, dimension)
    }

    /// Reads one pdfTeX margin-kern enquiry through typed box-list state.
    #[must_use]
    pub fn box_margin_kern(
        &mut self,
        index: u16,
        side: crate::node::MarginKernSide,
    ) -> Option<crate::scaled::Scaled> {
        self.observe_cell(BankTag::Box, u32::from(index));
        self.universe.box_margin_kern(index, side)
    }

    /// Reads one token register for direct `\\the` insertion.
    #[must_use]
    pub fn toks(&mut self, index: u16) -> TokenListId {
        self.observe_cell(BankTag::Toks, u32::from(index));
        self.universe.toks(index)
    }

    /// Reads one TeX82 page-mark slot for expandable mark retrieval.
    #[must_use]
    pub fn page_mark(&mut self, mark: PageMark) -> TokenListId {
        self.observe_dependency(DependencyKey::PageMark(mark.index()));
        self.universe.page_mark(mark)
    }

    #[must_use]
    pub fn page_mark_value(&mut self, mark: PageMark) -> Option<TokenListId> {
        self.observe_dependency(DependencyKey::PageMark(mark.index()));
        self.universe.page_mark_value(mark)
    }

    /// Reads one e-TeX mark-class slot for expandable mark retrieval.
    #[must_use]
    pub fn page_mark_class(&mut self, mark: PageMark, class: u16) -> TokenListId {
        self.observe_dependency(DependencyKey::PageMarkClass {
            mark: mark.index(),
            class,
        });
        self.universe.page_mark_class(mark, class)
    }

    /// Distinguishes an absent sparse mark from a present empty token list.
    #[must_use]
    pub fn page_mark_class_value(&mut self, mark: PageMark, class: u16) -> Option<TokenListId> {
        self.observe_dependency(DependencyKey::PageMarkClass {
            mark: mark.index(),
            class,
        });
        self.universe.page_mark_class_value(mark, class)
    }

    /// Reads a font's immutable external name for `\\fontname` and meaning.
    #[must_use]
    pub fn font_name(&mut self, font: FontId) -> String {
        self.observe_font(font, DependencyFontField::Name, 0);
        self.universe.font_name(font)
    }

    /// TeX82 §578's `find_font_dimen` decision, made before §1253 scans
    /// `=<dimen>`; see [`crate::stores::Stores::font_dimen_writable`].
    #[must_use]
    pub fn font_dimen_writable(&mut self, font: FontId, number: u32) -> bool {
        self.observe_font(font, DependencyFontField::ParameterCount, 0);
        self.unsupported_command_state();
        self.universe.font_dimen_writable(font, number)
    }

    /// TeX82 §578's `find_font_dimen(false)` decision for an internal
    /// `\\fontdimen` enquiry. Unlike the assignment form, an enquiry cannot
    /// extend even the most recently loaded font.
    #[must_use]
    pub fn font_dimen_readable(&mut self, font: FontId, number: u32) -> bool {
        self.observe_font(font, DependencyFontField::ParameterCount, 0);
        number > 0 && number <= self.universe.font_parameter_count(font)
    }

    /// Reads a font's immutable external name without `\fontname`'s size
    /// suffix.
    #[must_use]
    pub fn font_external_name(&self, font: FontId) -> &str {
        self.universe.font(font).name()
    }

    /// Reads a font's selected size for TeX82 §298's `print_cmd_chr`.
    #[must_use]
    pub fn font_size(&self, font: FontId) -> crate::scaled::Scaled {
        self.universe.font(font).size()
    }

    /// Reads a font size as a semantic command enquiry.
    #[must_use]
    pub fn tracked_font_size(&mut self, font: FontId) -> crate::scaled::Scaled {
        self.observe_font(font, DependencyFontField::Metrics, 0);
        self.universe.font(font).size()
    }

    /// Reads a font's design size for TeX82 §298's `print_cmd_chr`.
    #[must_use]
    pub fn font_design_size(&self, font: FontId) -> crate::scaled::Scaled {
        self.universe.font(font).design_size()
    }

    /// Reads one pdfTeX per-font byte-code table through the canonical
    /// checkpointed font-state owner.
    #[must_use]
    pub fn pdf_font_code(&mut self, table: crate::PdfFontCode, font: FontId, code: u8) -> i32 {
        let table_index = match table {
            crate::PdfFontCode::Lp => 0,
            crate::PdfFontCode::Rp => 1,
            crate::PdfFontCode::Ef => 2,
            crate::PdfFontCode::Tag => 3,
            crate::PdfFontCode::Knbs => 4,
            crate::PdfFontCode::Stbs => 5,
            crate::PdfFontCode::Shbs => 6,
            crate::PdfFontCode::Knbc => 7,
            crate::PdfFontCode::Knac => 8,
        };
        self.observe_font(
            font,
            DependencyFontField::PdfCode,
            table_index * 256 + u32::from(code),
        );
        self.universe.pdf_font_code(table, font, code)
    }

    /// Reads one immutable TFM character metric for e-TeX font enquiries.
    #[must_use]
    pub fn font_char_metrics(
        &mut self,
        font: FontId,
        code: u8,
    ) -> Option<crate::font::CharMetrics> {
        self.observe_font(font, DependencyFontField::Metrics, u32::from(code));
        self.universe.font_char_metrics(font, code)
    }

    /// Reads the control-sequence identity assigned to a font.
    #[must_use]
    pub fn font_identifier_symbol(&mut self, font: FontId) -> Option<Symbol> {
        self.observe_font(font, DependencyFontField::Identifier, 0);
        self.universe
            .font_identifier_symbol(font)
            .map(SymbolId::symbol)
    }

    /// Reads the currently selected font through the command boundary.
    #[must_use]
    pub fn current_font(&mut self) -> FontId {
        self.observe_cell(BankTag::CurrentFont, 0);
        self.universe.current_font()
    }

    /// Validates and freezes TeX's job magnification, for `scan_dimen`'s
    /// `true` unit prefix (tex.web's `prepare_mag`).
    ///
    /// The returned diagnostic is the aggregate's own report of an illegal or
    /// incompatible `\mag`; presenting it is the caller's concern.
    pub fn prepare_mag(&mut self) -> (i32, Option<crate::PrepareMagDiagnostic>) {
        self.universe.prepare_mag()
    }

    /// Reads one current-font parameter through the command boundary.
    #[must_use]
    pub fn current_font_parameter(&mut self, number: u32) -> crate::scaled::Scaled {
        let font = self.current_font();
        self.observe_font(font, DependencyFontField::Parameter, number);
        self.universe.font_parameter(font, number)
    }

    /// Reads one named font's `\\fontdimen` parameter through the command
    /// boundary. TeX82 §424's `scan_something_internal` reaches this through
    /// `assign_font_dimen`'s "Fetch a font dimension" (§8548): an out-of-range
    /// or non-positive parameter number recovers as zero, matching the
    /// pre-canonical `Variable::FontDimen` read policy already established in
    /// `tex-exec`'s `variable_access` module.
    #[must_use]
    pub fn font_dimen(&mut self, font: FontId, number: u32) -> crate::scaled::Scaled {
        self.observe_font(font, DependencyFontField::Parameter, number);
        self.universe.font_parameter(font, number)
    }

    /// Reads one named font's `\\hyphenchar` through the command boundary.
    ///
    /// TeX82 §426's "Fetch a font integer" is `assign_font_int`'s half of
    /// `scan_something_internal`: `scan_font_ident` selects the font and
    /// `hyphen_char[cur_val]` is delivered at `int_val`.
    #[must_use]
    pub fn font_hyphen_char(&mut self, font: FontId) -> i32 {
        self.observe_font(font, DependencyFontField::HyphenChar, 0);
        self.universe.font_hyphen_char(font)
    }

    /// Reads one named font's `\\skewchar` through the command boundary.
    ///
    /// The `m<>0` half of TeX82 §426's "Fetch a font integer".
    #[must_use]
    pub fn font_skew_char(&mut self, font: FontId) -> i32 {
        self.observe_font(font, DependencyFontField::SkewChar, 0);
        self.universe.font_skew_char(font)
    }

    /// Reads one font-family selector through the command boundary.
    #[must_use]
    pub fn math_family_font(&mut self, size: MathFontSize, family: u8) -> FontId {
        let size_index = match size {
            MathFontSize::Text => 0,
            MathFontSize::Script => 1,
            MathFontSize::ScriptScript => 2,
        };
        self.observe_cell(BankTag::MathFamilyFont, size_index * 16 + u32::from(family));
        self.universe.math_family_font(size, family)
    }

    /// Returns the mutation stamp for a typed aggregate-state dependency.
    #[must_use]
    pub fn dependency_changed_at(&self, key: DependencyKey) -> ChangedAt {
        self.universe.dependency_changed_at(key)
    }

    /// Records a typed aggregate-state read.
    pub fn track_dependency(&mut self, key: DependencyKey) -> ChangedAt {
        self.universe.track_dependency(key)
    }

    /// Reads the detached semantic value for a typed aggregate-state
    /// dependency.
    #[must_use]
    pub fn semantic_dependency_value(&self, key: DependencyKey) -> Option<DependencyValue> {
        self.universe.semantic_dependency_value(key)
    }

    /// Resolves a control sequence's current meaning and records that semantic
    /// read for the active dependency region.
    #[must_use]
    pub fn meaning(&mut self, symbol: Symbol) -> Meaning {
        let key = DependencyKey::Cell(CellId::new(BankTag::Meaning, symbol.raw()));
        self.observe_dependency(key);
        self.universe.meaning(symbol)
    }

    /// Applies a command-scanner-owned provisional meaning. TeX82 §1224
    /// temporarily makes a definition target `\\relax` before it scans an
    /// expanded integer, preventing a self-reference from expanding.
    pub fn set_provisional_meaning(&mut self, symbol: Symbol, meaning: Meaning, global: bool) {
        if global {
            self.universe.set_meaning_global(symbol, meaning);
        } else {
            self.universe.set_meaning(symbol, meaning);
        }
    }

    /// Interns the distinct control sequence represented by an active
    /// character, if it has not already been interned.
    #[must_use]
    pub fn intern_active_character(&mut self, ch: char) -> Symbol {
        self.universe.intern_active_character(ch).symbol()
    }

    /// The eqtb text of a frozen control sequence, for §262's `print_cs`.
    #[must_use]
    pub fn frozen_primitive_name(&self, token: Token) -> Option<&str> {
        self.universe.frozen_primitive_name(token)
    }

    /// Resolves an engine-owned frozen token without consulting a mutable
    /// control-sequence meaning cell.
    #[must_use]
    pub fn frozen_primitive_meaning(&self, token: Token) -> Option<Meaning> {
        self.universe.frozen_primitive_meaning(token)
    }

    /// Returns the immutable replay token for a registered primitive.
    ///
    /// Command recovery uses this rather than a mutable control-sequence
    /// spelling for TeX's `frozen_cr`, `frozen_fi`, and `frozen_par` tokens.
    #[must_use]
    pub fn primitive_token(&self, name: &str) -> Option<Token> {
        self.universe.primitive_token(name)
    }

    /// Returns diagnostic provenance retained beside an immutable macro
    /// definition. This is deliberately separate from the definition's
    /// semantic token lists.
    #[must_use]
    pub fn macro_definition_provenance(
        &mut self,
        definition: MacroDefinitionId,
    ) -> MacroDefinitionProvenance {
        self.universe.macro_definition_provenance(definition)
    }

    /// Reads one immutable macro definition through the command-state boundary.
    #[must_use]
    pub fn macro_definition(&self, definition: MacroDefinitionId) -> MacroDefinitionView<'_> {
        self.universe.macro_definition(definition)
    }

    /// Returns the copy-only identity for one live definition occurrence.
    #[must_use]
    pub fn macro_definition_ref(&self, definition: MacroDefinitionId) -> MacroDefinitionRef {
        self.universe.macro_definition_ref(definition)
    }

    /// Materializes one cold replacement origin from the provenance archive.
    pub fn materialize_macro_replacement_word(
        &mut self,
        definition: MacroDefinitionId,
        index: usize,
    ) -> Option<crate::token::RootedTracedTokenWord> {
        self.universe
            .materialize_macro_replacement_word(definition, index)
    }

    /// Returns TeX82's definition-head identity for detached observation.
    #[must_use]
    pub fn macro_definition_observation_operand(&self, definition: MacroDefinitionId) -> i64 {
        self.universe
            .macro_definition_observation_operand(definition)
    }

    /// Reads the prevalidated parameter-marker layout for one macro definition.
    #[must_use]
    pub fn macro_definition_parameter_pattern(
        &self,
        definition: MacroDefinitionId,
    ) -> MacroParameterPattern {
        self.universe.macro_definition_parameter_pattern(definition)
    }

    /// Allocates one rollback-coupled macro invocation node.
    ///
    /// The command machine supplies the live parent invocation from its
    /// activation stack. The aggregate state owns arena allocation, so an
    /// invocation frame never stores an arena handle outside the usual origin
    /// representation.
    pub fn macro_invocation_origin(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId {
        self.universe.macro_invocation_origin(
            definition,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    /// Allocates invocation provenance from the observation coordinate held
    /// by an already admitted macro chunk.
    pub fn macro_invocation_origin_from_nonowning_operand(
        &mut self,
        definition_operand: u64,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId {
        self.universe
            .macro_invocation_origin_from_nonowning_operand(
                definition_operand,
                invocation,
                definition_origin,
                parent_invocation,
            )
    }

    /// Creates one structurally rooted macro expansion frame. Its parent,
    /// invocation, and definition positions are strong child edges.
    pub fn macro_invocation_frame(
        &mut self,
        definition: MacroDefinitionId,
        invocation: crate::provenance::OriginRef,
        definition_origin: crate::provenance::OriginRef,
        parent_invocation: crate::provenance::OriginRef,
    ) -> crate::provenance::ExpansionFrameRef {
        self.universe.macro_invocation_frame(
            definition,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    /// Allocates provenance for a token manufactured by canonical expansion.
    pub fn synthesized_origin(
        &mut self,
        kind: SynthesizedOriginKind,
        parent: OriginId,
    ) -> OriginId {
        self.universe.synthesized_origin(kind, parent)
    }

    pub fn synthesized_origin_ref(
        &mut self,
        kind: SynthesizedOriginKind,
        parent: crate::provenance::OriginRef,
    ) -> crate::provenance::OriginRef {
        self.universe.synthesized_origin_ref(kind, parent)
    }

    #[must_use]
    pub fn origin_ref(&self, id: OriginId) -> Option<crate::provenance::OriginRef> {
        self.universe.origin_ref(id)
    }

    pub fn materialize_origin_ref(&mut self, id: OriginId) -> Option<crate::provenance::OriginRef> {
        self.universe.materialize_origin_ref(id)
    }
}

/// The line source one [`CommandContext::input_ln`] acquisition names.
///
/// tex.web's `input_ln` takes the Pascal file to read; Umber names the two
/// the command core can reach instead, because neither is a file it may open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandLineSource<'a> {
    /// tex.web §71's `term_in`, reached through `prompt_input(#)`: `print(#)`
    /// runs first, then `term_input` reads and echoes one line. An empty
    /// prompt is §484's `prompt_input("")`, which prints nothing.
    Terminal { prompt: &'a str },
    /// tex.web §480's `read_file[m]`, read by §31's `input_ln` directly.
    Stream(crate::world::StreamSlot),
}

/// Aggregate classification used by the command conditional boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandBoxKind {
    Horizontal,
    Vertical,
}

impl Universe {
    /// Borrows the interpretation-neutral aggregate boundary used by the
    /// canonical command processor.
    pub fn command_context(&mut self) -> CommandContext<'_> {
        CommandContext { universe: self }
    }
}

#[cfg(test)]
mod tests {
    use super::CommandLineSource;
    use crate::{
        DependencyKey, DependencyValue, InteractionMode, Universe,
        cell::{BankTag, CellId},
        meaning::InternalInteger,
        print::{ErrorOutcome, ErrorRecoveryRequest},
        world::StreamSlot,
    };

    #[test]
    fn command_context_records_exact_reads_and_ignores_unrelated_mutation() {
        let mut universe = Universe::new();
        universe.set_count(7, 11);
        let mark = universe.begin_tracked_region().expect("start region");
        assert_eq!(universe.command_context().count(7), 11);
        let mut observations = universe
            .finish_tracked_region(mark)
            .expect("finish region")
            .observations()
            .to_vec();
        let key = DependencyKey::Cell(CellId::new(BankTag::Count, 7));
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].key, key);
        assert_eq!(observations[0].value, DependencyValue::Integer(11));

        universe.set_count(8, 12);
        assert!(universe.validate_dependencies(&mut observations, |_| {
            panic!("unrelated mutation must retain the stamp fast path")
        }));
        universe.set_count(7, 13);
        assert!(
            !universe
                .validate_dependencies(&mut observations, |_| { DependencyValue::Integer(13) })
        );
    }

    #[test]
    fn command_context_transfers_error_stop_recovery_once() {
        let mut universe = Universe::new();
        universe
            .world_mut()
            .push_memory_terminal_line("Ireplacement")
            .expect("memory terminal accepts an ErrorStop answer");
        assert_eq!(
            universe.print_err("Controlled error").error(),
            ErrorOutcome::Continue
        );
        let world_scans = universe.testing_tracked_world_scan_calls();

        {
            let mut context = universe.command_context();
            assert_eq!(
                context.take_error_recovery_request(),
                Some(ErrorRecoveryRequest::Insert("replacement".to_owned()))
            );
            assert_eq!(context.take_error_recovery_request(), None);
        }
        assert_eq!(universe.testing_tracked_world_scan_calls(), world_scans);
    }

    #[test]
    fn command_font_code_read_tracks_the_exact_table_entry() {
        let mut universe = Universe::new();
        let font = crate::font::NULL_FONT;
        universe.set_pdf_font_code(crate::PdfFontCode::Lp, font, 65, 17);
        let mark = universe.begin_tracked_region().expect("start region");
        assert_eq!(
            universe
                .command_context()
                .pdf_font_code(crate::PdfFontCode::Lp, font, 65),
            17
        );
        let mut observations = universe
            .finish_tracked_region(mark)
            .expect("finish region")
            .observations()
            .to_vec();
        assert_eq!(observations.len(), 1);

        universe.set_pdf_font_code(crate::PdfFontCode::Rp, font, 65, 18);
        assert!(universe.validate_dependencies(&mut observations, |_| {
            panic!("nearby PDF font-code entry is unrelated")
        }));
        universe.set_pdf_font_code(crate::PdfFontCode::Lp, font, 65, 19);
        assert!(
            !universe
                .validate_dependencies(&mut observations, |_| { DependencyValue::Integer(19) })
        );
    }

    #[test]
    fn command_context_publishes_the_live_pdf_return_value() {
        let mut universe = Universe::new();
        universe.set_pdf_return_value(-1);

        assert_eq!(
            universe
                .command_context()
                .internal_integer(InternalInteger::PdfReturnValue),
            Some(-1)
        );
    }

    #[test]
    fn input_ln_removes_trailing_blanks_from_an_acquired_terminal_line() {
        // tex.web §31: "Trailing blanks are removed from the line; thus,
        // either `last=first` ... or `buffer[last-1]<>" "`."
        let mut universe = Universe::new();
        universe
            .world_mut()
            .push_memory_terminal_line("typed   ")
            .expect("memory world accepts terminal input");

        assert_eq!(
            universe
                .command_context()
                .input_ln(CommandLineSource::Terminal { prompt: "=>" }),
            Some("typed".to_owned())
        );
    }

    #[test]
    fn input_ln_reports_an_exhausted_terminal_as_section_31s_false_return() {
        let mut universe = Universe::new();

        assert_eq!(
            universe
                .command_context()
                .input_ln(CommandLineSource::Terminal { prompt: "" }),
            None
        );
    }

    #[test]
    fn input_ln_reads_stream_lines_and_appends_section_486s_empty_line() {
        // §486: "An empty line is appended at the end of a `read_file`."
        // tex.web reaches it through `input_ln` returning false with
        // `last=first`; the aggregate reports the same thing as one final
        // empty line that also closes the stream.
        let mut universe = Universe::new();
        universe
            .world_mut()
            .set_memory_file("stream.tex", b"one  \ntwo\n".to_vec())
            .expect("memory world accepts a seeded file");
        let slot = StreamSlot::new(1);
        universe
            .world_mut()
            .open_in(slot, "stream.tex")
            .expect("stream opens");

        let mut context = universe.command_context();
        assert_eq!(
            context.input_ln(CommandLineSource::Stream(slot)),
            Some("one".to_owned())
        );
        assert_eq!(
            context.input_ln(CommandLineSource::Stream(slot)),
            Some("two".to_owned())
        );
        assert_eq!(
            context.input_ln(CommandLineSource::Stream(slot)),
            Some(String::new())
        );
        assert!(context.read_stream_at_eof(slot));
        // §483 reads `read_open[m]=closed` before acquiring anything, so a
        // closed stream is `None` rather than a further empty line.
        assert_eq!(context.input_ln(CommandLineSource::Stream(slot)), None);
    }

    #[test]
    fn nonstop_and_batch_modes_refuse_terminal_acquisition() {
        // §71: `prompt_input` "is never called when
        // `interaction<scroll_mode`"; §363 and §484 both test
        // `interaction>nonstop_mode` before asking for a line.
        let mut universe = Universe::new();
        for (mode, permitted) in [
            (InteractionMode::Batch, false),
            (InteractionMode::Nonstop, false),
            (InteractionMode::Scroll, true),
            (InteractionMode::ErrorStop, true),
        ] {
            universe.set_interaction_mode(mode);
            assert_eq!(
                universe
                    .command_context()
                    .interaction_permits_terminal_input(),
                permitted,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn terminal_prompt_and_input_echo_are_channel_specific() {
        // tex.web §71: the terminal has already displayed the user's
        // keystrokes, so `term_input` writes only the prompt there. The
        // transcript receives the prompt followed by the accepted line and
        // its newline. Neither deferred channel is visible before commit.
        for mode in [
            InteractionMode::Scroll,
            InteractionMode::ErrorStop,
            InteractionMode::Nonstop,
            InteractionMode::Batch,
        ] {
            let mut universe = Universe::new();
            universe.set_interaction_mode(mode);
            universe
                .world_mut()
                .push_memory_terminal_line("typed")
                .expect("memory world accepts terminal input");

            let line = if universe
                .command_context()
                .interaction_permits_terminal_input()
            {
                universe
                    .command_context()
                    .input_ln(CommandLineSource::Terminal { prompt: "? " })
            } else {
                Some("noninteractive".to_owned())
            };

            let interactive = matches!(mode, InteractionMode::Scroll | InteractionMode::ErrorStop);
            assert_eq!(
                line.as_deref(),
                Some(if interactive {
                    "typed"
                } else {
                    "noninteractive"
                }),
                "{mode:?}"
            );
            assert_eq!(universe.world().memory_terminal_output(), Some(&b""[..]));
            assert_eq!(universe.world().memory_log_output(), Some(&b""[..]));

            let effects = universe.world().effect_pos();
            universe
                .commit_effects(effects)
                .expect("commit terminal acquisition effects");
            assert_eq!(
                universe.world().memory_terminal_output(),
                Some(if interactive { &b"? "[..] } else { &b""[..] }),
                "{mode:?}"
            );
            assert_eq!(
                universe.world().memory_log_output(),
                Some(if interactive {
                    &b"? typed\n"[..]
                } else {
                    &b""[..]
                }),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn terminal_input_echo_wraps_the_transcript_without_wrapping_the_terminal() {
        let mut universe = Universe::new();
        universe
            .world_mut()
            .push_memory_terminal_line("typed")
            .expect("memory world accepts terminal input");
        universe
            .world_mut()
            .write_text(crate::world::PrintSink::Log, &"l".repeat(78));

        assert_eq!(
            universe
                .command_context()
                .input_ln(CommandLineSource::Terminal { prompt: "? " }),
            Some("typed".to_owned())
        );
        let effects = universe.world().effect_pos();
        universe
            .commit_effects(effects)
            .expect("commit independently wrapped acquisition effects");

        assert_eq!(universe.world().memory_terminal_output(), Some(&b"? "[..]));
        assert_eq!(
            universe.world().memory_log_output(),
            Some(format!("{}?\n typed\n", "l".repeat(78)).as_bytes())
        );
    }

    #[test]
    fn exhausted_terminal_does_not_fabricate_an_echoed_line() {
        let mut universe = Universe::new();

        assert_eq!(
            universe
                .command_context()
                .input_ln(CommandLineSource::Terminal { prompt: "? " }),
            None
        );
        let effects = universe.world().effect_pos();
        universe
            .commit_effects(effects)
            .expect("commit terminal EOF prompt");

        assert_eq!(universe.world().memory_terminal_output(), Some(&b"? "[..]));
        assert_eq!(universe.world().memory_log_output(), Some(&b"? "[..]));
    }
}
