//! Generation-branded direct-index TeX state and exact save semantics.

#[path = "env/banks.rs"]
pub mod banks;
mod durable_boxes;
pub use durable_boxes::DurableNodeMetadata;
pub(crate) use durable_boxes::{
    AcceptedDurableBoxTail, DurableBoxCursor, DurableBoxOperation, DurableBoxState,
    DurableFormState, DurableGroupRestoration,
};
mod font_runtime;
#[path = "env/group.rs"]
pub(crate) mod group;

use banks::{
    BankCell, BankError, DenseBank, IntParam, LEVEL_ONE, PARAMETER_COUNT, PagedDenseBank,
    RegisterBank,
};
pub(crate) use font_runtime::DerivedFontRuntimeRequest;
pub(crate) use font_runtime::FontRuntimeCell;
use font_runtime::{AcceptedFontRuntimeTail, BankCellValue, FontRuntimeBank, PreparedFontRuntime};
use group::{GroupFrame, GroupKind, GroupMismatch};

use crate::durable_arena::{GlueId, TokenListId};
use crate::ids::FontId;
use crate::interner::Symbol;
use crate::journal::{
    AcceptedJournalTail, JournalCursor, JournalEntry, Mutation, RestoredGroups, SaveJournal,
    StateOperation,
};
use crate::meaning::{MeaningWord, ResolvedMeaning};
use crate::scaled::Scaled;
use crate::world::JobClock;

#[cfg(test)]
#[path = "env/tests.rs"]
mod tests;

const UNICODE_SCALAR_COUNT: u32 = 0x11_0000;
const MATH_FAMILY_FONT_COUNT: usize = 48;

/// Assignment scope after `\globaldefs` and explicit-prefix resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssignmentScope {
    Local,
    Global,
}

/// One semantic profile layer admitted during fresh engine construction.
///
/// Restored formats do not install these layers: their dense parameter banks
/// are already authoritative.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreshParameterProfile {
    Tex82,
    Etex26,
    Pdftex14029,
}

impl FreshParameterProfile {
    const fn bit(self) -> u8 {
        match self {
            Self::Tex82 => 1 << 0,
            Self::Etex26 => 1 << 1,
            Self::Pdftex14029 => 1 << 2,
        }
    }
}

/// One physical dense-bank value in a fresh profile batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreshParameterDefault {
    Integer(banks::IntParam, i32),
    Dimension(banks::DimenParam, Scaled),
    EmptyGlue(banks::GlueParam),
    EmptyTokens(banks::TokParam),
}

impl FreshParameterDefault {
    const fn cell(self) -> (FreshParameterBank, u16) {
        match self {
            Self::Integer(parameter, _) => (FreshParameterBank::Integer, parameter.raw()),
            Self::Dimension(parameter, _) => (FreshParameterBank::Dimension, parameter.raw()),
            Self::EmptyGlue(parameter) => (FreshParameterBank::Glue, parameter.raw()),
            Self::EmptyTokens(parameter) => (FreshParameterBank::Tokens, parameter.raw()),
        }
    }
}

/// Dense parameter bank named by a rejected fresh-profile entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreshParameterBank {
    Integer,
    Dimension,
    Glue,
    Tokens,
}

impl FreshParameterBank {
    const fn slot(self) -> usize {
        match self {
            Self::Integer => 0,
            Self::Dimension => 1,
            Self::Glue => 2,
            Self::Tokens => 3,
        }
    }
}

/// Result of an exact-once fresh profile installation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreshParameterInstallation {
    Installed,
    AlreadyInstalled,
}

/// A fresh parameter profile batch violated the construction contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreshParameterInstallError {
    Retired,
    MissingTex82Base(FreshParameterProfile),
    DuplicateCell {
        bank: FreshParameterBank,
        index: u16,
    },
}

/// The six eqtb code-table families.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CodeTableKind {
    Catcode,
    Lccode,
    Uccode,
    Sfcode,
    Mathcode,
    Delcode,
}

/// Typed identity of one mutable current-value cell.
// Deliberately not `Hash`: dense first-touch tracking is one direct cell-stamp
// comparison and cannot regress to a coordinate hash table unnoticed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StateCell {
    Meaning(u32),
    Count(u16),
    Dimension(u16),
    TokenRegister(u16),
    GlueRegister(u16),
    BoxRegister(u16),
    MuGlueRegister(u16),
    IntegerParameter(u16),
    DimensionParameter(u16),
    TokenParameter(u16),
    GlueParameter(u16),
    CurrentFont,
    MathFamilyFont(u8),
    Code(CodeTableKind, u32),
    FontRuntime(FontRuntimeCell),
}

/// One mutable cell named by an in-session group-restoration receipt.
///
/// This is a borrow-free coordinate, not a cold DTO: token, glue, node, and
/// definition coordinates carried by the matching value remain valid only
/// under the admitted generation which produced the receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupRestorationCell {
    Meaning(Symbol),
    Count(u16),
    Dimension(u16),
    TokenRegister(u16),
    GlueRegister(u16),
    BoxRegister(u16),
    MuGlueRegister(u16),
    IntegerParameter(u16),
    DimensionParameter(u16),
    TokenParameter(u16),
    GlueParameter(u16),
    CurrentFont,
    MathFamilyFont(u8),
    Code(CodeTableKind, u32),
    FontRuntime(GroupRestorationFontRuntimeCell),
}

/// One mutable per-font cell named by a restoration receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupRestorationFontRuntimeCell {
    ParameterCount(u32),
    Dimension { font: u32, number: u32 },
    HyphenChar(u32),
    SkewChar(u32),
    PdfCode { table: u8, font: u32, code: u8 },
    LigaturesDisabled(u32),
}

/// One exact saved or live state word in a group-restoration receipt.
///
/// The value is owned and contains no borrow or coarse owner. Generation
/// coordinates are deliberately retained because the executor consumes the
/// receipt synchronously under the same [`crate::CommandContext`].
pub enum GroupRestorationValue<G> {
    Meaning(ResolvedMeaning<G>),
    Integer(i32),
    Dimension(Scaled),
    TokenList(Option<TokenListId<G>>),
    Glue(Option<GlueId<G>>),
    NodeList(Option<DurableNodeMetadata>),
    Font(FontId),
    Code(i64),
}

impl<G> Clone for GroupRestorationValue<G> {
    fn clone(&self) -> Self {
        match self {
            Self::Meaning(value) => Self::Meaning(*value),
            Self::Integer(value) => Self::Integer(*value),
            Self::Dimension(value) => Self::Dimension(*value),
            Self::TokenList(value) => Self::TokenList(value.clone()),
            Self::Glue(value) => Self::Glue(*value),
            Self::NodeList(value) => Self::NodeList(*value),
            Self::Font(value) => Self::Font(*value),
            Self::Code(value) => Self::Code(*value),
        }
    }
}

impl<G> core::fmt::Debug for GroupRestorationValue<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Meaning(value) => value.fmt(formatter),
            Self::Integer(value) => value.fmt(formatter),
            Self::Dimension(value) => value.fmt(formatter),
            Self::TokenList(None) => formatter.write_str("TokenList(None)"),
            Self::TokenList(Some(_)) => formatter.write_str("TokenList(Some(..))"),
            Self::Glue(None) => formatter.write_str("Glue(None)"),
            Self::Glue(Some(_)) => formatter.write_str("Glue(Some(..))"),
            Self::NodeList(None) => formatter.write_str("NodeList(None)"),
            Self::NodeList(Some(_)) => formatter.write_str("NodeList(Some(..))"),
            Self::Font(value) => value.fmt(formatter),
            Self::Code(value) => value.fmt(formatter),
        }
    }
}

impl<G> PartialEq for GroupRestorationValue<G> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Meaning(left), Self::Meaning(right)) => left == right,
            (Self::Integer(left), Self::Integer(right)) => left == right,
            (Self::Dimension(left), Self::Dimension(right)) => left == right,
            (Self::TokenList(left), Self::TokenList(right)) => left == right,
            (Self::Glue(left), Self::Glue(right)) => left == right,
            (Self::NodeList(left), Self::NodeList(right)) => left == right,
            (Self::Font(left), Self::Font(right)) => left == right,
            (Self::Code(left), Self::Code(right)) => left == right,
            _ => false,
        }
    }
}

impl<G> Eq for GroupRestorationValue<G> {}

/// Whether TeX restored the saved word or retained a later global word.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupRestorationOutcome {
    Restored,
    Retained,
}

/// The live print controls immediately after one §283 restoration decision.
///
/// Capturing these scalars per entry preserves the order-sensitive case where
/// `unsave` itself restores a tracing or print parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupRestorationTraceState {
    tracing_restores: i32,
    tracing_online: i32,
    newline_char: i32,
    escape_char: i32,
}

impl GroupRestorationTraceState {
    #[must_use]
    pub const fn tracing_restores(self) -> i32 {
        self.tracing_restores
    }

    #[must_use]
    pub const fn tracing_online(self) -> i32 {
        self.tracing_online
    }

    #[must_use]
    pub const fn newline_char(self) -> i32 {
        self.newline_char
    }

    #[must_use]
    pub const fn escape_char(self) -> i32 {
        self.escape_char
    }
}

/// One entry in TeX82 §283's top-down `unsave` restoration order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupRestorationEntry<G> {
    cell: GroupRestorationCell,
    saved: GroupRestorationValue<G>,
    live: GroupRestorationValue<G>,
    outcome: GroupRestorationOutcome,
    trace: GroupRestorationTraceState,
}

impl<G> GroupRestorationEntry<G> {
    #[must_use]
    pub const fn cell(&self) -> GroupRestorationCell {
        self.cell
    }

    #[must_use]
    pub fn saved_value(&self) -> GroupRestorationValue<G> {
        self.saved.clone()
    }

    #[must_use]
    pub fn live_value(&self) -> GroupRestorationValue<G> {
        self.live.clone()
    }

    #[must_use]
    pub const fn outcome(&self) -> GroupRestorationOutcome {
        self.outcome
    }

    #[must_use]
    pub const fn trace_state(&self) -> GroupRestorationTraceState {
        self.trace
    }
}

/// Ordered, borrow-free result of closing one TeX save level.
///
/// This receipt is an admitted in-session handoff, not a serialization or
/// cold-detachment boundary. Consumers must render it synchronously under the
/// generation which produced it and before replaying its `\aftergroup` input.
#[derive(Debug, Eq, PartialEq)]
pub struct GroupRestorationReceipt<G> {
    frame: GroupFrame,
    entries: Vec<GroupRestorationEntry<G>>,
}

impl<G> GroupRestorationReceipt<G> {
    #[must_use]
    pub const fn frame(&self) -> GroupFrame {
        self.frame
    }

