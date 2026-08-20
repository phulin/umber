//! Operation-scoped command scratch and explicit-root promotion.
//!
//! Values in this module are coordinates, never owners. One [`AttemptArena`]
//! owns all backing storage and can be truncated to a fixed-size mark or moved
//! intact into an in-process resource continuation.

use core::marker::PhantomData;
use core::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use tex_state::glue::GlueSpec;
use tex_state::provenance::OriginRecord;
use tex_state::token::{TokenWord, TracedTokenWord};
use tex_state::{
    DefinitionId, DefinitionPromotion, GenerationOwner, GlueId, PromotionError, ProvenanceId,
    TokenListId, TokenListPromotion, Universe,
};

#[cfg(test)]
#[path = "attempt/tests.rs"]
mod tests;

static NEXT_ATTEMPT_KEY: AtomicU64 = AtomicU64::new(1);
static NEXT_ATTEMPT_SERIAL: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttemptKey(NonZeroU64);

impl AttemptKey {
    fn fresh() -> Self {
        loop {
            if let Some(key) = NonZeroU64::new(NEXT_ATTEMPT_KEY.fetch_add(1, Ordering::Relaxed)) {
                return Self(key);
            }
        }
    }
}

macro_rules! attempt_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(crate) struct $name {
            key: NonZeroU64,
            row: u32,
            serial: NonZeroU64,
        }

        impl $name {
            fn new(key: AttemptKey, row: usize) -> Result<Self, AttemptError> {
                Ok(Self {
                    key: key.0,
                    row: u32::try_from(row).map_err(|_| AttemptError::CapacityOverflow)?,
                    serial: NonZeroU64::new(NEXT_ATTEMPT_SERIAL.fetch_add(1, Ordering::Relaxed))
                        .ok_or(AttemptError::CapacityOverflow)?,
                })
            }

            const fn index(self) -> usize {
                self.row as usize
            }
        }
    };
}

attempt_id!(AttemptTokenListId);
attempt_id!(AttemptGlueId);
attempt_id!(AttemptDefinitionId);
attempt_id!(AttemptArgumentRecordId);
attempt_id!(AttemptTokenBufferId);
attempt_id!(AttemptNameId);
attempt_id!(AttemptProvenanceId);

/// Provenance beside one attempt token: either an already-admitted compact
/// origin or a typed row owned by this attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttemptOrigin {
    Admitted(tex_state::token::OriginId),
    Local(AttemptProvenanceId),
}

/// A typed open token-builder cursor. It names no allocation of its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AttemptTokenBuilder {
    key: NonZeroU64,
    start: u32,
    depth: u32,
    serial: NonZeroU64,
}

/// Fixed-size rollback coordinates for every command-attempt table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AttemptMark {
    key: NonZeroU64,
    traced_words: u32,
    traced_origins: u32,
    token_scratch: u32,
    origin_scratch: u32,
    token_builders: u32,
    token_lists: u32,
    glue_values: u32,
    definitions: u32,
    argument_words: u32,
    argument_records: u32,
    token_buffers: u32,
    name_bytes: u32,
    names: u32,
    provenance: u32,
}

/// Invalid foreign coordinates or bounded-capacity failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttemptError {
    ForeignAttempt,
    InvalidCoordinate,
    CapacityOverflow,
    AllocationFailed,
    Promotion(PromotionError),
}

