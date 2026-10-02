//! Immutable macro payloads over pinned format rows and stable counted dynamic slots.
//! Scope belongs to meaning/save levels. Dynamic coordinates never rewind or reuse.

use crate::generation::ArenaToken;
use crate::macro_definition::MacroParameterPatternBuilder;
use crate::memory_accounting::MemoryAccounting;
use crate::token::TokenWord;
use core::marker::PhantomData;
use core::num::NonZeroU32;
use std::cell::RefCell;
use std::rc::Rc;

#[path = "definition_arena/body.rs"]
mod body;
#[path = "definition_arena/build_lane.rs"]
mod build_lane;
#[path = "definition_arena/builder.rs"]
mod builder;
#[cfg(test)]
#[path = "definition_arena/ownership_tests.rs"]
mod ownership_tests;
#[cfg(test)]
#[path = "definition_arena/tests.rs"]
mod tests;
use body::DefinitionEntry;
pub use body::{
    AdmittedMacroDefinition, DefinitionRef, DefinitionView, DefinitionWords, ResidentMacroBody,
};
use build_lane::ActiveDefinitionBuild;
pub use build_lane::{DefinitionBuildKey, DefinitionBuildWriter};
pub use builder::{DefinitionBuildError, DefinitionBuildPhase, DefinitionBuilder};
pub(super) enum DefinitionNamespace {}
const FORMAT_REGION: u32 = 1;
const DYNAMIC_REGION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionAllocationError {
    CapacityOverflow,
    AllocationFailed,
    InvalidDefinition,
}

struct DynamicDefinition {
    payload: Option<DefinitionEntry>,
    roots: usize,
}
#[derive(Default)]
struct DefinitionBank {
    format: Vec<DefinitionEntry>,
    dynamic: Vec<DynamicDefinition>,
}
impl DefinitionBank {
    fn entry<G>(&self, id: DefinitionRef<G>) -> Option<&DefinitionEntry> {
        match id.region() {
            FORMAT_REGION => self.format.get(id.row_index()),
            DYNAMIC_REGION => self.dynamic.get(id.row_index())?.payload.as_ref(),
            _ => None,
        }
    }
}

/// Coarse shared bank admission for actual owning positions. Never used on token reads.
#[derive(Clone, Default)]
pub(crate) struct DefinitionRoots(Rc<RefCell<DefinitionBank>>);
impl DefinitionRoots {
    pub(crate) fn validates_meaning<G>(&self, meaning: crate::meaning::MeaningWord<G>) -> bool {
        match meaning {
            crate::meaning::MeaningWord::Macro { definition, .. } => {
                self.0.borrow().entry(definition).is_some()
            }
            _ => true,
        }
    }
    pub(crate) fn acquire<G>(&self, id: DefinitionRef<G>) {
        if id.region() == FORMAT_REGION {
            return;
        }
        let mut bank = self.0.borrow_mut();
        let slot = bank
            .dynamic
            .get_mut(id.row_index())
            .expect("definition slot exists");
        assert!(slot.payload.is_some(), "definition root names a tombstone");
        slot.roots = slot
            .roots
            .checked_add(1)
            .expect("definition root count overflow");
    }
    pub(crate) fn release<G>(&self, id: DefinitionRef<G>) {
        if id.region() == FORMAT_REGION {
            return;
        }
        let payload = {
            let mut bank = self.0.borrow_mut();
            let slot = bank
                .dynamic
                .get_mut(id.row_index())
                .expect("definition slot exists");
            slot.roots = slot
                .roots
                .checked_sub(1)
                .expect("definition root released twice");
            if slot.roots == 0 {
                slot.payload.take()
            } else {
                None
            }
        };
        drop(payload);
    }
    pub(crate) fn acquire_meaning<G>(&self, meaning: crate::meaning::MeaningWord<G>) {
        if let crate::meaning::MeaningWord::Macro { definition, .. } = meaning {
            self.acquire(definition);
        }
    }
    pub(crate) fn release_meaning<G>(&self, meaning: crate::meaning::MeaningWord<G>) {
        if let crate::meaning::MeaningWord::Macro { definition, .. } = meaning {
            self.release(definition);
        }
    }
    pub(crate) fn acquire_word<G>(&self, word: &crate::env::StateWord<G>) {
        if let crate::env::StateWord::Meaning(value) = word {
            self.acquire_meaning(*value);
        }
    }
    pub(crate) fn release_word<G>(&self, word: &crate::env::StateWord<G>) {
        if let crate::env::StateWord::Meaning(value) = word {
            self.release_meaning(*value);
        }
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        let bank = self.0.borrow();
        bank.format.capacity() * core::mem::size_of::<DefinitionEntry>()
            + bank.dynamic.capacity() * core::mem::size_of::<DynamicDefinition>()
    }
}

/// One newly published definition root. Assignment acquires its cell before this drops.
#[must_use]
pub struct PublishedDefinition<G> {
    reference: DefinitionRef<G>,
    roots: DefinitionRoots,
}
impl<G> PublishedDefinition<G> {
    #[must_use]
    pub const fn reference(&self) -> DefinitionRef<G> {
        self.reference
    }
    /// Acquires another actual cold owner when publication has multiple destinations.
    pub fn retain(&self) -> Self {
        self.roots.acquire(self.reference);
        Self {
            reference: self.reference,
            roots: self.roots.clone(),
        }
    }
}
impl<G> core::fmt::Debug for PublishedDefinition<G> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PublishedDefinition(..)")
    }
}
impl<G> Drop for PublishedDefinition<G> {
    fn drop(&mut self) {
        self.roots.release(self.reference);
    }
}