    #[must_use]
    pub fn entries(&self) -> &[GroupRestorationEntry<G>] {
        &self.entries
    }

    pub(crate) fn append_durable(
        &mut self,
        restorations: Vec<DurableGroupRestoration>,
        trace: GroupRestorationTraceState,
    ) {
        self.entries.extend(
            restorations
                .into_iter()
                .map(|restoration| GroupRestorationEntry {
                    cell: GroupRestorationCell::BoxRegister(restoration.index),
                    saved: GroupRestorationValue::NodeList(restoration.saved),
                    live: GroupRestorationValue::NodeList(restoration.live),
                    outcome: restoration.outcome,
                    trace,
                }),
        );
    }
}

/// One packed scalar or typed generation coordinate stored by a bank/journal.
pub(crate) enum StateWord<G> {
    Meaning(MeaningWord<G>),
    Integer(i32),
    Dimension(Scaled),
    TokenList(Option<TokenListId<G>>),
    Glue(Option<GlueId<G>>),
    Font(FontId),
    Code(i64),
}

impl<G> Clone for StateWord<G> {
    fn clone(&self) -> Self {
        match self {
            Self::Meaning(value) => Self::Meaning(*value),
            Self::Integer(value) => Self::Integer(*value),
            Self::Dimension(value) => Self::Dimension(*value),
            Self::TokenList(value) => Self::TokenList(value.clone()),
            Self::Glue(value) => Self::Glue(*value),
            Self::Font(value) => Self::Font(*value),
            Self::Code(value) => Self::Code(*value),
        }
    }
}

impl<G> core::fmt::Debug for StateWord<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Meaning(value) => value.fmt(formatter),
            Self::Integer(value) => value.fmt(formatter),
            Self::Dimension(value) => value.fmt(formatter),
            Self::TokenList(None) => formatter.write_str("TokenList(None)"),
            Self::TokenList(Some(_)) => formatter.write_str("TokenList(Some(..))"),
            Self::Glue(None) => formatter.write_str("Glue(None)"),
            Self::Glue(Some(_)) => formatter.write_str("Glue(Some(..))"),
            Self::Font(value) => value.fmt(formatter),
            Self::Code(value) => value.fmt(formatter),
        }
    }
}

impl<G> PartialEq for StateWord<G> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Meaning(left), Self::Meaning(right)) => left == right,
            (Self::Integer(left), Self::Integer(right)) => left == right,
            (Self::Dimension(left), Self::Dimension(right)) => left == right,
            (Self::TokenList(left), Self::TokenList(right)) => left == right,
            (Self::Glue(left), Self::Glue(right)) => left == right,
            (Self::Font(left), Self::Font(right)) => left == right,
            (Self::Code(left), Self::Code(right)) => left == right,
            _ => false,
        }
    }
}

impl<G> Eq for StateWord<G> {}

/// Rejected state construction, access, grouping, or restoration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateError {
    Bank(BankError),
    ForeignSession,
    InvalidCursor,
    CellKindMismatch,
    GroupDepthExhausted,
    GroupLineageExhausted,
    GroupMismatch(GroupMismatch),
    /// A retained coarse owner still prevents whole-generation retirement.
    GenerationInUse,
}

impl From<BankError> for StateError {
    fn from(error: BankError) -> Self {
        Self::Bank(error)
    }
}

/// All eqtb-equivalent current-value banks for one revision generation.
pub(crate) struct DenseState<G> {
    meanings: DenseBank<MeaningWord<G>>,
    counts: RegisterBank<i32>,
    dimensions: RegisterBank<Scaled>,
    token_registers: RegisterBank<Option<TokenListId<G>>>,
    glue_registers: RegisterBank<Option<GlueId<G>>>,
    mu_glue_registers: RegisterBank<Option<GlueId<G>>>,
    integer_parameters: DenseBank<i32>,
    dimension_parameters: DenseBank<Scaled>,
    token_parameters: DenseBank<Option<TokenListId<G>>>,
    glue_parameters: DenseBank<Option<GlueId<G>>>,
    current_font: BankCell<FontId>,
    math_family_fonts: DenseBank<FontId>,
    catcodes: PagedDenseBank<i64>,
    lccodes: PagedDenseBank<i64>,
    uccodes: PagedDenseBank<i64>,
    sfcodes: PagedDenseBank<i64>,
    mathcodes: PagedDenseBank<i64>,
    delcodes: PagedDenseBank<i64>,
    font_runtime: FontRuntimeBank,
    fresh_parameter_profiles: u8,
    journal: Option<SaveJournal<G>>,
    groups: Vec<GroupFrame>,
    next_group_lineage: u64,
    reachable_state_identity_enabled: bool,
}

/// Accepted dense suffix retained while the current candidate mutates the
/// same direct banks from a rooted mark.
pub(crate) struct AcceptedDenseStateTail<G> {
    journal: AcceptedJournalTail<G>,
    font_runtime: AcceptedFontRuntimeTail,
    groups: Vec<GroupFrame>,
    next_group_lineage: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub struct DenseStateCursor {
    font_runtime_rows: u32,
}

impl<G> DenseState<G> {
    pub(crate) fn definition_meanings_match_accepted_checkpoint(
        &self,
        restart: JournalCursor<G>,
        prior: JournalCursor<G>,
        mut definitions_equal: impl FnMut(crate::DefinitionRef<G>, crate::DefinitionRef<G>) -> bool,
    ) -> bool {
        fn meaning_index<G>(delta: &crate::journal::CheckpointDelta<G>) -> Option<u32> {
            match (delta.cell, &delta.alternate) {
                (StateCell::Meaning(index), StateWord::Meaning(_)) => Some(index),
                _ => None,
            }
        }

        for (index, current) in self.meanings.values().enumerate() {
            let index = index as u32;
            let mut restart_value = current;
            let mut candidate_touched = false;
            self.journal().visit_checkpoint_suffix(restart, |delta| {
                if !candidate_touched && meaning_index(delta) == Some(index) {
                    let StateWord::Meaning(value) = delta.alternate else {
                        unreachable!("meaning cell carries a meaning word")
                    };
                    restart_value = value;
                    candidate_touched = true;
                }
            });

            let mut prior_value = restart_value;
            let mut prior_touched = false;
            if !self.journal().visit_detached_prefix(prior, |delta| {
                if meaning_index(delta) == Some(index) {
                    let StateWord::Meaning(value) = delta.alternate else {
                        unreachable!("meaning cell carries a meaning word")
                    };
                    prior_value = value;
                    prior_touched = true;
                }
            }) {
                return false;
            }
            if !(candidate_touched || prior_touched) {
                continue;
            }

            let equal = match (current, prior_value) {
                (MeaningWord::Static(left), MeaningWord::Static(right)) => left == right,
                (MeaningWord::Font(left), MeaningWord::Font(right)) => left == right,
                (
                    MeaningWord::Macro {
                        flags: left_flags,
                        definition: left,
                    },
                    MeaningWord::Macro {
                        flags: right_flags,
                        definition: right,
                    },
                ) => left_flags == right_flags && definitions_equal(left, right),
                _ => false,
            };
            if !equal {
                return false;
            }
        }
        true
    }

    fn journal(&self) -> &SaveJournal<G> {
        self.journal
            .as_ref()
            .expect("dense state journal is not recursively borrowed")
    }

    fn journal_mut(&mut self) -> &mut SaveJournal<G> {
        self.journal
            .as_mut()
            .expect("dense state journal is not recursively borrowed")
    }

    pub(crate) fn checkpoint_cursor(&self) -> DenseStateCursor {
        DenseStateCursor {
            font_runtime_rows: u32::try_from(self.font_runtime.cursor())
                .expect("font runtime rows fit u32"),
        }
    }

    pub(crate) fn validate_checkpoint_cursor(&self, cursor: DenseStateCursor) -> bool {
        cursor.font_runtime_rows as usize <= self.font_runtime.cursor()
    }

    pub(crate) fn restore_checkpoint_cursor(&mut self, cursor: DenseStateCursor) {
        assert!(self.validate_checkpoint_cursor(cursor));
        self.font_runtime
            .truncate(cursor.font_runtime_rows as usize);
    }

    pub(crate) fn visit_dynamic_memory_roots(&self, mut visit: impl FnMut(DynamicMemoryRoot<G>)) {
        for meaning in self.meanings.values() {
            if let MeaningWord::Macro { definition, .. } = meaning {
                visit(DynamicMemoryRoot::Definition(definition));
            }
        }
        for index in u16::MIN..=u16::MAX {
            if let Some(tokens) = self.token_registers.get(index).expect("u16 register").value {
                visit(DynamicMemoryRoot::TokenList(tokens));
            }
        }
        for index in 0..PARAMETER_COUNT as u16 {
            if let Some(tokens) = self
                .token_parameters
                .get(u32::from(index))
                .expect("parameter")
                .value
            {
                visit(DynamicMemoryRoot::TokenList(tokens));
            }
        }
    }

    pub(crate) fn capture_format_font_runtime(
        &self,
        font: FontId,
    ) -> Result<crate::format::schema::FormatFontRuntime, &'static str> {
        self.font_runtime
            .capture_format(font.raw())
            .map_err(|_| "format font runtime is not live")
    }