impl From<PromotionError> for AttemptError {
    fn from(error: PromotionError) -> Self {
        Self::Promotion(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttemptRange {
    start: u32,
    len: u32,
}

impl AttemptRange {
    fn checked(start: usize, len: usize) -> Result<Self, AttemptError> {
        let start = u32::try_from(start).map_err(|_| AttemptError::CapacityOverflow)?;
        let len = u32::try_from(len).map_err(|_| AttemptError::CapacityOverflow)?;
        start
            .checked_add(len)
            .ok_or(AttemptError::CapacityOverflow)?;
        Ok(Self { start, len })
    }

    fn resolve<T>(self, values: &[T]) -> Result<&[T], AttemptError> {
        let start = self.start as usize;
        let end = start
            .checked_add(self.len as usize)
            .ok_or(AttemptError::InvalidCoordinate)?;
        values
            .get(start..end)
            .ok_or(AttemptError::InvalidCoordinate)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttemptDefinition {
    parameter_text: AttemptTokenListId,
    replacement_text: AttemptTokenListId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttemptRow<T> {
    serial: NonZeroU64,
    value: T,
}

/// All command-side storage which shares one operation lifetime.
pub(crate) struct AttemptArena<G> {
    key: AttemptKey,
    traced_words: Vec<TracedTokenWord>,
    traced_origins: Vec<Option<AttemptProvenanceId>>,
    token_scratch: Vec<TracedTokenWord>,
    origin_scratch: Vec<Option<AttemptProvenanceId>>,
    token_builders: Vec<AttemptTokenBuilder>,
    token_lists: Vec<AttemptRow<AttemptRange>>,
    glue_values: Vec<AttemptRow<GlueSpec>>,
    definitions: Vec<AttemptRow<AttemptDefinition>>,
    argument_words: Vec<AttemptTokenListId>,
    argument_records: Vec<AttemptRow<AttemptRange>>,
    token_buffers: Vec<AttemptRow<Vec<TracedTokenWord>>>,
    name_bytes: Vec<u8>,
    names: Vec<AttemptRow<AttemptRange>>,
    provenance: Vec<AttemptRow<OriginRecord>>,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Default for AttemptArena<G> {
    fn default() -> Self {
        Self {
            key: AttemptKey::fresh(),
            traced_words: Vec::new(),
            traced_origins: Vec::new(),
            token_scratch: Vec::new(),
            origin_scratch: Vec::new(),
            token_builders: Vec::new(),
            token_lists: Vec::new(),
            glue_values: Vec::new(),
            definitions: Vec::new(),
            argument_words: Vec::new(),
            argument_records: Vec::new(),
            token_buffers: Vec::new(),
            name_bytes: Vec::new(),
            names: Vec::new(),
            provenance: Vec::new(),
            _generation: PhantomData,
        }
    }
}

impl<G> AttemptArena<G> {
    #[must_use]
    pub(crate) fn mark(&self) -> AttemptMark {
        AttemptMark {
            key: self.key.0,
            traced_words: u32::try_from(self.traced_words.len())
                .expect("attempt traced-word length is bounded"),
            traced_origins: u32::try_from(self.traced_origins.len())
                .expect("attempt traced-origin length is bounded"),
            token_scratch: u32::try_from(self.token_scratch.len())
                .expect("attempt token-scratch length is bounded"),
            origin_scratch: u32::try_from(self.origin_scratch.len())
                .expect("attempt origin-scratch length is bounded"),
            token_builders: u32::try_from(self.token_builders.len())
                .expect("attempt token-builder length is bounded"),
            token_lists: u32::try_from(self.token_lists.len())
                .expect("attempt token-list length is bounded"),
            glue_values: u32::try_from(self.glue_values.len())
                .expect("attempt glue length is bounded"),
            definitions: u32::try_from(self.definitions.len())
                .expect("attempt definition length is bounded"),
            argument_words: u32::try_from(self.argument_words.len())
                .expect("attempt argument-word length is bounded"),
            argument_records: u32::try_from(self.argument_records.len())
                .expect("attempt argument-record length is bounded"),
            token_buffers: u32::try_from(self.token_buffers.len())
                .expect("attempt token-buffer length is bounded"),
            name_bytes: u32::try_from(self.name_bytes.len())
                .expect("attempt name-byte length is bounded"),
            names: u32::try_from(self.names.len()).expect("attempt name length is bounded"),
            provenance: u32::try_from(self.provenance.len())
                .expect("attempt provenance length is bounded"),
        }
    }

    /// Rejects a suffix in constant time per table. No value is inspected.
    pub(crate) fn truncate(&mut self, mark: AttemptMark) -> Result<(), AttemptError> {
        if mark.key != self.key.0 {
            return Err(AttemptError::ForeignAttempt);
        }
        let lengths = [
            (mark.traced_words as usize, self.traced_words.len()),
            (mark.traced_origins as usize, self.traced_origins.len()),
            (mark.token_scratch as usize, self.token_scratch.len()),
            (mark.origin_scratch as usize, self.origin_scratch.len()),
            (mark.token_builders as usize, self.token_builders.len()),
            (mark.token_lists as usize, self.token_lists.len()),
            (mark.glue_values as usize, self.glue_values.len()),
            (mark.definitions as usize, self.definitions.len()),
            (mark.argument_words as usize, self.argument_words.len()),
            (mark.argument_records as usize, self.argument_records.len()),
            (mark.token_buffers as usize, self.token_buffers.len()),
            (mark.name_bytes as usize, self.name_bytes.len()),
            (mark.names as usize, self.names.len()),
            (mark.provenance as usize, self.provenance.len()),
        ];
        if lengths.iter().any(|(mark, live)| mark > live) {
            return Err(AttemptError::InvalidCoordinate);
        }
        // Child coordinates disappear before their backing suffixes.
        self.names.truncate(mark.names as usize);
        self.provenance.truncate(mark.provenance as usize);
        self.argument_records
            .truncate(mark.argument_records as usize);
        self.argument_words.truncate(mark.argument_words as usize);
        self.token_buffers.truncate(mark.token_buffers as usize);
        self.definitions.truncate(mark.definitions as usize);
        self.glue_values.truncate(mark.glue_values as usize);
        self.token_lists.truncate(mark.token_lists as usize);
        self.token_builders.truncate(mark.token_builders as usize);
        self.token_scratch.truncate(mark.token_scratch as usize);
        self.origin_scratch.truncate(mark.origin_scratch as usize);
        self.traced_words.truncate(mark.traced_words as usize);
        self.traced_origins.truncate(mark.traced_origins as usize);
        self.name_bytes.truncate(mark.name_bytes as usize);
        Ok(())
    }

    #[must_use]
    pub(crate) fn begin_token_list(&mut self) -> Result<AttemptTokenBuilder, AttemptError> {
        let builder = AttemptTokenBuilder {
            key: self.key.0,
            start: u32::try_from(self.token_scratch.len())
                .map_err(|_| AttemptError::CapacityOverflow)?,
            depth: u32::try_from(self.token_builders.len())
                .map_err(|_| AttemptError::CapacityOverflow)?,
            serial: NonZeroU64::new(NEXT_ATTEMPT_SERIAL.fetch_add(1, Ordering::Relaxed))
                .ok_or(AttemptError::CapacityOverflow)?,
        };
        self.token_builders
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_builders.push(builder);
        Ok(builder)
    }

    pub(crate) fn push_token(
        &mut self,
        builder: AttemptTokenBuilder,
        word: TracedTokenWord,
    ) -> Result<(), AttemptError> {
        self.push_token_parts(builder, word, None)
    }

    pub(crate) fn push_token_with_local_origin(
        &mut self,
        builder: AttemptTokenBuilder,
        word: TokenWord,
        origin: AttemptProvenanceId,
    ) -> Result<(), AttemptError> {
        self.provenance(origin)?;
        self.push_token_parts(
            builder,
            TracedTokenWord::from_parts(word, tex_state::token::OriginId::UNKNOWN),
            Some(origin),
        )
    }

    fn push_token_parts(
        &mut self,
        builder: AttemptTokenBuilder,
        word: TracedTokenWord,
        origin: Option<AttemptProvenanceId>,
    ) -> Result<(), AttemptError> {
        self.validate_key(builder.key)?;
        if self.token_builders.last() != Some(&builder)
            || builder.start as usize > self.token_scratch.len()
        {
            return Err(AttemptError::InvalidCoordinate);
        }
        self.token_scratch
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.origin_scratch
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_scratch.push(word);
        self.origin_scratch.push(origin);
        Ok(())
    }

    pub(crate) fn finish_token_list(
        &mut self,
        builder: AttemptTokenBuilder,
    ) -> Result<AttemptTokenListId, AttemptError> {
        self.validate_key(builder.key)?;
        if self.token_builders.last() != Some(&builder) {
            return Err(AttemptError::InvalidCoordinate);
        }
        let start = builder.start as usize;
        let len = self
            .token_scratch
            .len()
            .checked_sub(start)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let range = AttemptRange::checked(self.traced_words.len(), len)?;
        let id = AttemptTokenListId::new(self.key, self.token_lists.len())?;
        self.traced_words
            .try_reserve(len)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.traced_origins
            .try_reserve(len)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_lists
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.traced_words
            .extend_from_slice(&self.token_scratch[start..]);
        self.traced_origins
            .extend_from_slice(&self.origin_scratch[start..]);
        self.token_scratch.truncate(start);
        self.origin_scratch.truncate(start);
        self.token_builders.pop();
        self.token_lists.push(AttemptRow {
            serial: id.serial,
            value: range,
        });
        Ok(id)
    }

    pub(crate) fn allocate_token_list(
        &mut self,
        words: impl IntoIterator<Item = TracedTokenWord>,
    ) -> Result<AttemptTokenListId, AttemptError> {
        let mark = self.mark();
        let result = (|| {
            let builder = self.begin_token_list()?;
            for word in words {
                self.push_token(builder, word)?;
            }
            self.finish_token_list(builder)
        })();
        if result.is_err() {
            self.truncate(mark)
                .expect("the allocation-local attempt mark is valid");
        }
        result
    }

    pub(crate) fn token_words(
        &self,
        id: AttemptTokenListId,
    ) -> Result<&[TracedTokenWord], AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .token_lists
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        row.value.resolve(&self.traced_words)
    }

    pub(crate) fn token_word(
        &self,
        id: AttemptTokenListId,
        index: usize,
    ) -> Result<TracedTokenWord, AttemptError> {
        self.token_words(id)?
            .get(index)
            .copied()
            .ok_or(AttemptError::InvalidCoordinate)
    }

    pub(crate) fn allocate_glue(&mut self, value: GlueSpec) -> Result<AttemptGlueId, AttemptError> {
        let id = AttemptGlueId::new(self.key, self.glue_values.len())?;
        self.glue_values
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.glue_values.push(AttemptRow {
            serial: id.serial,
            value,
        });
        Ok(id)
    }

    pub(crate) fn allocate_provenance(
        &mut self,
        value: OriginRecord,
    ) -> Result<AttemptProvenanceId, AttemptError> {
        let id = AttemptProvenanceId::new(self.key, self.provenance.len())?;
        self.provenance
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.provenance.push(AttemptRow {
            serial: id.serial,
            value,
        });
        Ok(id)
    }

    pub(crate) fn provenance(&self, id: AttemptProvenanceId) -> Result<OriginRecord, AttemptError> {
        self.validate_key(id.key)?;
        self.provenance
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .map(|row| row.value)
            .ok_or(AttemptError::InvalidCoordinate)
    }

    pub(crate) fn token_origin(
        &self,
        id: AttemptTokenListId,
        index: usize,
    ) -> Result<AttemptOrigin, AttemptError> {
        self.validate_key(id.key)?;
        let range = self
            .token_lists
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?
            .value;
        if index >= range.len as usize {
            return Err(AttemptError::InvalidCoordinate);
        }
        let absolute = range.start as usize + index;
        match self.traced_origins[absolute] {
            Some(origin) => Ok(AttemptOrigin::Local(origin)),
            None => Ok(AttemptOrigin::Admitted(
                self.traced_words[absolute].origin(),
            )),
        }
    }

    pub(crate) fn glue(&self, id: AttemptGlueId) -> Result<GlueSpec, AttemptError> {
        self.validate_key(id.key)?;
        self.glue_values
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .map(|row| row.value)
            .ok_or(AttemptError::InvalidCoordinate)
    }

    pub(crate) fn allocate_definition(
        &mut self,
        parameter_text: AttemptTokenListId,
        replacement_text: AttemptTokenListId,
    ) -> Result<AttemptDefinitionId, AttemptError> {
        self.token_words(parameter_text)?;
        self.token_words(replacement_text)?;
        let id = AttemptDefinitionId::new(self.key, self.definitions.len())?;
        self.definitions
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.definitions.push(AttemptRow {
            serial: id.serial,
            value: AttemptDefinition {
                parameter_text,
                replacement_text,
            },
        });
        Ok(id)
    }

    /// Stores one macro activation's argument ranges without cloning tokens.
    pub(crate) fn allocate_arguments(
        &mut self,
        arguments: &[AttemptTokenListId],
    ) -> Result<AttemptArgumentRecordId, AttemptError> {
        if arguments.len() > 9 {
            return Err(AttemptError::InvalidCoordinate);
        }
        for &argument in arguments {
            self.token_words(argument)?;
        }
        let range = AttemptRange::checked(self.argument_words.len(), arguments.len())?;
        let id = AttemptArgumentRecordId::new(self.key, self.argument_records.len())?;
        self.argument_words
            .try_reserve(arguments.len())
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.argument_records
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.argument_words.extend_from_slice(arguments);
        self.argument_records.push(AttemptRow {
            serial: id.serial,
            value: range,
        });
        Ok(id)
    }

    /// Allocates one mutable scanner buffer owned by this attempt.
    pub(crate) fn allocate_token_buffer(&mut self) -> Result<AttemptTokenBufferId, AttemptError> {
        let id = AttemptTokenBufferId::new(self.key, self.token_buffers.len())?;
        self.token_buffers
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_buffers.push(AttemptRow {
            serial: id.serial,
            value: Vec::new(),
        });
        Ok(id)
    }

    pub(crate) fn token_buffer(
        &self,
        id: AttemptTokenBufferId,
    ) -> Result<&[TracedTokenWord], AttemptError> {
        self.validate_key(id.key)?;
        self.token_buffers
            .get(id.index())
            .filter(|row| row.serial == id.serial)
            .map(|row| row.value.as_slice())
            .ok_or(AttemptError::InvalidCoordinate)
    }

    fn token_buffer_mut(
        &mut self,
        id: AttemptTokenBufferId,
    ) -> Result<&mut Vec<TracedTokenWord>, AttemptError> {
        self.validate_key(id.key)?;
        self.token_buffers
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial)
            .map(|row| &mut row.value)
            .ok_or(AttemptError::InvalidCoordinate)
    }

    pub(crate) fn push_buffer_token(
        &mut self,
        id: AttemptTokenBufferId,
        word: TracedTokenWord,
    ) -> Result<(), AttemptError> {
        self.token_buffer_mut(id)?
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_buffer_mut(id)?.push(word);
        Ok(())
    }

    pub(crate) fn drain_buffer_prefix(
        &mut self,
        id: AttemptTokenBufferId,
        len: usize,
    ) -> Result<Vec<TracedTokenWord>, AttemptError> {
        let buffer = self.token_buffer_mut(id)?;
        if len > buffer.len() {
            return Err(AttemptError::InvalidCoordinate);
        }
        Ok(buffer.drain(..len).collect())
    }

    pub(crate) fn strip_buffer_outer_group(
        &mut self,
        id: AttemptTokenBufferId,
    ) -> Result<(), AttemptError> {
        let buffer = self.token_buffer_mut(id)?;
        if buffer.len() >= 2 {
            buffer.pop();
            buffer.remove(0);
        }
        Ok(())
    }

    pub(crate) fn finish_token_buffer(
        &mut self,
        id: AttemptTokenBufferId,
    ) -> Result<AttemptTokenListId, AttemptError> {
        let words = self.token_buffer(id)?.to_vec();
        self.allocate_token_list(words)
    }

    pub(crate) fn arguments(
        &self,
        id: AttemptArgumentRecordId,
    ) -> Result<&[AttemptTokenListId], AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .argument_records
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        row.value.resolve(&self.argument_words)
    }

    pub(crate) fn argument(
        &self,
        id: AttemptArgumentRecordId,
        slot: u8,
    ) -> Result<Option<AttemptTokenListId>, AttemptError> {
        if !(1..=9).contains(&slot) {
            return Err(AttemptError::InvalidCoordinate);
        }
        Ok(self.arguments(id)?.get(usize::from(slot - 1)).copied())
    }

    pub(crate) fn allocate_name(&mut self, name: &str) -> Result<AttemptNameId, AttemptError> {
        let range = AttemptRange::checked(self.name_bytes.len(), name.len())?;
        let id = AttemptNameId::new(self.key, self.names.len())?;
        self.name_bytes
            .try_reserve(name.len())
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.names
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.name_bytes.extend_from_slice(name.as_bytes());
        self.names.push(AttemptRow {
            serial: id.serial,
            value: range,
        });
        Ok(id)
    }

    pub(crate) fn name(&self, id: AttemptNameId) -> Result<&str, AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .names
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let bytes = row.value.resolve(&self.name_bytes)?;
        std::str::from_utf8(bytes).map_err(|_| AttemptError::InvalidCoordinate)
    }

    /// Copies only declared roots and schema-declared definition children.
    /// Dense relocation vectors exist only for this publication call.
    pub(crate) fn promote(
        &self,
        universe: &mut Universe<G>,
        roots: AttemptEscapeRoots<'_>,
    ) -> Result<AttemptPromotion<G>, AttemptError> {
        let mut token_relocation = vec![None; self.token_lists.len()];
        let mut glue_relocation = vec![None; self.glue_values.len()];
        let mut definition_relocation = vec![None; self.definitions.len()];
        let mut provenance_relocation = vec![None; self.provenance.len()];
        let mut token_scheduled = vec![false; self.token_lists.len()];
        let mut glue_scheduled = vec![false; self.glue_values.len()];
        let mut definition_scheduled = vec![false; self.definitions.len()];
        let mut provenance_scheduled = vec![false; self.provenance.len()];
        let mut token_sources = Vec::<(AttemptTokenListId, Vec<TokenWord>)>::new();
        let mut glue_sources = Vec::<(AttemptGlueId, GlueSpec)>::new();
        let mut definition_sources =
            Vec::<(AttemptDefinitionId, Vec<TokenWord>, Vec<TokenWord>)>::new();
        let mut provenance_sources = Vec::<(AttemptProvenanceId, OriginRecord)>::new();

        for &id in roots.token_lists {
            self.collect_token_source(id, &mut token_scheduled, &mut token_sources)?;
        }
        for &id in roots.glue {
            self.validate_key(id.key)?;
            let scheduled = glue_scheduled
                .get_mut(id.index())
                .ok_or(AttemptError::InvalidCoordinate)?;
            self.glue(id)?;
            if !*scheduled {
                *scheduled = true;
                glue_sources.push((id, self.glue(id)?));
            }
        }
        for &id in roots.definitions {
            self.validate_key(id.key)?;
            let scheduled = definition_scheduled
                .get_mut(id.index())
                .ok_or(AttemptError::InvalidCoordinate)?;
            let row = self
                .definitions
                .get(id.index())
                .copied()
                .filter(|row| row.serial == id.serial)
                .ok_or(AttemptError::InvalidCoordinate)?;
            if *scheduled {
                continue;
            }
            *scheduled = true;
            let definition = row.value;
            // The two token ranges are schema-declared children. Definition
            // text is copied into DefinitionArena directly, not published as
            // independent durable token-list rows unless separately rooted.
            let parameter_text = self.semantic_words(definition.parameter_text)?;
            let replacement_text = self.semantic_words(definition.replacement_text)?;
            definition_sources.push((id, parameter_text, replacement_text));
        }
        for &id in roots.provenance {
            self.validate_key(id.key)?;
            let scheduled = provenance_scheduled
                .get_mut(id.index())
                .ok_or(AttemptError::InvalidCoordinate)?;
            let record = self.provenance(id)?;
            if !*scheduled {
                *scheduled = true;
                provenance_sources.push((id, record));
            }
        }

        let definitions = definition_sources
            .iter()
            .map(
                |(_, parameter_text, replacement_text)| DefinitionPromotion {
                    parameter_text,
                    replacement_text,
                },
            )
            .collect::<Vec<_>>();
        let token_lists = token_sources
            .iter()
            .map(|(_, words)| TokenListPromotion { words })
            .collect::<Vec<_>>();
        let glue_values = glue_sources
            .iter()
            .map(|(_, glue)| *glue)
            .collect::<Vec<_>>();
        let provenance = provenance_sources
            .iter()
            .map(|(_, record)| *record)
            .collect::<Vec<_>>();
        let receipt =
            universe.promote_values(&definitions, &token_lists, &glue_values, &provenance)?;

        for ((source, _), destination) in token_sources.iter().zip(receipt.token_lists) {
            token_relocation[source.index()] = Some(destination);
        }
        for ((source, _), destination) in glue_sources.iter().zip(receipt.glue) {
            glue_relocation[source.index()] = Some(destination);
        }
        for ((source, _, _), destination) in definition_sources.iter().zip(receipt.definitions) {
            definition_relocation[source.index()] = Some(destination);
        }
        for ((source, _), destination) in provenance_sources.iter().zip(receipt.provenance) {
            provenance_relocation[source.index()] = Some(destination);
        }

        Ok(AttemptPromotion {
            token_lists: roots
                .token_lists
                .iter()
                .map(|id| token_relocation[id.index()].expect("declared root was promoted"))
                .collect(),
            glue: roots
                .glue
                .iter()
                .map(|id| glue_relocation[id.index()].expect("declared root was promoted"))
                .collect(),
            definitions: roots
                .definitions
                .iter()
                .map(|id| definition_relocation[id.index()].expect("declared root was promoted"))
                .collect(),
            provenance: roots
                .provenance
                .iter()
                .map(|id| provenance_relocation[id.index()].expect("declared root was promoted"))
                .collect(),
        })
    }

    fn collect_token_source(
        &self,
        id: AttemptTokenListId,
        scheduled: &mut [bool],
        sources: &mut Vec<(AttemptTokenListId, Vec<TokenWord>)>,
    ) -> Result<(), AttemptError> {
        self.validate_key(id.key)?;
        let scheduled = scheduled
            .get_mut(id.index())
            .ok_or(AttemptError::InvalidCoordinate)?;
        self.token_words(id)?;
        if !*scheduled {
            *scheduled = true;
            sources.push((id, self.semantic_words(id)?));
        }
        Ok(())
    }

    fn semantic_words(&self, id: AttemptTokenListId) -> Result<Vec<TokenWord>, AttemptError> {
        Ok(self
            .token_words(id)?
            .iter()
            .map(|word| word.token_word())
            .collect())
    }

    fn validate_key(&self, key: NonZeroU64) -> Result<(), AttemptError> {
        if key == self.key.0 {
            Ok(())
        } else {
            Err(AttemptError::ForeignAttempt)
        }
    }
}

/// Explicit roots permitted to escape one command operation.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AttemptEscapeRoots<'a> {
    pub(crate) token_lists: &'a [AttemptTokenListId],
    pub(crate) glue: &'a [AttemptGlueId],
    pub(crate) definitions: &'a [AttemptDefinitionId],
    pub(crate) provenance: &'a [AttemptProvenanceId],
}

/// Durable coordinates returned in the caller's declared root order.
#[derive(Debug)]
pub(crate) struct AttemptPromotion<G> {
    pub(crate) token_lists: Vec<TokenListId<G>>,
    pub(crate) glue: Vec<GlueId<G>>,
    pub(crate) definitions: Vec<DefinitionId<G>>,
    pub(crate) provenance: Vec<ProvenanceId<G>>,
}

/// Opaque owner transferred between consecutive command operations.
///
/// Macro activations and other command continuations may intentionally keep
/// attempt coordinates live across more than one delivered command. The
/// owner moves; individual ids never retain it.
pub struct CommandAttempt<G> {
    arena: AttemptArena<G>,
}

impl<G> core::fmt::Debug for CommandAttempt<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("CommandAttempt(..)")
    }
}