pub(crate) struct DefinitionArena<G> {
    bank: Rc<RefCell<DefinitionBank>>,
    next_build_serial: u32,
    active_build: Option<ActiveDefinitionBuild>,
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
            bank: Rc::default(),
            next_build_serial: 1,
            active_build: None,
            build_words: Vec::new(),
            active_writer: None,
            accounting,
            _brand: PhantomData,
        }
    }
    pub(crate) fn roots(&self) -> DefinitionRoots {
        DefinitionRoots(Rc::clone(&self.bank))
    }
    #[inline(always)]
    pub(crate) fn admit_macro_definition(
        &self,
        id: DefinitionRef<G>,
    ) -> Option<AdmittedMacroDefinition<G>> {
        let bank = self.bank.borrow();
        let entry = bank.entry(id)?;
        if entry.parameter_len == 0 {
            Some(AdmittedMacroDefinition::SimpleMacro {
                body: (!entry.words.is_empty()).then(|| ResidentMacroBody::admit(entry)),
            })
        } else {
            Some(AdmittedMacroDefinition::MatchingMacro {
                body: ResidentMacroBody::admit(entry),
            })
        }
    }
    pub(crate) fn admit_macro_body(&self, id: DefinitionRef<G>) -> Option<ResidentMacroBody<G>> {
        Some(ResidentMacroBody::admit(self.bank.borrow().entry(id)?))
    }
    pub(crate) fn contents_equal(&self, left: DefinitionRef<G>, right: DefinitionRef<G>) -> bool {
        let bank = self.bank.borrow();
        match (bank.entry(left), bank.entry(right)) {
            (Some(left), Some(right)) => left.contents_equal(right),
            _ => false,
        }
    }
    pub(crate) fn abort_interrupted_build(&mut self) {
        self.abort_active_build();
    }
    pub(crate) fn allocate(
        &mut self,
        parameter_text: &[TokenWord],
        replacement_text: &[TokenWord],
    ) -> Result<PublishedDefinition<G>, DefinitionAllocationError> {
        self.allocate_from_iter(
            parameter_text.iter().copied(),
            replacement_text.iter().copied(),
        )
    }

    pub(crate) fn allocate_from_iter<Parameters, Replacement>(
        &mut self,
        parameter_text: Parameters,
        replacement_text: Replacement,
    ) -> Result<PublishedDefinition<G>, DefinitionAllocationError>
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
    ) -> Result<PublishedDefinition<G>, DefinitionAllocationError> {
        if self.active_build.is_some() || self.active_writer.is_some() {
            return Err(DefinitionAllocationError::InvalidDefinition);
        }
        self.validate_builder(builder)?;
        self.reserve_batch(1)?;
        Ok(self.publish_prevalidated(builder))
    }

    pub(crate) fn publish_prevalidated(
        &mut self,
        builder: &mut DefinitionBuilder,
    ) -> PublishedDefinition<G> {
        let entry = DefinitionEntry::new(
            builder.words().into(),
            builder.parameter_len,
            crate::token::OriginId::UNKNOWN,
            builder.pattern,
            self.accounting.clone(),
        );
        builder.phase = DefinitionBuildPhase::Published;
        self.push_dynamic(entry)
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
        let words: Box<[TokenWord]> = row
            .parameter_text
            .iter()
            .chain(&row.replacement_text)
            .map(|&word| TokenWord::from_raw(word))
            .collect();
        u32::try_from(words.len()).map_err(|_| DefinitionAllocationError::CapacityOverflow)?;
        reserve_rows(&mut self.bank.borrow_mut().format, 1)?;
        let entry = DefinitionEntry::new(
            words,
            parameter_len,
            crate::token::OriginId::UNKNOWN,
            pattern,
            self.accounting.clone(),
        );
        let mut bank = self.bank.borrow_mut();
        bank.format.push(entry);
        Ok(DefinitionRef::new(FORMAT_REGION, bank.format.len() - 1))
    }

    fn push_dynamic(&mut self, entry: DefinitionEntry) -> PublishedDefinition<G> {
        let mut bank = self.bank.borrow_mut();
        bank.dynamic.push(DynamicDefinition {
            payload: Some(entry),
            roots: 1,
        });
        let reference = DefinitionRef::new(DYNAMIC_REGION, bank.dynamic.len() - 1);
        PublishedDefinition {
            reference,
            roots: self.roots(),
        }
    }

    pub(crate) fn validate_builder(
        &self,
        builder: &DefinitionBuilder,
    ) -> Result<(), DefinitionAllocationError> {
        builder.validate_completed()
    }

    pub(crate) fn reserve_batch(&mut self, rows: usize) -> Result<(), DefinitionAllocationError> {
        reserve_rows(&mut self.bank.borrow_mut().dynamic, rows)
    }

    pub(crate) fn reserve_format_batch(
        &mut self,
        rows: usize,
    ) -> Result<(), DefinitionAllocationError> {
        reserve_rows(&mut self.bank.borrow_mut().format, rows)
    }

    #[must_use]
    pub(crate) fn get(&self, id: DefinitionRef<G>) -> DefinitionView<G> {
        DefinitionView {
            entry: self
                .bank
                .borrow()
                .entry(id)
                .expect("definition root is live")
                .clone(),
            _brand: PhantomData,
        }
    }
    pub(crate) fn len(&self) -> usize {
        let bank = self.bank.borrow();
        bank.format.len() + bank.dynamic.len()
    }
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
fn reserve_rows<T>(rows: &mut Vec<T>, additional: usize) -> Result<(), DefinitionAllocationError> {
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