    pub(crate) fn install_format_font_runtimes(
        &mut self,
        fonts: &[crate::format::schema::FormatFontRuntime],
    ) -> Result<(), &'static str> {
        let mut runtime = FontRuntimeBank::new();
        for (raw, font) in fonts.iter().enumerate() {
            runtime
                .install_format(raw as u32, font)
                .map_err(|_| "invalid format font runtime")?;
        }
        self.font_runtime = runtime;
        Ok(())
    }

    pub(crate) fn capture_format_cells(
        &self,
        mut definition_row: impl FnMut(crate::DefinitionRef<G>) -> Result<u32, String>,
        _node_row: impl FnMut(DurableNodeMetadata) -> Result<u32, String>,
    ) -> Result<Vec<crate::format::schema::FormatCell>, String> {
        use crate::format::schema::{FormatCell, FormatMeaning};

        let mut cells = Vec::new();
        for (index, meaning) in self.meanings.values().enumerate() {
            let meaning = match meaning {
                MeaningWord::Static(0) => continue,
                MeaningWord::Static(word) => FormatMeaning::Static(word),
                MeaningWord::Font(font) => FormatMeaning::Font(font.raw()),
                MeaningWord::Macro { flags, definition } => FormatMeaning::Macro {
                    flags: flags.bits(),
                    definition: definition_row(definition)?,
                },
            };
            cells.push(FormatCell::Meaning(index as u32, meaning));
        }
        for index in u16::MIN..=u16::MAX {
            let count = self.counts.get(index).expect("u16 register").value;
            if count != 0 {
                cells.push(FormatCell::Count(index, count));
            }
            let dimension = self
                .dimensions
                .get(index)
                .expect("u16 register")
                .value
                .raw();
            if dimension != 0 {
                cells.push(FormatCell::Dimension(index, dimension));
            }
            if let Some(tokens) = self.token_registers.get(index).expect("u16 register").value {
                cells.push(FormatCell::TokenRegister(index, tokens.format_index()));
            }
            if let Some(glue) = self.glue_registers.get(index).expect("u16 register").value {
                cells.push(FormatCell::GlueRegister(index, glue.format_index()));
            }
            if let Some(glue) = self
                .mu_glue_registers
                .get(index)
                .expect("u16 register")
                .value
            {
                cells.push(FormatCell::MuGlueRegister(index, glue.format_index()));
            }
        }
        for index in 0..PARAMETER_COUNT as u16 {
            let integer = self
                .integer_parameters
                .get(u32::from(index))
                .expect("parameter")
                .value;
            if integer != 0
                && !matches!(
                    IntParam::new(index),
                    IntParam::TIME | IntParam::DAY | IntParam::MONTH | IntParam::YEAR
                )
            {
                cells.push(FormatCell::IntegerParameter(index, integer));
            }
            let dimension = self
                .dimension_parameters
                .get(u32::from(index))
                .expect("parameter")
                .value
                .raw();
            if dimension != 0 {
                cells.push(FormatCell::DimensionParameter(index, dimension));
            }
            if let Some(tokens) = self
                .token_parameters
                .get(u32::from(index))
                .expect("parameter")
                .value
            {
                cells.push(FormatCell::TokenParameter(index, tokens.format_index()));
            }
            if let Some(glue) = self
                .glue_parameters
                .get(u32::from(index))
                .expect("parameter")
                .value
            {
                cells.push(FormatCell::GlueParameter(index, glue.format_index()));
            }
        }
        if self.current_font.value.raw() != 0 {
            cells.push(FormatCell::CurrentFont(self.current_font.value.raw()));
        }
        for (index, font) in self.math_family_fonts.values().enumerate() {
            if font.raw() != 0 {
                cells.push(FormatCell::MathFamilyFont(index as u8, font.raw()));
            }
        }
        for (kind, bank) in [
            (0, &self.catcodes),
            (1, &self.lccodes),
            (2, &self.uccodes),
            (3, &self.sfcodes),
            (4, &self.mathcodes),
            (5, &self.delcodes),
        ] {
            cells.extend(
                bank.nondefault_values()
                    .map(|(scalar, value)| FormatCell::Code {
                        kind,
                        scalar,
                        value,
                    }),
            );
        }
        Ok(cells)
    }

    pub(crate) fn install_format_cells(
        &mut self,
        cells: &[crate::format::schema::FormatCell],
        definitions: &[Option<crate::DefinitionRef<G>>],
        token_lists: &[crate::TokenListId<G>],
        glue_values: &[crate::GlueId<G>],
        _node_lists: &[crate::page_node_arena::PageListId],
        fonts: &[FontId],
    ) -> Result<(), &'static str> {
        use crate::format::schema::{FormatCell, FormatMeaning};
        for &cell in cells {
            match cell {
                FormatCell::Meaning(index, meaning) => {
                    let meaning = match meaning {
                        FormatMeaning::Static(word) => MeaningWord::Static(word),
                        FormatMeaning::Font(font) => MeaningWord::Font(
                            *fonts
                                .get(font as usize)
                                .ok_or("format references an unloaded font")?,
                        ),
                        FormatMeaning::Macro { flags, definition } => MeaningWord::Macro {
                            flags: crate::meaning::MeaningFlags::from_bits(flags),
                            definition: definitions
                                .get(definition as usize)
                                .cloned()
                                .flatten()
                                .ok_or("format macro reference is not live")?,
                        },
                    };
                    self.meanings
                        .write(index, BankCell::level_one(meaning))
                        .map_err(|_| "format meaning index is out of range")?;
                }
                FormatCell::Count(index, value) => self
                    .counts
                    .write(index, BankCell::level_one(value))
                    .map_err(|_| "format count index")?,
                FormatCell::Dimension(index, value) => self
                    .dimensions
                    .write(index, BankCell::level_one(Scaled::from_raw(value)))
                    .map_err(|_| "format dimension index")?,
                FormatCell::TokenRegister(index, value) => self
                    .token_registers
                    .write(
                        index,
                        BankCell::level_one(Some(
                            token_lists
                                .get(value as usize)
                                .ok_or("format token reference is out of range")?
                                .clone(),
                        )),
                    )
                    .map_err(|_| "format token register index")?,
                FormatCell::GlueRegister(index, value) => self
                    .glue_registers
                    .write(
                        index,
                        BankCell::level_one(Some(
                            *glue_values
                                .get(value as usize)
                                .ok_or("format glue reference is out of range")?,
                        )),
                    )
                    .map_err(|_| "format glue register index")?,
                FormatCell::MuGlueRegister(index, value) => self
                    .mu_glue_registers
                    .write(
                        index,
                        BankCell::level_one(Some(
                            *glue_values
                                .get(value as usize)
                                .ok_or("format mu-glue reference is out of range")?,
                        )),
                    )
                    .map_err(|_| "format mu-glue register index")?,
                FormatCell::BoxRegister(_, _) => {
                    // Move-only node owners are installed by the aggregate
                    // format restorer after metadata cells are admitted.
                }
                FormatCell::IntegerParameter(index, value) => self
                    .integer_parameters
                    .write(u32::from(index), BankCell::level_one(value))
                    .map_err(|_| "format integer parameter index")?,
                FormatCell::DimensionParameter(index, value) => self
                    .dimension_parameters
                    .write(
                        u32::from(index),
                        BankCell::level_one(Scaled::from_raw(value)),
                    )
                    .map_err(|_| "format dimension parameter index")?,
                FormatCell::TokenParameter(index, value) => self
                    .token_parameters
                    .write(
                        u32::from(index),
                        BankCell::level_one(Some(
                            token_lists
                                .get(value as usize)
                                .ok_or("format token reference is out of range")?
                                .clone(),
                        )),
                    )
                    .map_err(|_| "format token parameter index")?,
                FormatCell::GlueParameter(index, value) => self
                    .glue_parameters
                    .write(
                        u32::from(index),
                        BankCell::level_one(Some(
                            *glue_values
                                .get(value as usize)
                                .ok_or("format glue reference is out of range")?,
                        )),
                    )
                    .map_err(|_| "format glue parameter index")?,
                FormatCell::CurrentFont(font) => {
                    self.current_font = BankCell::level_one(
                        *fonts
                            .get(font as usize)
                            .ok_or("format references an unloaded font")?,
                    );
                }
                FormatCell::MathFamilyFont(index, font) => self
                    .math_family_fonts
                    .write(
                        u32::from(index),
                        BankCell::level_one(
                            *fonts
                                .get(font as usize)
                                .ok_or("format references an unloaded font")?,
                        ),
                    )
                    .map_err(|_| "format math-family index")?,
                FormatCell::Code {
                    kind,
                    scalar,
                    value,
                } => {
                    let kind = match kind {
                        0 => CodeTableKind::Catcode,
                        1 => CodeTableKind::Lccode,
                        2 => CodeTableKind::Uccode,
                        3 => CodeTableKind::Sfcode,
                        4 => CodeTableKind::Mathcode,
                        5 => CodeTableKind::Delcode,
                        _ => return Err("unknown format code-table kind"),
                    };
                    self.code_bank_mut(kind)
                        .write(scalar, BankCell::level_one(value))
                        .map_err(|_| "format code-table scalar")?;
                }
            }
        }
        Ok(())
    }
    pub(crate) fn new() -> Result<Self, StateError> {
        Ok(Self {
            meanings: DenseBank::growing(MeaningWord::UNDEFINED),
            counts: RegisterBank::new(zero_i32)?,
            dimensions: RegisterBank::new(zero_scaled)?,
            token_registers: RegisterBank::new(no_token_list::<G>)?,
            glue_registers: RegisterBank::new(no_glue::<G>)?,
            mu_glue_registers: RegisterBank::new(no_glue::<G>)?,
            integer_parameters: DenseBank::fixed(PARAMETER_COUNT, 0, LEVEL_ONE)?,
            dimension_parameters: DenseBank::fixed(
                PARAMETER_COUNT,
                Scaled::from_raw(0),
                LEVEL_ONE,
            )?,
            token_parameters: DenseBank::fixed(PARAMETER_COUNT, None, LEVEL_ONE)?,
            glue_parameters: DenseBank::fixed(PARAMETER_COUNT, None, LEVEL_ONE)?,
            current_font: BankCell::level_one(FontId::new(0)),
            math_family_fonts: DenseBank::fixed(MATH_FAMILY_FONT_COUNT, FontId::new(0), LEVEL_ONE)?,
            catcodes: PagedDenseBank::new(UNICODE_SCALAR_COUNT, catcode_default, LEVEL_ONE)?,
            lccodes: PagedDenseBank::new(UNICODE_SCALAR_COUNT, lccode_default, LEVEL_ONE)?,
            uccodes: PagedDenseBank::new(UNICODE_SCALAR_COUNT, uccode_default, LEVEL_ONE)?,
            sfcodes: PagedDenseBank::new(UNICODE_SCALAR_COUNT, sfcode_default, LEVEL_ONE)?,
            mathcodes: PagedDenseBank::new(UNICODE_SCALAR_COUNT, mathcode_default, LEVEL_ONE)?,
            delcodes: PagedDenseBank::new(UNICODE_SCALAR_COUNT, delcode_default, LEVEL_ONE)?,
            font_runtime: FontRuntimeBank::new(),
            fresh_parameter_profiles: 0,
            journal: Some(SaveJournal::new()),
            groups: Vec::new(),
            next_group_lineage: 1,
            reachable_state_identity_enabled: false,
        })
    }

    pub(crate) fn enable_reachable_state_identity(&mut self) -> bool {
        if self.reachable_state_identity_enabled {
            return true;
        }
        if self.fresh_parameter_profiles != 0 || !self.groups.is_empty() || self.journal_len() != 0
        {
            return false;
        }
        self.reachable_state_identity_enabled = true;
        true
    }

    pub(crate) fn reachable_state_identity_root(
        &self,
        mut definition_identity: impl FnMut(crate::DefinitionRef<G>) -> Option<u64>,
    ) -> Option<u64> {
        if !self.reachable_state_identity_enabled {
            return None;
        }
        let mut root = crate::state_hash::SemanticMapIdentity::empty(0x636f_7265_5f65_6e76);
        let mut complete = true;
        let mut include = |cell, word: StateWord<G>| match state_word_semantic_contribution(
            cell,
            &word,
            &mut definition_identity,
        ) {
            Ok(Some(value)) => {
                root.replace(state_cell_semantic_key(cell), None, Some(value));
            }
            Ok(None) => {}
            Err(()) => complete = false,
        };
        for (index, meaning) in self.meanings.values().enumerate() {
            include(
                StateCell::Meaning(index as u32),
                StateWord::Meaning(meaning),
            );
        }
        for index in u16::MIN..=u16::MAX {
            include(
                StateCell::Count(index),
                StateWord::Integer(self.counts.get(index).expect("u16 register").value),
            );
            include(
                StateCell::Dimension(index),
                StateWord::Dimension(self.dimensions.get(index).expect("u16 register").value),
            );
            include(
                StateCell::TokenRegister(index),
                StateWord::TokenList(
                    self.token_registers
                        .get(index)
                        .expect("u16 register")
                        .value
                        .clone(),
                ),
            );
            include(
                StateCell::GlueRegister(index),
                StateWord::Glue(self.glue_registers.get(index).expect("u16 register").value),
            );
            include(
                StateCell::MuGlueRegister(index),
                StateWord::Glue(
                    self.mu_glue_registers
                        .get(index)
                        .expect("u16 register")
                        .value,
                ),
            );
        }
        for index in 0..PARAMETER_COUNT as u16 {
            include(
                StateCell::IntegerParameter(index),
                StateWord::Integer(
                    self.integer_parameters
                        .get(u32::from(index))
                        .expect("parameter")
                        .value,
                ),
            );
            include(
                StateCell::DimensionParameter(index),
                StateWord::Dimension(
                    self.dimension_parameters
                        .get(u32::from(index))
                        .expect("parameter")
                        .value,
                ),
            );
            include(
                StateCell::TokenParameter(index),
                StateWord::TokenList(
                    self.token_parameters
                        .get(u32::from(index))
                        .expect("parameter")
                        .value
                        .clone(),
                ),
            );
            include(
                StateCell::GlueParameter(index),
                StateWord::Glue(
                    self.glue_parameters
                        .get(u32::from(index))
                        .expect("parameter")
                        .value,
                ),
            );
        }
        include(
            StateCell::CurrentFont,
            StateWord::Font(self.current_font.value),
        );
        for (index, font) in self.math_family_fonts.values().enumerate() {
            include(
                StateCell::MathFamilyFont(index as u8),
                StateWord::Font(font),
            );
        }
        for (kind, bank) in [
            (CodeTableKind::Catcode, &self.catcodes),
            (CodeTableKind::Lccode, &self.lccodes),
            (CodeTableKind::Uccode, &self.uccodes),
            (CodeTableKind::Sfcode, &self.sfcodes),
            (CodeTableKind::Mathcode, &self.mathcodes),
            (CodeTableKind::Delcode, &self.delcodes),
        ] {
            for (scalar, value) in bank.nondefault_values() {
                include(StateCell::Code(kind, scalar), StateWord::Code(value));
            }
        }
        self.font_runtime.visit_cells(|cell, value| match value {
            BankCellValue::Integer(value) => include(
                StateCell::FontRuntime(cell),
                StateWord::Integer(value.value),
            ),
            BankCellValue::Dimension(value) => include(
                StateCell::FontRuntime(cell),
                StateWord::Dimension(value.value),
            ),
        });
        complete.then(|| root.root())
    }

    /// Installs one complete fresh profile layer without creating TeX
    /// assignment history. All cells are validated before the first write.
    pub(crate) fn install_fresh_parameter_profile(
        &mut self,
        profile: FreshParameterProfile,
        defaults: &[FreshParameterDefault],
    ) -> Result<FreshParameterInstallation, FreshParameterInstallError> {
        let bit = profile.bit();
        if self.fresh_parameter_profiles & bit != 0 {
            return Ok(FreshParameterInstallation::AlreadyInstalled);
        }
        if profile != FreshParameterProfile::Tex82
            && self.fresh_parameter_profiles & FreshParameterProfile::Tex82.bit() == 0
        {
            return Err(FreshParameterInstallError::MissingTex82Base(profile));
        }

        let mut seen = [[false; PARAMETER_COUNT]; 4];
        for &default in defaults {
            let (bank, index) = default.cell();
            let seen = &mut seen[bank.slot()][usize::from(index)];
            if *seen {
                return Err(FreshParameterInstallError::DuplicateCell { bank, index });
            }
            *seen = true;
        }

        for &default in defaults {
            match default {
                FreshParameterDefault::Integer(parameter, value) => self
                    .write_cell(
                        StateCell::IntegerParameter(parameter.raw()),
                        BankCell::level_one(StateWord::Integer(value)),
                    )
                    .expect("typed integer parameter fits the fixed bank"),
                FreshParameterDefault::Dimension(parameter, value) => self
                    .write_cell(
                        StateCell::DimensionParameter(parameter.raw()),
                        BankCell::level_one(StateWord::Dimension(value)),
                    )
                    .expect("typed dimension parameter fits the fixed bank"),
                FreshParameterDefault::EmptyGlue(parameter) => self
                    .write_cell(
                        StateCell::GlueParameter(parameter.raw()),
                        BankCell::level_one(StateWord::Glue(None)),
                    )
                    .expect("typed glue parameter fits the fixed bank"),
                FreshParameterDefault::EmptyTokens(parameter) => self
                    .write_cell(
                        StateCell::TokenParameter(parameter.raw()),
                        BankCell::level_one(StateWord::TokenList(None)),
                    )
                    .expect("typed token parameter fits the fixed bank"),
            }
        }
        self.fresh_parameter_profiles |= bit;
        Ok(FreshParameterInstallation::Installed)
    }

    /// Applies tex.web §241's volatile job clock outside TeX assignment
    /// history. This is valid for both fresh and restored jobs.
    pub(crate) fn refresh_job_clock(&mut self, clock: JobClock) {
        crate::world::install_job_clock_params(
            &mut |parameter, value| {
                self.write_cell(
                    StateCell::IntegerParameter(parameter.raw()),
                    BankCell::level_one(StateWord::Integer(value)),
                )
                .expect("job-clock parameter fits the fixed bank");
            },
            clock,
        );
    }

    /// Admits a session-validated symbol into the direct meaning bank.
    pub(crate) fn admit_symbol(&mut self, symbol: Symbol) -> Result<(), StateError> {
        self.meanings.admit_through(symbol.raw())?;
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn meaning(&self, symbol: Symbol) -> Result<ResolvedMeaning<G>, StateError> {
        Ok(self.meaning_word(symbol)?.resolve())
    }

    /// Borrows one admitted meaning row without acquiring its semantic owner.
    #[inline(always)]
    pub(crate) fn meaning_word(&self, symbol: Symbol) -> Result<&MeaningWord<G>, StateError> {
        Ok(&self.meanings.get_ref(symbol.raw())?.value)
    }

    pub(crate) fn assign_meaning(
        &mut self,
        symbol: Symbol,
        value: MeaningWord<G>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        let index = symbol.raw();
        let before = self.meanings.get_ref(index)?;
        let before_value = before.value;
        let before_level = before.level;
        let before_save_serial = before.save_serial;
        let current_level = self.current_level();
        let after_level = match scope {
            AssignmentScope::Global => LEVEL_ONE,
            AssignmentScope::Local => current_level,
        };
        let saved_at = (scope == AssignmentScope::Local
            && current_level != LEVEL_ONE
            && before_level != current_level)
            .then_some(current_level);
        let save_serial = self.journal().save_serial();
        let needs_mutation = self.journal().needs_mutation(before_save_serial, saved_at);
        *self.meanings.get_mut(index)? = BankCell::new(value, after_level, save_serial);
        if needs_mutation {
            self.journal_mut().record_mutation(Mutation::new(
                StateCell::Meaning(index),
                StateWord::Meaning(before_value),
                before_level,
                before_save_serial,
                saved_at,
            ));
        }
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn count(&self, index: u16) -> Result<i32, StateError> {
        Ok(self.counts.get(index)?.value)
    }

    pub(crate) fn assign_count(
        &mut self,
        index: u16,
        value: i32,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(StateCell::Count(index), StateWord::Integer(value), scope)
    }

    #[inline(always)]
    pub(crate) fn dimension(&self, index: u16) -> Result<Scaled, StateError> {
        Ok(self.dimensions.get(index)?.value)
    }

    #[inline(always)]
    pub(crate) fn integer_parameter(&self, parameter: IntParam) -> Result<i32, StateError> {
        Ok(self
            .integer_parameters
            .get(u32::from(parameter.raw()))?
            .value)
    }

    pub(crate) fn assign_integer_parameter(
        &mut self,
        parameter: IntParam,
        value: i32,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::IntegerParameter(parameter.raw()),
            StateWord::Integer(value),
            scope,
        )
    }

    pub(crate) fn dimension_parameter(
        &self,
        parameter: banks::DimenParam,
    ) -> Result<Scaled, StateError> {
        Ok(self
            .dimension_parameters
            .get(u32::from(parameter.raw()))?
            .value)
    }

    pub(crate) fn assign_dimension_parameter(
        &mut self,
        parameter: banks::DimenParam,
        value: Scaled,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::DimensionParameter(parameter.raw()),
            StateWord::Dimension(value),
            scope,
        )
    }

    pub(crate) fn glue_parameter(
        &self,
        parameter: banks::GlueParam,
    ) -> Result<Option<GlueId<G>>, StateError> {
        Ok(self.glue_parameters.get(u32::from(parameter.raw()))?.value)
    }

    pub(crate) fn assign_glue_parameter(
        &mut self,
        parameter: banks::GlueParam,
        value: Option<GlueId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::GlueParameter(parameter.raw()),
            StateWord::Glue(value),
            scope,
        )
    }

    pub(crate) fn assign_mu_glue_register(
        &mut self,
        index: u16,
        value: Option<GlueId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::MuGlueRegister(index),
            StateWord::Glue(value),
            scope,
        )
    }

    pub(crate) fn mu_glue_register(&self, index: u16) -> Result<Option<GlueId<G>>, StateError> {
        Ok(self.mu_glue_registers.get(index)?.value)
    }

    pub(crate) const fn current_font(&self) -> FontId {
        self.current_font.value
    }

    pub(crate) fn math_family_font(&self, index: u8) -> Result<FontId, StateError> {
        Ok(self.math_family_fonts.get(u32::from(index))?.value)
    }

    pub(crate) fn assign_dimension(
        &mut self,
        index: u16,
        value: Scaled,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::Dimension(index),
            StateWord::Dimension(value),
            scope,
        )
    }

    #[inline(always)]
    pub(crate) fn token_register(&self, index: u16) -> Result<Option<TokenListId<G>>, StateError> {
        Ok(self.token_registers.get(index)?.value)
    }

    #[inline(always)]
    pub(crate) fn token_parameter(
        &self,
        parameter: banks::TokParam,
    ) -> Result<Option<TokenListId<G>>, StateError> {
        Ok(self.token_parameters.get(u32::from(parameter.raw()))?.value)
    }

    pub(crate) fn assign_token_register(
        &mut self,
        index: u16,
        value: Option<TokenListId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::TokenRegister(index),
            StateWord::TokenList(value),
            scope,
        )
    }

    pub(crate) fn assign_token_parameter(
        &mut self,
        parameter: banks::TokParam,
        value: Option<TokenListId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::TokenParameter(parameter.raw()),
            StateWord::TokenList(value),
            scope,
        )
    }

    pub(crate) fn assign_current_font(
        &mut self,
        value: FontId,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(StateCell::CurrentFont, StateWord::Font(value), scope)
    }

    pub(crate) fn assign_math_family_font(
        &mut self,
        index: u8,
        value: FontId,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::MathFamilyFont(index),
            StateWord::Font(value),
            scope,
        )
    }

    pub(crate) fn prepare_font_runtime(
        &mut self,
        parameters: &[Scaled],
        hyphen_char: i32,
        skew_char: i32,
    ) -> Result<PreparedFontRuntime, StateError> {
        Ok(self
            .font_runtime
            .prepare(parameters, hyphen_char, skew_char)?)
    }

    pub(crate) fn install_font_runtime(
        &mut self,
        font: FontId,
        prepared: PreparedFontRuntime,
    ) -> Result<(), StateError> {
        self.font_runtime.install(font.raw(), prepared)?;
        Ok(())
    }

    pub(crate) fn prepare_derived_font_runtime(
        &mut self,
        request: DerivedFontRuntimeRequest<'_, FontId>,
    ) -> Result<PreparedFontRuntime, StateError> {
        Ok(self
            .font_runtime
            .prepare_derived(DerivedFontRuntimeRequest {
                source: request.source.raw(),
                parameters: request.parameters,
                preserve_character_settings: request.preserve_character_settings,
                preserve_pdf_settings: request.preserve_pdf_settings,
                disable_ligatures: request.disable_ligatures,
                default_hyphen_char: request.default_hyphen_char,
                default_skew_char: request.default_skew_char,
            })?)
    }

    pub(crate) fn font_parameter_count(&self, font: FontId) -> Result<u32, StateError> {
        Ok(self.font_runtime.parameter_count(font.raw())?)
    }

    pub(crate) fn font_parameter_words(&self) -> usize {
        self.font_runtime.parameter_words()
    }

    pub(crate) fn hash_font_runtime(
        &self,
        font: FontId,
        loaded: &tex_fonts::LoadedFont,
        hasher: &mut crate::state_hash::StateHasher,
    ) -> Result<(), StateError> {
        self.font_runtime
            .hash_semantic(font.raw(), loaded, hasher)?;
        Ok(())
    }

    pub(crate) fn font_dimen(&self, font: FontId, number: u32) -> Result<Scaled, StateError> {
        let StateWord::Dimension(value) = self
            .read_cell(StateCell::FontRuntime(FontRuntimeCell::Dimen {
                font: font.raw(),
                number,
            }))?
            .value
        else {
            return Err(StateError::CellKindMismatch);
        };
        Ok(value)
    }

    pub(crate) fn assign_font_dimen(
        &mut self,
        font: FontId,
        number: u32,
        value: Scaled,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        let previous_count = self.font_runtime.parameter_count(font.raw())?;
        self.font_runtime.prepare_dimen_growth(font.raw(), number)?;
        if number > previous_count {
            self.assign(
                StateCell::FontRuntime(FontRuntimeCell::ParameterCount(font.raw())),
                StateWord::Integer(
                    i32::try_from(number).map_err(|_| StateError::CellKindMismatch)?,
                ),
                scope,
            )?;
        }
        self.assign(
            StateCell::FontRuntime(FontRuntimeCell::Dimen {
                font: font.raw(),
                number,
            }),
            StateWord::Dimension(value),
            scope,
        )
    }

    pub(crate) fn font_hyphen_char(&self, font: FontId) -> Result<i32, StateError> {
        self.font_runtime_integer(FontRuntimeCell::HyphenChar(font.raw()))
    }

    pub(crate) fn assign_font_hyphen_char(
        &mut self,
        font: FontId,
        value: i32,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::FontRuntime(FontRuntimeCell::HyphenChar(font.raw())),
            StateWord::Integer(value),
            scope,
        )
    }

    pub(crate) fn font_skew_char(&self, font: FontId) -> Result<i32, StateError> {
        self.font_runtime_integer(FontRuntimeCell::SkewChar(font.raw()))
    }

    pub(crate) fn assign_font_skew_char(
        &mut self,
        font: FontId,
        value: i32,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::FontRuntime(FontRuntimeCell::SkewChar(font.raw())),
            StateWord::Integer(value),
            scope,
        )
    }

    pub(crate) fn prepare_pdf_font_code_table(
        &mut self,
        font: FontId,
        table: crate::font::PdfFontCode,
        defaults: [i32; 256],
    ) -> Result<(), StateError> {
        self.font_runtime
            .ensure_pdf_table(font.raw(), table, defaults)?;
        Ok(())
    }

    pub(crate) fn pdf_font_code(
        &self,
        font: FontId,
        table: crate::font::PdfFontCode,
        code: u8,
    ) -> Result<i32, StateError> {
        self.font_runtime_integer(font_runtime::table_cell(table, font.raw(), code))
    }

    pub(crate) fn assign_pdf_font_code(
        &mut self,
        font: FontId,
        table: crate::font::PdfFontCode,
        code: u8,
        value: i32,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::FontRuntime(font_runtime::table_cell(table, font.raw(), code)),
            StateWord::Integer(value),
            scope,
        )
    }

    pub(crate) fn pdf_font_ligatures_disabled(&self, font: FontId) -> Result<bool, StateError> {
        Ok(self.font_runtime_integer(FontRuntimeCell::LigaturesDisabled(font.raw()))? != 0)
    }

    pub(crate) fn assign_pdf_font_ligatures_disabled(
        &mut self,
        font: FontId,
        disabled: bool,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::FontRuntime(FontRuntimeCell::LigaturesDisabled(font.raw())),
            StateWord::Integer(i32::from(disabled)),
            scope,
        )
    }

    fn font_runtime_integer(&self, cell: FontRuntimeCell) -> Result<i32, StateError> {
        let StateWord::Integer(value) = self.read_cell(StateCell::FontRuntime(cell))?.value else {
            return Err(StateError::CellKindMismatch);
        };
        Ok(value)
    }

    #[inline(always)]
    pub(crate) fn glue_register(&self, index: u16) -> Result<Option<GlueId<G>>, StateError> {
        Ok(self.glue_registers.get(index)?.value)
    }

    pub(crate) fn assign_glue_register(
        &mut self,
        index: u16,
        value: Option<GlueId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::GlueRegister(index),
            StateWord::Glue(value),
            scope,
        )
    }

    #[inline(always)]
    pub(crate) fn code(&self, kind: CodeTableKind, scalar: char) -> Result<i64, StateError> {
        Ok(self.code_bank(kind).get(scalar as u32)?.value)
    }

    pub(crate) fn assign_code(
        &mut self,
        kind: CodeTableKind,
        scalar: char,
        value: i64,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::Code(kind, scalar as u32),
            StateWord::Code(value),
            scope,
        )
    }

    #[must_use]
    pub(crate) fn journal_cursor(&mut self) -> JournalCursor<G> {
        let group_depth = self.groups.len();
        self.journal_mut().checkpoint_cursor(group_depth)
    }

    #[must_use]
    pub(crate) fn group_depth(&self) -> usize {
        self.groups.len()
    }

    #[must_use]
    pub(crate) fn group_frames(&self) -> &[GroupFrame] {
        &self.groups
    }

    /// Returns the state-journal coordinate used to order a command-owned
    /// `\aftergroup` push against state-owned save records.
    #[must_use]
    pub(crate) fn save_stack_order_position(&self) -> u32 {
        u32::try_from(self.journal().len()).expect("state journal exceeds u32 entries")
    }

    /// TeX82 §§273/275's depth immediately before the newest checked
    /// push, merged with command-owned `\aftergroup` words.
    #[must_use]
    pub(crate) fn checked_save_stack_words(
        &self,
        aftergroup_words: usize,
        latest_aftergroup_position: Option<u32>,
        save_group_source_lines: bool,
    ) -> usize {
        let (state_words, latest_state_push) = self.journal().save_stack_projection();
        let latest_push_words = match (latest_state_push, latest_aftergroup_position) {
            (Some((state_position, _)), Some(aftergroup_position))
                if aftergroup_position >= state_position =>
            {
                1
            }
            (Some((_, words)), _) => words,
            (None, Some(_)) => 1,
            (None, None) => 0,
        };
        state_words
            .saturating_add(aftergroup_words)
            // e-TeX [19.274] stores one source-line word immediately before
            // every §273 boundary, so it contributes to the checked depth.
            .saturating_add(if save_group_source_lines {
                self.groups.len()
            } else {
                0
            })
            .saturating_sub(latest_push_words)
    }

    pub(crate) fn begin_group(
        &mut self,
        kind: GroupKind,
        entered_line: u32,
    ) -> Result<GroupFrame, StateError> {
        let depth =
            u32::try_from(self.groups.len()).map_err(|_| StateError::GroupDepthExhausted)?;
        let level = depth
            .checked_add(2)
            .ok_or(StateError::GroupDepthExhausted)?;
        let lineage = self.next_group_lineage;
        self.next_group_lineage = lineage
            .checked_add(1)
            .ok_or(StateError::GroupLineageExhausted)?;
        let journal_start =
            u32::try_from(self.journal().len() + 1).map_err(|_| StateError::GroupDepthExhausted)?;
        let (save_stack_words_before, latest_save_push_before) =
            self.journal().save_stack_projection();
        let frame = GroupFrame::new(
            kind,
            entered_line,
            lineage,
            journal_start,
            level,
            save_stack_words_before,
            latest_save_push_before,
        );
        self.journal_mut().record_group_enter(frame);
        self.groups.push(frame);
        Ok(frame)
    }

    /// Performs TeX82 §283 restoration in exact save-stack order, including
    /// e-TeX [53a]'s single sparse-array restore marker.
    pub(crate) fn end_group(
        &mut self,
        expected: GroupKind,
    ) -> Result<GroupRestorationReceipt<G>, StateError> {
        let frame = *self
            .groups
            .last()
            .ok_or(StateError::GroupMismatch(GroupMismatch::no_group(expected)))?;
        if frame.kind() != expected {
            return Err(StateError::GroupMismatch(GroupMismatch::new(
                expected,
                frame.kind(),
            )));
        }

        let end = self.journal().len();
        let restoration_count = (frame.journal_start as usize..end)
            .filter(|&index| {
                matches!(
                    self.journal().entry(index),
                    JournalEntry::Mutation(saved) if saved.saved_at() == Some(frame.level)
                )
            })
            .count();
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(restoration_count)
            .map_err(|_| StateError::Bank(BankError::AllocationFailed))?;
        let start = frame.journal_start as usize;
        let first_extended = (start..end).find(|&index| {
            matches!(
                self.journal().entry(index),
                JournalEntry::Mutation(saved)
                    if saved.saved_at() == Some(frame.level)
                        && is_extended_register_cell(saved.cell())
            )
        });
        if let Some(marker) = first_extended {
            // e-TeX [53a] pushes one `restore_sa` marker at the first sparse
            // save. Dense eqtb saves made later sit above that marker and
            // restore first; reaching the marker restores the whole sparse
            // chain in reverse sparse-save order; older dense saves follow.
            for index in (marker..end).rev() {
                let JournalEntry::Mutation(saved) = self.journal().entry(index) else {
                    continue;
                };
                if saved.saved_at() == Some(frame.level) && !is_extended_register_cell(saved.cell())
                {
                    self.restore_group_mutation(saved, &mut entries)?;
                }
            }
            for index in (marker..end).rev() {
                let JournalEntry::Mutation(saved) = self.journal().entry(index) else {
                    continue;
                };
                if saved.saved_at() == Some(frame.level) && is_extended_register_cell(saved.cell())
                {
                    self.restore_group_mutation(saved, &mut entries)?;
                }
            }
            for index in (start..marker).rev() {
                let JournalEntry::Mutation(saved) = self.journal().entry(index) else {
                    continue;
                };
                if saved.saved_at() == Some(frame.level) {
                    self.restore_group_mutation(saved, &mut entries)?;
                }
            }
        } else {
            for index in (start..end).rev() {
                let JournalEntry::Mutation(saved) = self.journal().entry(index) else {
                    continue;
                };
                if saved.saved_at() == Some(frame.level) {
                    self.restore_group_mutation(saved, &mut entries)?;
                }
            }
        }
        self.groups.pop();
        self.journal_mut().record_group_exit(frame);
        Ok(GroupRestorationReceipt { frame, entries })
    }

    fn restore_group_mutation(
        &mut self,
        saved: Mutation<G>,
        entries: &mut Vec<GroupRestorationEntry<G>>,
    ) -> Result<(), StateError> {
        let cell = saved.cell();
        let current = self.read_cell(cell)?;
        // A global definition made after this save suppresses it exactly
        // as TeX's `level_one` check in `unsave` does.
        let (live, outcome) = if current.level == LEVEL_ONE {
            (current.value, GroupRestorationOutcome::Retained)
        } else {
            self.write_cell(
                cell,
                BankCell::new(
                    saved.before.clone(),
                    saved.before_level,
                    self.journal().save_serial(),
                ),
            )?;
            self.journal_mut().record_mutation(Mutation::new(
                cell,
                current.value,
                current.level,
                current.save_serial,
                None,
            ));
            (saved.before.clone(), GroupRestorationOutcome::Restored)
        };
        entries.push(GroupRestorationEntry {
            cell: restoration_cell(cell),
            saved: restoration_value(saved.before),
            live: restoration_value(live),
            outcome,
            trace: self.group_restoration_trace_state()?,
        });
        Ok(())
    }

    pub(crate) fn group_restoration_trace_state(
        &self,
    ) -> Result<GroupRestorationTraceState, StateError> {
        Ok(GroupRestorationTraceState {
            tracing_restores: self.integer_parameter(IntParam::TRACING_RESTORES)?,
            tracing_online: self.integer_parameter(IntParam::TRACING_ONLINE)?,
            newline_char: self.integer_parameter(IntParam::NEWLINE_CHAR)?,
            escape_char: self.integer_parameter(IntParam::ESCAPE_CHAR)?,
        })
    }

    /// Atomically restores all banks and open-group state to `cursor`.
    pub(crate) fn restore(&mut self, cursor: JournalCursor<G>) -> Result<(), StateError> {
        self.validate_restore(cursor)?;
        let mut journal = self
            .journal
            .take()
            .expect("dense restore exclusively borrows its journal");
        journal.visit_checkpoint_suffix_mut_reverse(cursor, |delta| {
            self.swap_checkpoint_delta(delta)
                .expect("validated dense checkpoint value swaps in place");
        });
        match journal.restore_group_cursor(cursor) {
            RestoredGroups::Truncate(len) => self.groups.truncate(len),
            RestoredGroups::Replace(groups) => self.groups = groups,
        }
        journal.truncate_checkpoint(cursor);
        self.journal = Some(journal);
        Ok(())
    }

    /// Inverse-rewinds the accepted values and opens an empty candidate
    /// journal at `cursor` without cloning the dense banks.
    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        cursor: JournalCursor<G>,
        dense_cursor: DenseStateCursor,
    ) -> Result<AcceptedDenseStateTail<G>, StateError> {
        self.validate_restore(cursor)?;
        if !self.validate_checkpoint_cursor(dense_cursor) {
            return Err(StateError::InvalidCursor);
        }
        let mut journal = self
            .journal
            .take()
            .expect("dense fork exclusively borrows its journal");
        journal.visit_checkpoint_suffix_mut_reverse(cursor, |delta| {
            self.swap_checkpoint_delta(delta)
                .expect("validated accepted dense value swaps in place");
        });
        let journal_tail = journal.begin_checkpoint_candidate(cursor);
        let accepted_groups = if journal_tail.is_root_candidate() {
            Vec::new()
        } else {
            std::mem::take(&mut self.groups)
        };
        let accepted_next_group_lineage = self.next_group_lineage;
        if !journal_tail.is_root_candidate() {
            self.groups = journal.active_group_frames().collect::<Vec<GroupFrame>>();
        }
        let font_runtime = self
            .font_runtime
            .begin_checkpoint_candidate(dense_cursor.font_runtime_rows as usize);
        self.journal = Some(journal);
        Ok(AcceptedDenseStateTail {
            journal: journal_tail,
            font_runtime,
            groups: accepted_groups,
            next_group_lineage: accepted_next_group_lineage,
        })
    }

    /// Undoes only candidate mutations, then forward-redoes the saved
    /// accepted delta and restores the accepted group stack.
    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        cursor: JournalCursor<G>,
        dense_cursor: DenseStateCursor,
        tail: AcceptedDenseStateTail<G>,
    ) -> Result<(), StateError> {
        let mut journal = self
            .journal
            .take()
            .expect("dense rejection exclusively borrows its journal");
        journal.visit_current_suffix_mut_reverse(cursor, |delta| {
            self.swap_checkpoint_delta(delta)
                .expect("validated candidate dense value swaps in place");
        });
        self.font_runtime.reject_checkpoint_candidate(
            dense_cursor.font_runtime_rows as usize,
            tail.font_runtime,
        );
        journal.visit_detached_suffix_mut(|delta| {
            self.swap_checkpoint_delta(delta)
                .expect("validated accepted dense value redoes in place");
        });
        journal.reject_checkpoint_candidate(tail.journal);
        self.groups = tail.groups;
        self.next_group_lineage = tail.next_group_lineage;
        self.journal = Some(journal);
        Ok(())
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self, tail: AcceptedDenseStateTail<G>) {
        self.journal_mut().accept_checkpoint_candidate();
        drop(tail);
    }

    pub(crate) fn begin_state_transaction(&mut self) -> StateOperation<G> {
        let group_depth = self.groups.len();
        let save_stack = self.journal().current_save_stack();
        let position = self.journal_mut().begin_transaction();
        StateOperation::transaction(position, group_depth, save_stack)
    }

    pub(crate) fn commit_state_transaction(&mut self, position: usize) {
        self.journal_mut().commit_transaction(position);
    }

    pub(crate) fn rollback_state_transaction(
        &mut self,
        operation: &StateOperation<G>,
    ) -> Result<(), StateError> {
        let position = operation.transaction_position();
        let suffix_len = self.journal().transaction_len();
        for index in (position..suffix_len).rev() {
            let mutation = self
                .journal()
                .transaction_entry(index)
                .expect("transaction suffix length remains stable");
            self.write_cell(
                mutation.cell(),
                BankCell::new(
                    mutation.before,
                    mutation.before_level,
                    mutation.before_save_serial,
                ),
            )?;
        }
        if !self
            .journal_mut()
            .rollback_group_suffix(operation.group_depth(), operation.save_stack())
        {
            return Err(StateError::InvalidCursor);
        }
        self.groups.truncate(operation.group_depth());
        self.journal_mut().finish_transaction_rollback(position);
        Ok(())
    }

    #[must_use]
    pub(crate) fn journal_len(&self) -> usize {
        self.journal().retained_len()
    }

    #[must_use]
    pub(crate) fn journal_retained_bytes(&self) -> usize {
        self.journal().retained_bytes()
    }

    #[must_use]
    pub(crate) fn allocated_overflow_pages(&self) -> usize {
        self.counts.allocated_overflow_pages()
            + self.dimensions.allocated_overflow_pages()
            + self.token_registers.allocated_overflow_pages()
            + self.glue_registers.allocated_overflow_pages()
            + self.mu_glue_registers.allocated_overflow_pages()
            + self.font_runtime.allocated_pages()
    }

    fn assign(
        &mut self,
        cell: StateCell,
        value: StateWord<G>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        let before = self.read_cell(cell)?;
        if !word_matches(cell, &value) {
            return Err(StateError::CellKindMismatch);
        }
        let current_level = self.current_level();
        let after_level = match scope {
            AssignmentScope::Global => LEVEL_ONE,
            AssignmentScope::Local => current_level,
        };
        let saved_at = (scope == AssignmentScope::Local
            && current_level != LEVEL_ONE
            && before.level != current_level)
            .then_some(current_level);
        let needs_mutation = self.journal().needs_mutation(before.save_serial, saved_at);
        self.write_cell(
            cell,
            BankCell::new(value, after_level, self.journal().save_serial()),
        )?;
        if needs_mutation {
            self.journal_mut().record_mutation(Mutation::new(
                cell,
                before.value,
                before.level,
                before.save_serial,
                saved_at,
            ));
        }
        Ok(())
    }

    pub(crate) fn current_level(&self) -> u32 {
        self.groups.last().map_or(LEVEL_ONE, |frame| frame.level)
    }

    fn read_cell(&self, cell: StateCell) -> Result<BankCell<StateWord<G>>, StateError> {
        let value = match cell {
            StateCell::Meaning(index) => self.meanings.get(index)?.map(StateWord::Meaning),
            StateCell::Count(index) => self.counts.get(index)?.map(StateWord::Integer),
            StateCell::Dimension(index) => self.dimensions.get(index)?.map(StateWord::Dimension),
            StateCell::TokenRegister(index) => {
                self.token_registers.get(index)?.map(StateWord::TokenList)
            }
            StateCell::GlueRegister(index) => self.glue_registers.get(index)?.map(StateWord::Glue),
            StateCell::BoxRegister(_) => return Err(StateError::CellKindMismatch),
            StateCell::MuGlueRegister(index) => {
                self.mu_glue_registers.get(index)?.map(StateWord::Glue)
            }
            StateCell::IntegerParameter(index) => self
                .integer_parameters
                .get(u32::from(index))?
                .map(StateWord::Integer),
            StateCell::DimensionParameter(index) => self
                .dimension_parameters
                .get(u32::from(index))?
                .map(StateWord::Dimension),
            StateCell::TokenParameter(index) => self
                .token_parameters
                .get(u32::from(index))?
                .map(StateWord::TokenList),
            StateCell::GlueParameter(index) => self
                .glue_parameters
                .get(u32::from(index))?
                .map(StateWord::Glue),
            StateCell::CurrentFont => self.current_font.clone().map(StateWord::Font),
            StateCell::MathFamilyFont(index) => self
                .math_family_fonts
                .get(u32::from(index))?
                .map(StateWord::Font),
            StateCell::Code(kind, index) => self.code_bank(kind).get(index)?.map(StateWord::Code),
            StateCell::FontRuntime(cell) => match self.font_runtime.read(cell)? {
                BankCellValue::Integer(value) => value.map(StateWord::Integer),
                BankCellValue::Dimension(value) => value.map(StateWord::Dimension),
            },
        };
        Ok(value)
    }

    fn write_cell(
        &mut self,
        cell: StateCell,
        value: BankCell<StateWord<G>>,
    ) -> Result<(), StateError> {
        let BankCell {
            value,
            level,
            save_serial,
        } = value;
        match (cell, value) {
            (StateCell::Meaning(index), StateWord::Meaning(word)) => self
                .meanings
                .write(index, BankCell::new(word, level, save_serial))?,
            (StateCell::Count(index), StateWord::Integer(word)) => self
                .counts
                .write(index, BankCell::new(word, level, save_serial))?,
            (StateCell::Dimension(index), StateWord::Dimension(word)) => self
                .dimensions
                .write(index, BankCell::new(word, level, save_serial))?,
            (StateCell::TokenRegister(index), StateWord::TokenList(word)) => {
                self.token_registers
                    .write(index, BankCell::new(word, level, save_serial))?
            }
            (StateCell::GlueRegister(index), StateWord::Glue(word)) => self
                .glue_registers
                .write(index, BankCell::new(word, level, save_serial))?,
            (StateCell::MuGlueRegister(index), StateWord::Glue(word)) => self
                .mu_glue_registers
                .write(index, BankCell::new(word, level, save_serial))?,
            (StateCell::IntegerParameter(index), StateWord::Integer(word)) => self
                .integer_parameters
                .write(u32::from(index), BankCell::new(word, level, save_serial))?,
            (StateCell::DimensionParameter(index), StateWord::Dimension(word)) => self
                .dimension_parameters
                .write(u32::from(index), BankCell::new(word, level, save_serial))?,
            (StateCell::TokenParameter(index), StateWord::TokenList(word)) => self
                .token_parameters
                .write(u32::from(index), BankCell::new(word, level, save_serial))?,
            (StateCell::GlueParameter(index), StateWord::Glue(word)) => self
                .glue_parameters
                .write(u32::from(index), BankCell::new(word, level, save_serial))?,
            (StateCell::CurrentFont, StateWord::Font(word)) => {
                self.current_font = BankCell::new(word, level, save_serial)
            }
            (StateCell::MathFamilyFont(index), StateWord::Font(word)) => self
                .math_family_fonts
                .write(u32::from(index), BankCell::new(word, level, save_serial))?,
            (StateCell::Code(kind, index), StateWord::Code(word)) => self
                .code_bank_mut(kind)
                .write(index, BankCell::new(word, level, save_serial))?,
            (StateCell::FontRuntime(cell), StateWord::Integer(word)) => self.font_runtime.write(
                cell,
                BankCellValue::Integer(BankCell::new(word, level, save_serial)),
            )?,
            (StateCell::FontRuntime(cell), StateWord::Dimension(word)) => self.font_runtime.write(
                cell,
                BankCellValue::Dimension(BankCell::new(word, level, save_serial)),
            )?,
            _ => return Err(StateError::CellKindMismatch),
        }
        Ok(())
    }

    fn swap_checkpoint_delta(
        &mut self,
        delta: &mut crate::journal::CheckpointDelta<G>,
    ) -> Result<(), StateError> {
        fn swap_cell<T>(
            target: &mut BankCell<T>,
            value: &mut T,
            level: &mut u32,
            save_serial: &mut u64,
        ) {
            std::mem::swap(&mut target.value, value);
            std::mem::swap(&mut target.level, level);
            std::mem::swap(&mut target.save_serial, save_serial);
        }

        match (delta.cell, &mut delta.alternate) {
            (StateCell::Meaning(index), StateWord::Meaning(value)) => swap_cell(
                self.meanings.get_mut(index)?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::Count(index), StateWord::Integer(value)) => swap_cell(
                self.counts.get_mut(index)?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::Dimension(index), StateWord::Dimension(value)) => swap_cell(
                self.dimensions.get_mut(index)?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::TokenRegister(index), StateWord::TokenList(value)) => swap_cell(
                self.token_registers.get_mut(index)?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::GlueRegister(index), StateWord::Glue(value)) => swap_cell(
                self.glue_registers.get_mut(index)?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::MuGlueRegister(index), StateWord::Glue(value)) => swap_cell(
                self.mu_glue_registers.get_mut(index)?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::IntegerParameter(index), StateWord::Integer(value)) => swap_cell(
                self.integer_parameters.get_mut(u32::from(index))?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::DimensionParameter(index), StateWord::Dimension(value)) => swap_cell(
                self.dimension_parameters.get_mut(u32::from(index))?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::TokenParameter(index), StateWord::TokenList(value)) => swap_cell(
                self.token_parameters.get_mut(u32::from(index))?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::GlueParameter(index), StateWord::Glue(value)) => swap_cell(
                self.glue_parameters.get_mut(u32::from(index))?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::CurrentFont, StateWord::Font(value)) => {
                swap_cell(
                    &mut self.current_font,
                    value,
                    &mut delta.alternate_level,
                    &mut delta.alternate_save_serial,
                );
            }
            (StateCell::MathFamilyFont(index), StateWord::Font(value)) => swap_cell(
                self.math_family_fonts.get_mut(u32::from(index))?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::Code(kind, index), StateWord::Code(value)) => swap_cell(
                self.code_bank_mut(kind).get_mut(index)?,
                value,
                &mut delta.alternate_level,
                &mut delta.alternate_save_serial,
            ),
            (StateCell::FontRuntime(cell), StateWord::Integer(value)) => {
                self.font_runtime.swap_integer(
                    cell,
                    value,
                    &mut delta.alternate_level,
                    &mut delta.alternate_save_serial,
                )?
            }
            (StateCell::FontRuntime(cell), StateWord::Dimension(value)) => {
                self.font_runtime.swap_dimension(
                    cell,
                    value,
                    &mut delta.alternate_level,
                    &mut delta.alternate_save_serial,
                )?
            }
            _ => return Err(StateError::CellKindMismatch),
        }
        Ok(())
    }

    pub(crate) fn validate_restore(&self, cursor: JournalCursor<G>) -> Result<(), StateError> {
        if !self.journal().validate_cursor(cursor) {
            return Err(StateError::InvalidCursor);
        }
        let mut words_match = true;
        self.journal().visit_checkpoint_suffix(cursor, |delta| {
            words_match &= word_matches(delta.cell, &delta.alternate);
        });
        if !words_match {
            return Err(StateError::CellKindMismatch);
        }
        Ok(())
    }

    pub(crate) fn release_checkpoint_prefix(
        &mut self,
        cursor: JournalCursor<G>,
    ) -> Result<usize, StateError> {
        self.journal_mut().release_checkpoint_prefix(cursor)
    }

    fn code_bank(&self, kind: CodeTableKind) -> &PagedDenseBank<i64> {
        match kind {
            CodeTableKind::Catcode => &self.catcodes,
            CodeTableKind::Lccode => &self.lccodes,
            CodeTableKind::Uccode => &self.uccodes,
            CodeTableKind::Sfcode => &self.sfcodes,
            CodeTableKind::Mathcode => &self.mathcodes,
            CodeTableKind::Delcode => &self.delcodes,
        }
    }

    fn code_bank_mut(&mut self, kind: CodeTableKind) -> &mut PagedDenseBank<i64> {
        match kind {
            CodeTableKind::Catcode => &mut self.catcodes,
            CodeTableKind::Lccode => &mut self.lccodes,
            CodeTableKind::Uccode => &mut self.uccodes,
            CodeTableKind::Sfcode => &mut self.sfcodes,
            CodeTableKind::Mathcode => &mut self.mathcodes,
            CodeTableKind::Delcode => &mut self.delcodes,
        }
    }
}