impl<G> Default for CommandAttempt<G> {
    fn default() -> Self {
        Self {
            arena: AttemptArena::default(),
        }
    }
}

impl<G> CommandAttempt<G> {
    pub(crate) const fn arena(&self) -> &AttemptArena<G> {
        &self.arena
    }

    pub(crate) const fn arena_mut(&mut self) -> &mut AttemptArena<G> {
        &mut self.arena
    }
}

/// Integer-only state-machine position retained at a resource barrier.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AttemptResumePoint {
    pub command: u32,
    pub scanner: u32,
    pub expansion: u32,
    pub subordinate: u32,
}

/// Complete in-process command suspension package.
///
/// `R` owns the typed request and resume variant. No field borrows either the
/// attempt or generation; resumption consumes the package, validates its
/// coarse generation, drops that extra owner, and only then re-borrows live
/// storage through `Universe`.
pub struct PendingCommandAttempt<G, R> {
    attempt: CommandAttempt<G>,
    generation: GenerationOwner<G>,
    resume: AttemptResumePoint,
    pending: R,
}

impl<G, R> PendingCommandAttempt<G, R> {
    #[must_use]
    pub fn new(
        attempt: CommandAttempt<G>,
        generation: GenerationOwner<G>,
        resume: AttemptResumePoint,
        pending: R,
    ) -> Self {
        Self {
            attempt,
            generation,
            resume,
            pending,
        }
    }

    pub fn resume(
        self,
        universe: &Universe<G>,
    ) -> Result<(CommandAttempt<G>, AttemptResumePoint, R), Self> {
        if !universe.owns_generation(&self.generation) {
            return Err(self);
        }
        let Self {
            attempt,
            generation,
            resume,
            pending,
        } = self;
        drop(generation);
        Ok((attempt, resume, pending))
    }
}
