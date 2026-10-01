//! Immutable, generation-branded macro-definition storage.
//!
//! Every published definition is one immutable row whose words live in a
//! shared `Rc<[TokenWord]>` span. Format and global rows are append-only and
//! rewind only to checkpoint cursors. Local rows form one LIFO stack whose
//! group marks truncate it at group exit. An executing macro retains its own
//! span handle, so retiring a row never invalidates a body being read and no
//! region lease, slot incarnation, or retirement queue is needed.

use core::marker::PhantomData;
use core::num::NonZeroU32;
use std::rc::Rc;

use crate::generation::ArenaToken;
use crate::macro_definition::{MacroParameterPattern, MacroParameterPatternBuilder};
use crate::memory_accounting::MemoryAccounting;
use crate::token::TokenWord;

#[path = "definition_arena/body.rs"]
mod body;
#[path = "definition_arena/build_lane.rs"]
mod build_lane;
#[path = "definition_arena/builder.rs"]
mod builder;
#[cfg(test)]
#[path = "definition_arena/tests.rs"]
mod tests;

use body::DefinitionEntry;
#[cfg(any(test, feature = "profiling", feature = "testing"))]
pub use body::definition_retain_count;
pub use body::{
    AdmittedMacroDefinition, DefinitionRef, DefinitionView, DefinitionWords, ResidentMacroBody,
};
use build_lane::ActiveDefinitionBuild;
pub use build_lane::{DefinitionBuildKey, DefinitionBuildWriter};
pub use builder::{DefinitionBuildError, DefinitionBuildPhase, DefinitionBuilder};

pub(super) enum DefinitionNamespace {}