#[derive(Clone)]
pub(crate) enum DynamicMemoryRoot<G> {
    Definition(crate::DefinitionRef<G>),
    TokenList(crate::TokenListId<G>),
}

impl<T> BankCell<T> {
    fn map<U>(self, map: impl FnOnce(T) -> U) -> BankCell<U> {
        BankCell::new(map(self.value), self.level, self.save_serial)
    }
}

fn restoration_cell(cell: StateCell) -> GroupRestorationCell {
    match cell {
        StateCell::Meaning(index) => GroupRestorationCell::Meaning(Symbol::from_packed_slot(index)),
        StateCell::Count(index) => GroupRestorationCell::Count(index),
        StateCell::Dimension(index) => GroupRestorationCell::Dimension(index),
        StateCell::TokenRegister(index) => GroupRestorationCell::TokenRegister(index),
        StateCell::GlueRegister(index) => GroupRestorationCell::GlueRegister(index),
        StateCell::BoxRegister(index) => GroupRestorationCell::BoxRegister(index),
        StateCell::MuGlueRegister(index) => GroupRestorationCell::MuGlueRegister(index),
        StateCell::IntegerParameter(index) => GroupRestorationCell::IntegerParameter(index),
        StateCell::DimensionParameter(index) => GroupRestorationCell::DimensionParameter(index),
        StateCell::TokenParameter(index) => GroupRestorationCell::TokenParameter(index),
        StateCell::GlueParameter(index) => GroupRestorationCell::GlueParameter(index),
        StateCell::CurrentFont => GroupRestorationCell::CurrentFont,
        StateCell::MathFamilyFont(index) => GroupRestorationCell::MathFamilyFont(index),
        StateCell::Code(kind, index) => GroupRestorationCell::Code(kind, index),
        StateCell::FontRuntime(cell) => GroupRestorationCell::FontRuntime(match cell {
            FontRuntimeCell::ParameterCount(font) => {
                GroupRestorationFontRuntimeCell::ParameterCount(font)
            }
            FontRuntimeCell::Dimen { font, number } => {
                GroupRestorationFontRuntimeCell::Dimension { font, number }
            }
            FontRuntimeCell::HyphenChar(font) => GroupRestorationFontRuntimeCell::HyphenChar(font),
            FontRuntimeCell::SkewChar(font) => GroupRestorationFontRuntimeCell::SkewChar(font),
            FontRuntimeCell::PdfCode { table, font, code } => {
                GroupRestorationFontRuntimeCell::PdfCode { table, font, code }
            }
            FontRuntimeCell::LigaturesDisabled(font) => {
                GroupRestorationFontRuntimeCell::LigaturesDisabled(font)
            }
        }),
    }
}

