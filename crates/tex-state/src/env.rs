//! Generation-branded direct-index TeX state and exact save semantics.

#[path = "env/banks.rs"]
pub mod banks;
#[path = "env/group.rs"]
pub(crate) mod group;

use banks::{
    BankCell, BankError, DenseBank, IntParam, LEVEL_ONE, PARAMETER_COUNT, PagedDenseBank,
    RegisterBank,
};
use group::{GroupFrame, GroupKind, GroupMismatch};

use crate::durable_arena::{GlueId, TokenListId};
use crate::ids::FontId;
use crate::interner::Symbol;
use crate::journal::{JournalCursor, JournalEntry, Mutation, MutationKind, SaveJournal};
use crate::meaning::{MeaningWord, ResolvedMeaning};
use crate::node_arena::DurableListId;
use crate::scaled::Scaled;

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

/// The six eqtb code-table families.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodeTableKind {
    Catcode,
    Lccode,
    Uccode,
    Sfcode,
    Mathcode,
    Delcode,
}

/// Typed identity of one mutable current-value cell.
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
}

/// One packed scalar or typed generation coordinate stored by a bank/journal.
pub(crate) enum StateWord<G> {
    Meaning(MeaningWord<G>),
    Integer(i32),
    Dimension(Scaled),
    TokenList(Option<TokenListId<G>>),
    Glue(Option<GlueId<G>>),
    NodeList(Option<DurableListId<G>>),
    Font(FontId),
    Code(i64),
}

impl<G> Clone for StateWord<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for StateWord<G> {}

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
            Self::NodeList(None) => formatter.write_str("NodeList(None)"),
            Self::NodeList(Some(_)) => formatter.write_str("NodeList(Some(..))"),
            Self::Font(value) => value.fmt(formatter),
            Self::Code(value) => value.fmt(formatter),
        }
    }
}

impl<G> PartialEq for StateWord<G> {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
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
    box_registers: RegisterBank<Option<DurableListId<G>>>,
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
    journal: SaveJournal<G>,
    groups: Vec<GroupFrame>,
    next_group_lineage: u64,
}

impl<G> DenseState<G> {
    pub(crate) fn new() -> Result<Self, StateError> {
        Ok(Self {
            meanings: DenseBank::growing(MeaningWord::UNDEFINED),
            counts: RegisterBank::new(zero_i32)?,
            dimensions: RegisterBank::new(zero_scaled)?,
            token_registers: RegisterBank::new(no_token_list::<G>)?,
            glue_registers: RegisterBank::new(no_glue::<G>)?,
            box_registers: RegisterBank::new(no_node_list::<G>)?,
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
            journal: SaveJournal::new(),
            groups: Vec::new(),
            next_group_lineage: 1,
        })
    }