const FORMAT_REGION: u32 = 1;
const GLOBAL_REGION: u32 = 2;
const LOCAL_REGION: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DefinitionArenaCursor {
    format_rows: u32,
    global_rows: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionDestination {
    Format,
    Global,
    Local,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionAllocationError {
    CapacityOverflow,
    AllocationFailed,
    InvalidDefinition,
}

/// Format and global rows detached by a checkpoint candidate.
///
/// Rejection appends them back at their original row numbers; acceptance
/// drops them.
pub(crate) struct AcceptedDefinitionTail<G> {
    format_start: usize,
    format: Vec<DefinitionEntry>,
    global_start: usize,
    global: Vec<DefinitionEntry>,
    next_build_serial: u32,
    _brand: PhantomData<fn(&G) -> &G>,
}

/// One promoted local row and its global copy.
#[derive(Clone, Copy)]
struct DefinitionPromotion {
    local_row: u32,
    global_row: u32,
}

/// Revision-owned definition rows.
pub(crate) struct DefinitionArena<G> {
    format: Vec<DefinitionEntry>,
    global: Vec<DefinitionEntry>,
    local: Vec<DefinitionEntry>,
    /// Local row count at entry to each open group.
    local_marks: Vec<u32>,
    /// Memoized local-to-global copies, ordered by local row.
    promotions: Vec<DefinitionPromotion>,
    next_build_serial: u32,
    active_build: Option<ActiveDefinitionBuild>,
    /// Words of the active build. A writer owns this lane while admitted.
    build_words: Vec<TokenWord>,
    active_writer: Option<NonZeroU32>,
    accounting: MemoryAccounting,
    _brand: PhantomData<fn(&G) -> &G>,
}

impl<G> DefinitionArena<G> {
    pub(super) fn new(
        _token: ArenaToken<G, DefinitionNamespace>,
        accounting: MemoryAccounting,
    ) -> Self {
        Self {
            format: Vec::new(),
            global: Vec::new(),
            local: Vec::new(),
            local_marks: Vec::new(),
            promotions: Vec::new(),
            next_build_serial: 1,
            active_build: None,
            build_words: Vec::new(),
            active_writer: None,
            accounting,
            _brand: PhantomData,
        }
    }

    #[inline(always)]
    fn rows(&self, region: u32) -> Option<&Vec<DefinitionEntry>> {
        match region {
            FORMAT_REGION => Some(&self.format),
            GLOBAL_REGION => Some(&self.global),
            LOCAL_REGION => Some(&self.local),
            _ => None,
        }
    }

    #[inline(always)]
    fn entry(&self, id: DefinitionRef<G>) -> Option<&DefinitionEntry> {
        self.rows(id.region())?.get(id.row_index())
    }

    #[inline]
    pub(crate) fn admit_macro_definition(
        &self,
        id: DefinitionRef<G>,
    ) -> Option<AdmittedMacroDefinition<G>> {
        let entry = self.entry(id)?;
        if entry.parameter_len == 0 {
            let body = (!entry.words.is_empty()).then(|| ResidentMacroBody::admit(entry));
            return Some(AdmittedMacroDefinition::SimpleMacro {
                pattern: entry.pattern,
                body,
            });
        }
        Some(AdmittedMacroDefinition::MatchingMacro {
            pattern: entry.pattern,
            parameter_len: entry.parameter_len as usize,
            body: ResidentMacroBody::admit(entry),
        })
    }

    pub(crate) fn admit_macro_body(
        &self,
        id: DefinitionRef<G>,
    ) -> Option<(MacroParameterPattern, usize, ResidentMacroBody<G>)> {
        let entry = self.entry(id)?;
        Some((
            entry.pattern,
            entry.parameter_len as usize,
            ResidentMacroBody::admit(entry),
        ))
    }

    pub(crate) fn contents_equal(&self, left: DefinitionRef<G>, right: DefinitionRef<G>) -> bool {
        if left == right {
            return true;
        }
        match (self.entry(left), self.entry(right)) {
            (Some(left), Some(right)) => left.contents_equal(right),
            _ => false,
        }
    }

    /// Compares a current reference with one from the accepted head whose
    /// row may have been detached into a checkpoint candidate's tail.
    pub(crate) fn current_and_accepted_contents_equal(
        &self,
        accepted: &AcceptedDefinitionTail<G>,
        current: DefinitionRef<G>,
        prior: DefinitionRef<G>,
    ) -> bool {
        let Some(current_entry) = self.entry(current) else {
            return false;
        };
        let row = prior.row_index();
        let detached = match prior.region() {
            FORMAT_REGION => row
                .checked_sub(accepted.format_start)
                .map(|row| accepted.format.get(row)),
            GLOBAL_REGION => row
                .checked_sub(accepted.global_start)
                .map(|row| accepted.global.get(row)),
            _ => None,
        };
        let prior_entry = match detached {
            Some(entry) => entry,
            None if current == prior => return true,
            None => self.entry(prior),
        };
        prior_entry.is_some_and(|prior| current_entry.contents_equal(prior))
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        cursor: DefinitionArenaCursor,
    ) -> AcceptedDefinitionTail<G> {
        assert!(self.validates_cursor(cursor));
        assert!(self.active_build.is_none() && self.active_writer.is_none());
        let format_start = cursor.format_rows as usize;
        let global_start = cursor.global_rows as usize;
        AcceptedDefinitionTail {
            format_start,
            format: self.format.split_off(format_start),
            global_start,
            global: self.global.split_off(global_start),
            next_build_serial: self.next_build_serial,
            _brand: PhantomData,
        }
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        cursor: DefinitionArenaCursor,
        mut tail: AcceptedDefinitionTail<G>,
    ) {
        self.restore_cursor(cursor);
        debug_assert_eq!(self.format.len(), tail.format_start);
        debug_assert_eq!(self.global.len(), tail.global_start);
        self.format.append(&mut tail.format);
        self.global.append(&mut tail.global);
        self.next_build_serial = tail.next_build_serial;
    }

    pub(crate) fn accept_checkpoint_candidate(&self, tail: AcceptedDefinitionTail<G>) {
        self.release_rows(&tail.format);
        self.release_rows(&tail.global);
    }

    /// Captures the format and global frontiers at a checkpoint. Checkpoints
    /// are taken only outside every group, so no local row is live.
    pub(crate) fn cursor(&self) -> DefinitionArenaCursor {
        assert!(self.active_writer.is_none());
        debug_assert!(
            self.local_marks.is_empty(),
            "checkpoints are outside groups"
        );
        DefinitionArenaCursor {
            format_rows: u32::try_from(self.format.len()).expect("definition rows fit u32"),
            global_rows: u32::try_from(self.global.len()).expect("definition rows fit u32"),
        }
    }

    pub(crate) fn validates_cursor(&self, cursor: DefinitionArenaCursor) -> bool {
        cursor.format_rows as usize <= self.format.len()
            && cursor.global_rows as usize <= self.global.len()
    }

    /// Rewinds to a checkpoint frontier. The restored state is outside every
    /// group, so the local stack and any abandoned build are discarded.
    pub(crate) fn restore_cursor(&mut self, cursor: DefinitionArenaCursor) {
        assert!(self.validates_cursor(cursor));
        assert!(self.active_writer.is_none());
        self.abort_active_build();
        self.truncate_format_rows(cursor.format_rows as usize);
        self.truncate_global_rows(cursor.global_rows as usize);
        self.truncate_local_rows(0);
        self.local_marks.clear();
    }

    fn truncate_format_rows(&mut self, len: usize) {
        if len < self.format.len() {
            let detached = self.format.split_off(len);
            self.release_rows(&detached);
        }
    }

    fn truncate_global_rows(&mut self, len: usize) {
        if len < self.global.len() {
            let detached = self.global.split_off(len);
            self.release_rows(&detached);
        }
    }

    fn truncate_local_rows(&mut self, len: usize) {
        if len < self.local.len() {
            let detached = self.local.split_off(len);
            self.release_rows(&detached);
        }
        let retained = self
            .promotions
            .partition_point(|promotion| (promotion.local_row as usize) < len);
        self.promotions.truncate(retained);
    }

    fn release_rows(&self, rows: &[DefinitionEntry]) {
        for row in rows {
            self.accounting
                .release_shared_dynamic(definition_memory_words(row.words.len()));
        }
    }

    pub(crate) fn allocate(
        &mut self,
        parameter_text: &[TokenWord],
        replacement_text: &[TokenWord],
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        self.allocate_from_iter(
            parameter_text.iter().copied(),
            replacement_text.iter().copied(),
        )
    }

    pub(crate) fn allocate_from_iter<Parameters, Replacement>(
        &mut self,
        parameter_text: Parameters,
        replacement_text: Replacement,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError>
    where
        Parameters: ExactSizeIterator<Item = TokenWord>,
        Replacement: ExactSizeIterator<Item = TokenWord>,
    {
        if self.active_build.is_some() || self.active_writer.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        let mut builder = DefinitionBuilder::new();
        for word in parameter_text {
            builder.push_parameter(word).map_err(map_build_error)?;
        }
        builder.finish_parameters().map_err(map_build_error)?;
        for word in replacement_text {
            builder.push_replacement(word).map_err(map_build_error)?;
        }
        builder.seal().map_err(map_build_error)?;
        self.publish(&mut builder)
    }

    pub(crate) fn publish(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        if self.active_build.is_some() || self.active_writer.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        self.validate_builder(builder)?;
        self.reserve_batch(1, builder.words().len())?;
        Ok(self.publish_prevalidated(builder))
    }

    pub(crate) fn publish_prevalidated(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> DefinitionRef<G> {
        let entry = DefinitionEntry {
            words: Rc::from(builder.words()),
            parameter_len: builder.parameter_len,
            origin: crate::token::OriginId::UNKNOWN,
            pattern: builder.pattern.finish(),
        };
        builder.phase = DefinitionBuildPhase::Published;
        self.push_row(GLOBAL_REGION, entry)
    }

    /// Writes one validated wire definition into the immutable format region.
    pub(crate) fn publish_format_row(
        &mut self,
        row: &crate::format::schema::FormatDefinition,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        if self.active_build.is_some() || self.active_writer.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        let parameter_len = u32::try_from(row.parameter_text.len())
            .map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        let mut pattern = MacroParameterPatternBuilder::new();
        for &word in &row.parameter_text {
            pattern
                .push_parameter(TokenWord::from_raw(word))
                .map_err(DefinitionBuildError::from)
                .map_err(map_build_error)?;
        }
        for &word in &row.replacement_text {
            pattern
                .validate_replacement(TokenWord::from_raw(word))
                .map_err(DefinitionBuildError::from)
                .map_err(map_build_error)?;
        }
        let words: Rc<[TokenWord]> = row
            .parameter_text
            .iter()
            .chain(&row.replacement_text)
            .map(|&word| TokenWord::from_raw(word))
            .collect();
        u32::try_from(words.len()).map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        self.format
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        Ok(self.push_row(
            FORMAT_REGION,
            DefinitionEntry {
                words,
                parameter_len,
                origin: crate::token::OriginId::UNKNOWN,
                pattern: pattern.finish(),
            },
        ))
    }

    fn push_row(&mut self, region: u32, entry: DefinitionEntry) -> DefinitionRef<G> {
        self.accounting
            .allocate_shared_dynamic(definition_memory_words(entry.words.len()));
        let rows = match region {
            FORMAT_REGION => &mut self.format,
            GLOBAL_REGION => &mut self.global,
            _ => &mut self.local,
        };
        rows.push(entry);
        DefinitionRef::new(region, rows.len() - 1)
    }

    pub(crate) fn validate_builder(
        &self,
        builder: &DefinitionBuilder,
    ) -> Result<(), DefinitionAllocationError> {
        builder.validate_completed()
    }

    pub(crate) fn reserve_batch(
        &mut self,
        rows: usize,
        _words: usize,
    ) -> Result<(), DefinitionAllocationError> {
        reserve_rows(&mut self.global, rows)
    }

    pub(crate) fn reserve_format_batch(
        &mut self,
        rows: usize,
        _words: usize,
    ) -> Result<(), DefinitionAllocationError> {
        reserve_rows(&mut self.format, rows)
    }

    /// Opens one local scope. No storage is allocated until the group
    /// defines something.
    pub(crate) fn begin_group(&mut self) -> Result<(), DefinitionAllocationError> {
        if self.active_build.is_some() || self.active_writer.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        self.local_marks
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        self.local_marks
            .push(u32::try_from(self.local.len()).expect("definition rows fit u32"));
        Ok(())
    }

    /// Drops the innermost group's local rows. Executing bodies keep their
    /// own word spans alive.
    pub(crate) fn end_group(&mut self) {
        assert!(
            self.active_build.is_none() && self.active_writer.is_none(),
            "definition scan crosses group exit"
        );
        let mark = self
            .local_marks
            .pop()
            .expect("definition group stack matches TeX groups");
        self.truncate_local_rows(mark as usize);
    }

    pub(crate) fn promote_global(
        &mut self,
        id: DefinitionRef<G>,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        if self.active_build.is_some() || self.active_writer.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        if id.region() != LOCAL_REGION {
            return Ok(id);
        }
        let local_row = u32::try_from(id.row_index()).expect("definition rows fit u32");
        let memo = self
            .promotions
            .binary_search_by_key(&local_row, |promotion| promotion.local_row);
        let insert_at = match memo {
            Ok(index) => {
                return Ok(DefinitionRef::new(
                    GLOBAL_REGION,
                    self.promotions[index].global_row as usize,
                ));
            }
            Err(index) => index,
        };
        let entry = self
            .local
            .get(id.row_index())
            .cloned()
            .ok_or(DefinitionAllocationError::InvalidDefinition)?;
        reserve_rows(&mut self.global, 1)?;
        self.promotions
            .try_reserve(1)
            .map_err(|_| DefinitionAllocationError::AllocationFailed)?;
        let promoted = self.push_row(GLOBAL_REGION, entry);
        self.promotions.insert(
            insert_at,
            DefinitionPromotion {
                local_row,
                global_row: u32::try_from(promoted.row_index()).expect("definition rows fit u32"),
            },
        );
        Ok(promoted)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get(&self, id: DefinitionRef<G>) -> DefinitionView<'_, G> {
        DefinitionView {
            entry: self.entry(id).expect("definition row is live"),
            _brand: PhantomData,
        }
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.format.len() + self.global.len() + self.local.len()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<G> Drop for DefinitionArena<G> {
    fn drop(&mut self) {
        self.release_rows(&self.format);
        self.release_rows(&self.global);
        self.release_rows(&self.local);
    }
}

fn reserve_rows(
    rows: &mut Vec<DefinitionEntry>,
    additional: usize,
) -> Result<(), DefinitionAllocationError> {
    rows.len()
        .checked_add(additional)
        .and_then(|len| u32::try_from(len).ok())
        .ok_or(DefinitionAllocationError::CapacityOverflow)?;
    rows.try_reserve(additional)
        .map_err(|_| DefinitionAllocationError::AllocationFailed)
}

fn map_build_error(error: DefinitionBuildError) -> DefinitionAllocationError {
    match error {
        DefinitionBuildError::AllocationFailed => DefinitionAllocationError::AllocationFailed,
        DefinitionBuildError::CapacityOverflow => DefinitionAllocationError::CapacityOverflow,
        DefinitionBuildError::InvalidPhase | DefinitionBuildError::InvalidProgram(_) => {
            DefinitionAllocationError::InvalidDefinition
        }
    }
}

fn definition_memory_words(word_len: usize) -> usize {
    let header_words =
        std::mem::size_of::<DefinitionEntry>().div_ceil(std::mem::size_of::<usize>());
    word_len
        .checked_mul(std::mem::size_of::<TokenWord>())
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<usize>() - 1))
        .map(|bytes| bytes / std::mem::size_of::<usize>())
        .and_then(|words| words.checked_add(header_words))
        .expect("validated definition length has a canonical word count")
}