fn state_cell_semantic_key(cell: StateCell) -> u64 {
    crate::state_hash::semantic_scalar_root(0x636f_7265_5f6b_6579, |hasher| match cell {
        StateCell::Meaning(index) => {
            hasher.u8(0);
            hasher.u32(index);
        }
        StateCell::Count(index) => {
            hasher.u8(1);
            hasher.u16(index);
        }
        StateCell::Dimension(index) => {
            hasher.u8(2);
            hasher.u16(index);
        }
        StateCell::TokenRegister(index) => {
            hasher.u8(3);
            hasher.u16(index);
        }
        StateCell::GlueRegister(index) => {
            hasher.u8(4);
            hasher.u16(index);
        }
        StateCell::BoxRegister(index) => {
            hasher.u8(5);
            hasher.u16(index);
        }
        StateCell::MuGlueRegister(index) => {
            hasher.u8(6);
            hasher.u16(index);
        }
        StateCell::IntegerParameter(index) => {
            hasher.u8(7);
            hasher.u16(index);
        }
        StateCell::DimensionParameter(index) => {
            hasher.u8(8);
            hasher.u16(index);
        }
        StateCell::TokenParameter(index) => {
            hasher.u8(9);
            hasher.u16(index);
        }
        StateCell::GlueParameter(index) => {
            hasher.u8(10);
            hasher.u16(index);
        }
        StateCell::CurrentFont => hasher.u8(11),
        StateCell::MathFamilyFont(index) => {
            hasher.u8(12);
            hasher.u8(index);
        }
        StateCell::Code(kind, scalar) => {
            hasher.u8(13 + kind as u8);
            hasher.u32(scalar);
        }
        StateCell::FontRuntime(cell) => {
            hasher.u8(19);
            hasher.u64(font_runtime_cell_semantic_key(cell));
        }
    })
}