    /// Admits a session-validated symbol into the direct meaning bank.
    pub(crate) fn admit_symbol(&mut self, symbol: Symbol) -> Result<(), StateError> {
        self.meanings.admit_through(symbol.raw())?;
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn meaning(&self, symbol: Symbol) -> Result<ResolvedMeaning<G>, StateError> {
        Ok(self.meanings.get(symbol.raw())?.value.resolve())
    }

    pub(crate) fn assign_meaning(
        &mut self,
        symbol: Symbol,
        value: MeaningWord<G>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::Meaning(symbol.raw()),
            StateWord::Meaning(value),
            scope,
        )
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
    pub(crate) fn box_register(&self, index: u16) -> Result<Option<DurableListId<G>>, StateError> {
        Ok(self.box_registers.get(index)?.value)
    }

    pub(crate) fn assign_box_register(
        &mut self,
        index: u16,
        value: Option<DurableListId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        self.assign(
            StateCell::BoxRegister(index),
            StateWord::NodeList(value),
            scope,
        )
    }

    /// Replaces a box value without changing its current TeX eq level.
    pub(crate) fn replace_box_register(
        &mut self,
        index: u16,
        value: Option<DurableListId<G>>,
    ) -> Result<(), StateError> {
        let cell = StateCell::BoxRegister(index);
        let before = self.read_cell(cell)?;
        let StateWord::NodeList(old) = before.value else {
            return Err(StateError::CellKindMismatch);
        };
        let after = StateWord::NodeList(value);
        self.write_cell(
            cell,
            BankCell {
                value: after,
                level: before.level,
            },
        )?;
        self.journal.push(JournalEntry::Mutation(Mutation {
            cell,
            before: StateWord::NodeList(old),
            before_level: before.level,
            after,
            after_level: before.level,
            saved_at: None,
            kind: MutationKind::Assignment,
        }));
        Ok(())
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
    pub(crate) fn journal_cursor(&self) -> JournalCursor<G> {
        self.journal.cursor()
    }

    #[must_use]
    pub(crate) fn group_depth(&self) -> usize {
        self.groups.len()
    }

    #[must_use]
    pub(crate) fn group_frames(&self) -> &[GroupFrame] {
        &self.groups
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
            u32::try_from(self.journal.len() + 1).map_err(|_| StateError::GroupDepthExhausted)?;
        let frame = GroupFrame::new(kind, entered_line, lineage, journal_start, level);
        self.journal.push(JournalEntry::GroupEnter(frame));
        self.groups.push(frame);
        Ok(frame)
    }

    /// Performs TeX82 §283 restoration in exact reverse save order.
    pub(crate) fn end_group(&mut self, expected: GroupKind) -> Result<GroupFrame, StateError> {
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

        let end = self.journal.len();
        for index in (frame.journal_start as usize..end).rev() {
            let JournalEntry::Mutation(saved) = self.journal.entry(index) else {
                continue;
            };
            if saved.kind != MutationKind::Assignment || saved.saved_at != Some(frame.level) {
                continue;
            }
            let current = self.read_cell(saved.cell)?;
            // A global definition made after this save suppresses it exactly
            // as TeX's `level_one` check in `unsave` does.
            if current.level == LEVEL_ONE {
                continue;
            }
            self.write_cell(
                saved.cell,
                BankCell {
                    value: saved.before,
                    level: saved.before_level,
                },
            )?;
            self.journal.push(JournalEntry::Mutation(Mutation {
                cell: saved.cell,
                before: current.value,
                before_level: current.level,
                after: saved.before,
                after_level: saved.before_level,
                saved_at: None,
                kind: MutationKind::GroupRestore,
            }));
        }
        self.groups.pop();
        self.journal.push(JournalEntry::GroupExit(frame));
        Ok(frame)
    }

    /// Atomically restores all banks and open-group state to `cursor`.
    pub(crate) fn restore(&mut self, cursor: JournalCursor<G>) -> Result<(), StateError> {
        self.validate_restore(cursor)?;
        let start = cursor.position() as usize;
        for index in (start..self.journal.len()).rev() {
            match self.journal.entry(index) {
                JournalEntry::Mutation(mutation) => self.write_cell(
                    mutation.cell,
                    BankCell {
                        value: mutation.before,
                        level: mutation.before_level,
                    },
                )?,
                JournalEntry::GroupExit(frame) => self.groups.push(frame),
                JournalEntry::GroupEnter(frame) => {
                    let popped = self.groups.pop().expect("restore validation proved group");
                    debug_assert_eq!(popped, frame);
                }
            }
        }
        self.journal.truncate(cursor);
        Ok(())
    }

    #[must_use]
    pub(crate) fn journal_len(&self) -> usize {
        self.journal.len()
    }

    #[must_use]
    pub(crate) fn allocated_overflow_pages(&self) -> usize {
        self.counts.allocated_overflow_pages()
            + self.dimensions.allocated_overflow_pages()
            + self.token_registers.allocated_overflow_pages()
            + self.glue_registers.allocated_overflow_pages()
            + self.box_registers.allocated_overflow_pages()
            + self.mu_glue_registers.allocated_overflow_pages()
    }

    fn assign(
        &mut self,
        cell: StateCell,
        value: StateWord<G>,
        scope: AssignmentScope,
    ) -> Result<(), StateError> {
        let before = self.read_cell(cell)?;
        if !word_matches(cell, value) {
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
        self.write_cell(
            cell,
            BankCell {
                value,
                level: after_level,
            },
        )?;
        self.journal.push(JournalEntry::Mutation(Mutation {
            cell,
            before: before.value,
            before_level: before.level,
            after: value,
            after_level,
            saved_at,
            kind: MutationKind::Assignment,
        }));
        Ok(())
    }

    fn current_level(&self) -> u32 {
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
            StateCell::BoxRegister(index) => {
                self.box_registers.get(index)?.map(StateWord::NodeList)
            }
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
            StateCell::CurrentFont => self.current_font.map(StateWord::Font),
            StateCell::MathFamilyFont(index) => self
                .math_family_fonts
                .get(u32::from(index))?
                .map(StateWord::Font),
            StateCell::Code(kind, index) => self.code_bank(kind).get(index)?.map(StateWord::Code),
        };
        Ok(value)
    }

    fn write_cell(
        &mut self,
        cell: StateCell,
        value: BankCell<StateWord<G>>,
    ) -> Result<(), StateError> {
        match (cell, value.value) {
            (StateCell::Meaning(index), StateWord::Meaning(word)) => {
                self.meanings.write(index, value.map_value(word))?
            }
            (StateCell::Count(index), StateWord::Integer(word)) => {
                self.counts.write(index, value.map_value(word))?
            }
            (StateCell::Dimension(index), StateWord::Dimension(word)) => {
                self.dimensions.write(index, value.map_value(word))?
            }
            (StateCell::TokenRegister(index), StateWord::TokenList(word)) => {
                self.token_registers.write(index, value.map_value(word))?
            }
            (StateCell::GlueRegister(index), StateWord::Glue(word)) => {
                self.glue_registers.write(index, value.map_value(word))?
            }
            (StateCell::BoxRegister(index), StateWord::NodeList(word)) => {
                self.box_registers.write(index, value.map_value(word))?
            }
            (StateCell::MuGlueRegister(index), StateWord::Glue(word)) => {
                self.mu_glue_registers.write(index, value.map_value(word))?
            }
            (StateCell::IntegerParameter(index), StateWord::Integer(word)) => self
                .integer_parameters
                .write(u32::from(index), value.map_value(word))?,
            (StateCell::DimensionParameter(index), StateWord::Dimension(word)) => self
                .dimension_parameters
                .write(u32::from(index), value.map_value(word))?,
            (StateCell::TokenParameter(index), StateWord::TokenList(word)) => self
                .token_parameters
                .write(u32::from(index), value.map_value(word))?,
            (StateCell::GlueParameter(index), StateWord::Glue(word)) => self
                .glue_parameters
                .write(u32::from(index), value.map_value(word))?,
            (StateCell::CurrentFont, StateWord::Font(word)) => {
                self.current_font = value.map_value(word)
            }
            (StateCell::MathFamilyFont(index), StateWord::Font(word)) => self
                .math_family_fonts
                .write(u32::from(index), value.map_value(word))?,
            (StateCell::Code(kind, index), StateWord::Code(word)) => self
                .code_bank_mut(kind)
                .write(index, value.map_value(word))?,
            _ => return Err(StateError::CellKindMismatch),
        }
        Ok(())
    }

    pub(crate) fn validate_restore(&self, cursor: JournalCursor<G>) -> Result<(), StateError> {
        if !self.journal.validate_cursor(cursor) {
            return Err(StateError::InvalidCursor);
        }
        let mut groups = self.groups.clone();
        for entry in self
            .journal
            .suffix(cursor.position() as usize, self.journal.len())
            .iter()
            .rev()
        {
            match *entry {
                JournalEntry::Mutation(mutation) => {
                    if !word_matches(mutation.cell, mutation.before)
                        || !word_matches(mutation.cell, mutation.after)
                    {
                        return Err(StateError::CellKindMismatch);
                    }
                }
                JournalEntry::GroupExit(frame) => groups.push(frame),
                JournalEntry::GroupEnter(frame) => {
                    if groups.pop() != Some(frame) {
                        return Err(StateError::InvalidCursor);
                    }
                }
            }
        }
        Ok(())
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

impl<T: Copy> BankCell<T> {
    fn map<U: Copy>(self, map: impl FnOnce(T) -> U) -> BankCell<U> {
        BankCell {
            value: map(self.value),
            level: self.level,
        }
    }

    fn map_value<U: Copy>(self, value: U) -> BankCell<U> {
        BankCell {
            value,
            level: self.level,
        }
    }
}

fn word_matches<G>(cell: StateCell, word: StateWord<G>) -> bool {
    matches!(
        (cell, word),
        (StateCell::Meaning(_), StateWord::Meaning(_))
            | (StateCell::Count(_), StateWord::Integer(_))
            | (StateCell::Dimension(_), StateWord::Dimension(_))
            | (StateCell::TokenRegister(_), StateWord::TokenList(_))
            | (StateCell::GlueRegister(_), StateWord::Glue(_))
            | (StateCell::BoxRegister(_), StateWord::NodeList(_))
            | (StateCell::MuGlueRegister(_), StateWord::Glue(_))
            | (StateCell::IntegerParameter(_), StateWord::Integer(_))
            | (StateCell::DimensionParameter(_), StateWord::Dimension(_))
            | (StateCell::TokenParameter(_), StateWord::TokenList(_))
            | (StateCell::GlueParameter(_), StateWord::Glue(_))
            | (StateCell::CurrentFont, StateWord::Font(_))
            | (StateCell::MathFamilyFont(_), StateWord::Font(_))
            | (StateCell::Code(_, _), StateWord::Code(_))
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

fn no_node_list<G>(_: u32) -> Option<DurableListId<G>> {
    None
}