fn font_runtime_cell_semantic_key(cell: FontRuntimeCell) -> u64 {
    crate::state_hash::semantic_scalar_root(0x666f_6e74_5f72_746b, |hasher| match cell {
        FontRuntimeCell::ParameterCount(font) => {
            hasher.u8(0);
            hasher.u32(font);
        }
        FontRuntimeCell::Dimen { font, number } => {
            hasher.u8(1);
            hasher.u32(font);
            hasher.u32(number);
        }
        FontRuntimeCell::HyphenChar(font) => {
            hasher.u8(2);
            hasher.u32(font);
        }
        FontRuntimeCell::SkewChar(font) => {
            hasher.u8(3);
            hasher.u32(font);
        }
        FontRuntimeCell::PdfCode { table, font, code } => {
            hasher.u8(4);
            hasher.u8(table);
            hasher.u32(font);
            hasher.u8(code);
        }
        FontRuntimeCell::LigaturesDisabled(font) => {
            hasher.u8(5);
            hasher.u32(font);
        }
    })
}

fn state_word_semantic_contribution<G>(
    cell: StateCell,
    word: &StateWord<G>,
    definition_identity: &mut impl FnMut(crate::DefinitionRef<G>) -> Option<u64>,
) -> Result<Option<u64>, ()> {
    let is_default = match (cell, word) {
        (StateCell::Meaning(_), StateWord::Meaning(value)) => value == &MeaningWord::UNDEFINED,
        (StateCell::Count(_) | StateCell::IntegerParameter(_), StateWord::Integer(value)) => {
            *value == 0
        }
        (
            StateCell::Dimension(_) | StateCell::DimensionParameter(_),
            StateWord::Dimension(value),
        ) => value.raw() == 0,
        (
            StateCell::TokenRegister(_) | StateCell::TokenParameter(_),
            StateWord::TokenList(value),
        ) => value.is_none(),
        (
            StateCell::GlueRegister(_) | StateCell::MuGlueRegister(_) | StateCell::GlueParameter(_),
            StateWord::Glue(value),
        ) => value.is_none(),
        (StateCell::CurrentFont | StateCell::MathFamilyFont(_), StateWord::Font(value)) => {
            value.raw() == 0
        }
        (StateCell::Code(kind, scalar), StateWord::Code(value)) => {
            let expected = match kind {
                CodeTableKind::Catcode => catcode_default(scalar),
                CodeTableKind::Lccode => lccode_default(scalar),
                CodeTableKind::Uccode => uccode_default(scalar),
                CodeTableKind::Sfcode => sfcode_default(scalar),
                CodeTableKind::Mathcode => mathcode_default(scalar),
                CodeTableKind::Delcode => delcode_default(scalar),
            };
            *value == expected
        }
        _ => false,
    };
    if is_default {
        return Ok(None);
    }
    let identity = match word {
        StateWord::Meaning(value) => {
            let definition_identity = match value {
                MeaningWord::Macro { definition, .. } => definition_identity(*definition),
                MeaningWord::Static(_) | MeaningWord::Font(_) => None,
            };
            value.semantic_identity(definition_identity).ok_or(())?
        }
        StateWord::Integer(value) => {
            crate::state_hash::semantic_scalar_root(0x636f_7265_5f69_6e74, |hasher| {
                hasher.i32(*value)
            })
        }
        StateWord::Dimension(value) => {
            crate::state_hash::semantic_scalar_root(0x636f_7265_5f64_696d, |hasher| {
                hasher.i32(value.raw())
            })
        }
        StateWord::TokenList(Some(value)) => value.semantic_identity().ok_or(())?,
        StateWord::Glue(Some(value)) => value.semantic_identity().ok_or(())?,
        StateWord::Font(value) => {
            crate::state_hash::semantic_scalar_root(0x636f_7265_5f66_6e74, |hasher| {
                hasher.u32(value.raw())
            })
        }
        StateWord::Code(value) => {
            crate::state_hash::semantic_scalar_root(0x636f_7265_5f63_6f64, |hasher| {
                hasher.u64(*value as u64)
            })
        }
        StateWord::TokenList(None) | StateWord::Glue(None) => {
            unreachable!("default optional values returned above")
        }
    };
    Ok(Some(identity))
}

fn is_extended_register_cell(cell: StateCell) -> bool {
    match cell {
        StateCell::Count(index)
        | StateCell::Dimension(index)
        | StateCell::TokenRegister(index)
        | StateCell::GlueRegister(index)
        | StateCell::BoxRegister(index)
        | StateCell::MuGlueRegister(index) => index > u8::MAX.into(),
        _ => false,
    }
}

fn restoration_value<G>(word: StateWord<G>) -> GroupRestorationValue<G> {
    match word {
        StateWord::Meaning(value) => GroupRestorationValue::Meaning(value.resolve()),
        StateWord::Integer(value) => GroupRestorationValue::Integer(value),
        StateWord::Dimension(value) => GroupRestorationValue::Dimension(value),
        StateWord::TokenList(value) => GroupRestorationValue::TokenList(value),
        StateWord::Glue(value) => GroupRestorationValue::Glue(value),
        StateWord::Font(value) => GroupRestorationValue::Font(value),
        StateWord::Code(value) => GroupRestorationValue::Code(value),
    }
}

fn word_matches<G>(cell: StateCell, word: &StateWord<G>) -> bool {
    matches!(
        (cell, word),
        (StateCell::Meaning(_), StateWord::Meaning(_))
            | (StateCell::Count(_), StateWord::Integer(_))
            | (StateCell::Dimension(_), StateWord::Dimension(_))
            | (StateCell::TokenRegister(_), StateWord::TokenList(_))
            | (StateCell::GlueRegister(_), StateWord::Glue(_))
            | (StateCell::MuGlueRegister(_), StateWord::Glue(_))
            | (StateCell::IntegerParameter(_), StateWord::Integer(_))
            | (StateCell::DimensionParameter(_), StateWord::Dimension(_))
            | (StateCell::TokenParameter(_), StateWord::TokenList(_))
            | (StateCell::GlueParameter(_), StateWord::Glue(_))
            | (StateCell::CurrentFont, StateWord::Font(_))
            | (StateCell::MathFamilyFont(_), StateWord::Font(_))
            | (StateCell::Code(_, _), StateWord::Code(_))
            | (StateCell::FontRuntime(_), StateWord::Integer(_))
            | (StateCell::FontRuntime(_), StateWord::Dimension(_))
    )
}

fn catcode_default(code: u32) -> i64 {
    match code {
        0 => 9,
        13 => 5,
        32 => 10,
        37 => 14,
        92 => 0,
        127 => 15,
        65..=90 | 97..=122 => 11,
        _ => 12,
    }
}

fn lccode_default(code: u32) -> i64 {
    match code {
        65..=90 => i64::from(code + 32),
        97..=122 => i64::from(code),
        _ => 0,
    }
}

fn uccode_default(code: u32) -> i64 {
    match code {
        65..=90 => i64::from(code),
        97..=122 => i64::from(code - 32),
        _ => 0,
    }
}

fn sfcode_default(code: u32) -> i64 {
    if (65..=90).contains(&code) { 999 } else { 1000 }
}

fn mathcode_default(code: u32) -> i64 {
    let value = match code {
        48..=57 => (7 << 12) | code,
        65..=90 | 97..=122 => (7 << 12) | (1 << 8) | code,
        _ => code,
    };
    i64::from(value)
}

fn delcode_default(code: u32) -> i64 {
    if code == u32::from(b'.') { 0 } else { -1 }
}

fn zero_i32(_: u32) -> i32 {
    0
}

fn zero_scaled(_: u32) -> Scaled {
    Scaled::from_raw(0)
}

fn no_token_list<G>(_: u32) -> Option<TokenListId<G>> {
    None
}

fn no_glue<G>(_: u32) -> Option<GlueId<G>> {
    None
}
