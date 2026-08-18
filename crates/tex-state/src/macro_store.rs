//! Reachability-owned immutable macro definitions.
//!
//! A definition occurrence has a timeline-local [`MacroDefinitionId`] and
//! owns one exact immutable body. Equivalent occurrences keep their distinct
//! diagnostic identity while the weak body pool deduplicates flags,
//! parameter structure, and parameter/replacement token-list roots.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::identity::HandleIdentity;
use crate::ids::{MacroDefinitionId, OriginListId, TokenListId};
use crate::meaning::MeaningFlags;
use crate::patch_domain::{PatchAllocationDomain, PatchHandle, PatchRoot, PatchRootWeak};
#[cfg(any(test, feature = "testing"))]
use crate::reachable_value::LookupWork;
use crate::reachable_value::ReachableValuePool;
use crate::token::{OriginId, Token};
use crate::token::{RootedTracedTokenWord, TracedTokenWord};
use crate::token_store::{TokenListRef, TokenSemanticId};

const MACRO_PARAMETER_SLOTS: usize = 9;
const PACKED_MACRO_CHUNK_RECORDS: usize = 64;
mod owned;

pub use owned::MacroDefinitionRef;
use owned::{MacroBodyRef, MacroBodySemanticId, MacroBodyValue, MacroDefinitionValue};

/// Copy-only parameter program admitted with one packed macro chunk.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PackedMacroPattern {
    leading_end: u32,
    offsets: [u32; MACRO_PARAMETER_SLOTS],
    widths: [u8; MACRO_PARAMETER_SLOTS],
    count: u8,
}

impl PackedMacroPattern {
    #[must_use]
    pub const fn parameter_count(self) -> usize {
        self.count as usize
    }

    #[must_use]
    pub const fn leading_end(self) -> usize {
        self.leading_end as usize
    }

    #[must_use]
    pub const fn delimiter_bounds(self, parameter: usize, token_count: usize) -> (usize, usize) {
        assert!(parameter < self.parameter_count());
        let start = self.offsets[parameter] as usize + self.widths[parameter] as usize;
        let end = if parameter + 1 < self.parameter_count() {
            self.offsets[parameter + 1] as usize
        } else {
            token_count
        };
        (start, end)
    }

    #[must_use]
    pub const fn marker_index(self, parameter: usize) -> Option<usize> {
        assert!(parameter < self.parameter_count());
        if self.widths[parameter] == 2 {
            Some(self.offsets[parameter] as usize)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PackedMacroRecord {
    definition: MacroDefinitionId,
    flags: MeaningFlags,
    parameter_root: u32,
    replacement_root: u32,
    pattern: PackedMacroPattern,
    parameter_start: u32,
    parameter_len: u32,
    allocation_len: u32,
    replacement_len: u32,
    observation_operand: i64,
    allocation_serial: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PackedMacroDefinitionRoots {
    definition: MacroDefinitionId,
    parameter_text: TokenListRef,
    replacement_text: TokenListRef,
    provenance: Option<MacroDefinitionProvenance>,
}

/// One immutable admitted owner for up to 64 packed macro records.
///
/// Cloning this value retains a whole chunk once. Individual definitions,
/// parameter programs, replacement words, and replay positions are copy-only
/// coordinates within the chunk.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PackedMacroChunkOwner {
    chunk: Arc<PackedMacroChunk>,
    roots: Arc<[PackedMacroDefinitionRoots]>,
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
struct PackedMacroChunk {
    records: Vec<Option<PackedMacroRecord>>,
    words: Vec<TracedTokenWord>,
    token_ids: Vec<TokenListId>,
}

const _: () = assert!(core::mem::size_of::<PackedMacroPattern>() == 52);
const _: () = assert!(core::mem::size_of::<PackedMacroRecord>() == 112);

impl PackedMacroChunkOwner {
    fn record(&self, definition: MacroDefinitionId) -> Option<&PackedMacroRecord> {
        let index = definition.raw() as usize % PACKED_MACRO_CHUNK_RECORDS;
        self.chunk
            .records
            .get(index)?
            .as_ref()
            .filter(|record| record.definition == definition)
    }

    #[must_use]
    pub fn contains(&self, definition: MacroDefinitionId) -> bool {
        self.record(definition).is_some()
    }

    #[must_use]
    pub fn owns_definition_slot(&self, definition: MacroDefinitionId) -> bool {
        self.chunk
            .records
            .iter()
            .flatten()
            .next()
            .is_some_and(|record| {
                record.definition.raw() / PACKED_MACRO_CHUNK_RECORDS as u32
                    == definition.raw() / PACKED_MACRO_CHUNK_RECORDS as u32
            })
    }

    #[must_use]
    pub fn meaning(&self, definition: MacroDefinitionId) -> Option<MacroMeaning> {
        let record = self.record(definition)?;
        let roots = self.definition_roots(definition)?;
        debug_assert_eq!(
            self.chunk.token_ids[record.parameter_root as usize],
            roots.parameter_text.id()
        );
        debug_assert_eq!(
            self.chunk.token_ids[record.replacement_root as usize],
            roots.replacement_text.id()
        );
        Some(MacroMeaning::new(
            record.flags,
            roots.parameter_text.id(),
            roots.replacement_text.id(),
        ))
    }

    #[must_use]
    pub fn pattern(&self, definition: MacroDefinitionId) -> Option<PackedMacroPattern> {
        Some(self.record(definition)?.pattern)
    }

    #[must_use]
    pub fn parameter_token(&self, definition: MacroDefinitionId, index: usize) -> Option<Token> {
        let record = self.record(definition)?;
        if index >= record.parameter_len as usize {
            return None;
        }
        self.chunk
            .words
            .get(record.parameter_start as usize + index)?
            .token()
    }

    #[must_use]
    pub fn parameter_len(&self, definition: MacroDefinitionId) -> Option<usize> {
        Some(self.record(definition)?.parameter_len as usize)
    }

    #[must_use]
    pub fn replacement_len(&self, definition: MacroDefinitionId) -> Option<usize> {
        Some(self.record(definition)?.replacement_len as usize)
    }

    #[must_use]
    pub fn replacement_word(
        &self,
        definition: MacroDefinitionId,
        index: usize,
    ) -> Option<RootedTracedTokenWord> {
        let record = self.record(definition)?;
        if index >= record.replacement_len as usize {
            return None;
        }
        let word = self
            .chunk
            .words
            .get(record.parameter_start as usize + record.parameter_len as usize + index)
            .copied()?;
        let root = self
            .definition_roots(definition)
            .and_then(|roots| roots.provenance.as_ref())
            .and_then(|provenance| provenance.replacement_ref().root(index))
            .unwrap_or_else(crate::provenance::OriginRef::unknown);
        Some(RootedTracedTokenWord::from_word(word, root))
    }

    #[must_use]
    pub fn provenance(&self, definition: MacroDefinitionId) -> Option<MacroDefinitionProvenance> {
        self.record(definition)?;
        self.definition_roots(definition)?.provenance.clone()
    }

    #[must_use]
    pub fn observation_operand(&self, definition: MacroDefinitionId) -> Option<i64> {
        Some(self.record(definition)?.observation_operand)
    }

    fn definition_roots(
        &self,
        definition: MacroDefinitionId,
    ) -> Option<&PackedMacroDefinitionRoots> {
        self.roots
            .iter()
            .find(|roots| roots.definition == definition)
    }
}

/// Allocation-free index of parameter markers in frozen macro parameter text.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MacroParameterPattern {
    tokens: Arc<[Token]>,
    offsets: [u32; MACRO_PARAMETER_SLOTS],
    widths: [u8; MACRO_PARAMETER_SLOTS],
    count: u8,
}

impl MacroParameterPattern {
    pub fn from_tokens(tokens: &[Token]) -> Self {
        let mut offsets = [0; MACRO_PARAMETER_SLOTS];
        let mut widths = [0; MACRO_PARAMETER_SLOTS];
        let mut count = 0_usize;
        for (index, token) in tokens.iter().enumerate() {
            if matches!(token, Token::Param(_)) {
                assert!(
                    count < MACRO_PARAMETER_SLOTS,
                    "macro has more than nine parameters"
                );
                let has_spelled_marker = index != 0
                    && matches!(
                        tokens[index - 1],
                        Token::Char {
                            cat: crate::token::Catcode::Parameter,
                            ..
                        }
                    );
                offsets[count] = u32::try_from(index - usize::from(has_spelled_marker))
                    .expect("token list length exceeds u32");
                widths[count] = if has_spelled_marker { 2 } else { 1 };
                count += 1;
            }
        }
        Self {
            tokens: Arc::from(tokens),
            offsets,
            widths,
            count: count as u8,
        }
    }

    #[must_use]
    pub const fn parameter_count(&self) -> usize {
        self.count as usize
    }

    #[must_use]
    pub fn leading_end(&self, token_count: usize) -> usize {
        if self.count == 0 {
            token_count
        } else {
            self.offsets[0] as usize
        }
    }

    #[must_use]
    pub fn delimiter_bounds(&self, parameter: usize, token_count: usize) -> (usize, usize) {
        assert!(parameter < self.parameter_count());
        let start = self.offsets[parameter] as usize + usize::from(self.widths[parameter]);
        let end = if parameter + 1 < self.parameter_count() {
            self.offsets[parameter + 1] as usize
        } else {
            token_count
        };
        (start, end)
    }

    #[must_use]
    pub fn leading(&self) -> &[Token] {
        &self.tokens[..self.leading_end(self.tokens.len())]
    }

    #[must_use]
    pub fn delimiter(&self, parameter: usize) -> &[Token] {
        let (start, end) = self.delimiter_bounds(parameter, self.tokens.len());
        &self.tokens[start..end]
    }

    /// Character code retained by TeX82 §476's match token.
    #[must_use]
    pub fn marker(&self, parameter: usize) -> char {
        assert!(parameter < self.parameter_count());
        if self.widths[parameter] == 2 {
            let Token::Char {
                ch,
                cat: crate::token::Catcode::Parameter,
            } = self.tokens[self.offsets[parameter] as usize]
            else {
                unreachable!("two-token parameter marker has parameter catcode")
            };
            ch
        } else {
            '#'
        }
    }

    fn packed(&self) -> PackedMacroPattern {
        PackedMacroPattern {
            leading_end: u32::try_from(self.leading_end(self.tokens.len()))
                .expect("macro parameter text exceeds u32"),
            offsets: self.offsets,
            widths: self.widths,
            count: self.count,
        }
    }
}

/// Public semantic macro-body aggregate used at the Universe boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroMeaning {
    flags: MeaningFlags,
    parameter_text: TokenListId,
    replacement_text: TokenListId,
}

impl MacroMeaning {
    /// Creates a macro meaning over already-frozen token lists.
    #[must_use]
    pub const fn new(
        flags: MeaningFlags,
        parameter_text: TokenListId,
        replacement_text: TokenListId,
    ) -> Self {
        Self {
            flags,
            parameter_text,
            replacement_text,
        }
    }

    #[must_use]
    pub const fn flags(self) -> MeaningFlags {
        self.flags
    }

    #[must_use]
    pub const fn parameter_text(self) -> TokenListId {
        self.parameter_text
    }

    #[must_use]
    pub const fn replacement_text(self) -> TokenListId {
        self.replacement_text
    }

    #[must_use]
    pub const fn semantic_eq(self, other: Self) -> bool {
        self.flags.bits() == other.flags.bits()
            && self.parameter_text.raw() == other.parameter_text.raw()
            && self.replacement_text.raw() == other.replacement_text.raw()
    }
}

/// Diagnostic provenance captured while scanning one definition occurrence.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MacroDefinitionProvenance {
    definition_origin: crate::provenance::OriginRef,
    parameter_origins: crate::provenance::OriginListRef,
    replacement_origins: crate::provenance::OriginListRef,
}

impl MacroDefinitionProvenance {
    #[must_use]
    pub const fn new(
        definition_origin: crate::provenance::OriginRef,
        parameter_origins: crate::provenance::OriginListRef,
        replacement_origins: crate::provenance::OriginListRef,
    ) -> Self {
        Self {
            definition_origin,
            parameter_origins,
            replacement_origins,
        }
    }

    #[must_use]
    pub fn unknown() -> Self {
        Self {
            definition_origin: crate::provenance::OriginRef::unknown(),
            parameter_origins: crate::provenance::OriginListRef::empty(),
            replacement_origins: crate::provenance::OriginListRef::empty(),
        }
    }

    #[must_use]
    pub fn definition_origin(&self) -> OriginId {
        self.definition_origin.id()
    }

    #[must_use]
    pub const fn definition_ref(&self) -> &crate::provenance::OriginRef {
        &self.definition_origin
    }

    #[must_use]
    pub fn parameter_origins(&self) -> OriginListId {
        self.parameter_origins.id()
    }

    #[must_use]
    pub const fn parameter_ref(&self) -> &crate::provenance::OriginListRef {
        &self.parameter_origins
    }

    #[must_use]
    pub fn replacement_origins(&self) -> OriginListId {
        self.replacement_origins.id()
    }

    #[must_use]
    pub const fn replacement_ref(&self) -> &crate::provenance::OriginListRef {
        &self.replacement_origins
    }
}

/// Rollback state for private macro allocations and compatibility operands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MacroStoreMark {
    bodies: u32,
    pub(crate) definitions: u32,
    packed_serial: u64,
    patch_events: u32,
    next_observation_operand: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PatchEvent {
    Body(HandleIdentity),
    Definition(MacroDefinitionId),
}

#[cfg(any(test, feature = "testing"))]
pub(crate) type PoolShape = (usize, usize, usize, usize, usize, usize);

/// Weak macro-body and definition-occurrence storage.
#[derive(Debug)]
pub struct MacroStore {
    bodies: ReachableValuePool<MacroBodySemanticId, MacroBodyValue>,
    definitions: ReachableValuePool<u64, MacroDefinitionValue>,
    frozen_roots: Arc<[MacroDefinitionRef]>,
    next_definition_serial: u64,
    next_observation_operand: i64,
    body_patch_handles: HashMap<HandleIdentity, PatchHandle<MacroBodyValue>>,
    body_patch_leases: HashMap<HandleIdentity, PatchRootWeak>,
    definition_patch_handles: HashMap<MacroDefinitionId, PatchHandle<MacroDefinitionValue>>,
    definition_patch_leases: HashMap<MacroDefinitionId, PatchRootWeak>,
    patch_order: Vec<PatchEvent>,
    packed_chunks: Vec<Arc<PackedMacroChunk>>,
    next_packed_serial: u64,
    #[cfg(any(test, feature = "testing"))]
    force_candidate_collision: bool,
}

impl Clone for MacroStore {
    fn clone(&self) -> Self {
        debug_assert!(
            self.patch_order.is_empty(),
            "private macro allocations cannot cross a generation fork"
        );
        Self {
            bodies: self.bodies.clone(),
            definitions: self.definitions.clone(),
            frozen_roots: Arc::clone(&self.frozen_roots),
            next_definition_serial: self.next_definition_serial,
            next_observation_operand: self.next_observation_operand,
            body_patch_handles: HashMap::new(),
            body_patch_leases: HashMap::new(),
            definition_patch_handles: HashMap::new(),
            definition_patch_leases: HashMap::new(),
            patch_order: Vec::new(),
            packed_chunks: self.packed_chunks.clone(),
            next_packed_serial: self.next_packed_serial,
            #[cfg(any(test, feature = "testing"))]
            force_candidate_collision: self.force_candidate_collision,
        }
    }
}

impl MacroStore {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            bodies: ReachableValuePool::new(),
            definitions: ReachableValuePool::new(),
            frozen_roots: Arc::from([]),
            next_definition_serial: 0,
            next_observation_operand: 249_985,
            body_patch_handles: HashMap::new(),
            body_patch_leases: HashMap::new(),
            definition_patch_handles: HashMap::new(),
            definition_patch_leases: HashMap::new(),
            patch_order: Vec::new(),
            packed_chunks: Vec::new(),
            next_packed_serial: 0,
            #[cfg(any(test, feature = "testing"))]
            force_candidate_collision: false,
        }
    }

    /// Installs validated frozen definitions as one explicitly owned base.
    pub(crate) fn from_frozen(
        definitions: Vec<MacroMeaning>,
        parameter_roots: Vec<TokenListRef>,
        replacement_roots: Vec<TokenListRef>,
        parameter_patterns: Vec<MacroParameterPattern>,
        parameter_semantic_ids: Vec<TokenSemanticId>,
        replacement_semantic_ids: Vec<TokenSemanticId>,
        observation_widths: Vec<u32>,
    ) -> Result<Self, &'static str> {
        let len = definitions.len();
        if parameter_roots.len() != len
            || replacement_roots.len() != len
            || parameter_patterns.len() != len
            || parameter_semantic_ids.len() != len
            || replacement_semantic_ids.len() != len
            || observation_widths.len() != len
        {
            return Err("frozen macro column length mismatch");
        }
        let mut bodies = ReachableValuePool::new();
        let mut operands = observation_operands(&observation_widths)?;
        let next_observation_operand = observation_widths
            .iter()
            .try_fold(249_985_i64, |next, width| {
                next.checked_sub(i64::from(*width))
            })
            .ok_or("macro observation operand underflow")?;
        let mut values = Vec::with_capacity(len);
        for (
            (
                ((((meaning, parameter_text), replacement_text), parameter_pattern), parameter_id),
                replacement_id,
            ),
            operand,
        ) in definitions
            .into_iter()
            .zip(parameter_roots)
            .zip(replacement_roots)
            .zip(parameter_patterns)
            .zip(parameter_semantic_ids)
            .zip(replacement_semantic_ids)
            .zip(operands.drain(..))
        {
            let semantic_id =
                MacroBodySemanticId::new(meaning.flags(), parameter_id, replacement_id);
            let value = MacroBodyValue {
                flags: meaning.flags(),
                parameter_text,
                replacement_text,
                parameter_pattern,
            };
            let body = MacroBodyRef {
                value: bodies.intern(semantic_id, value, MacroBodyValue::exact_eq),
                patch_root: None,
            };
            values.push(MacroDefinitionValue {
                body,
                provenance: OnceLock::new(),
                observation_operand: operand,
            });
        }
        let (definitions, roots) = ReachableValuePool::from_fixed_values(values, 0);
        let frozen_roots: Arc<[MacroDefinitionRef]> = roots
            .into_iter()
            .map(|value| MacroDefinitionRef {
                value,
                patch_root: None,
            })
            .collect::<Vec<_>>()
            .into();
        let mut store = Self {
            bodies,
            definitions,
            frozen_roots,
            next_definition_serial: len as u64,
            next_observation_operand,
            body_patch_handles: HashMap::new(),
            body_patch_leases: HashMap::new(),
            definition_patch_handles: HashMap::new(),
            definition_patch_leases: HashMap::new(),
            patch_order: Vec::new(),
            packed_chunks: Vec::new(),
            next_packed_serial: 0,
            #[cfg(any(test, feature = "testing"))]
            force_candidate_collision: false,
        };
        for raw in 0..len as u32 {
            let root = store.frozen_roots[raw as usize].clone();
            let body = root.value.value().body.value.value();
            store.install_packed_record(
                root.id(),
                body.meaning(),
                &body.parameter_pattern,
                body.parameter_text.tokens(),
                body.replacement_text.tokens(),
                Some((&body.parameter_text, &body.replacement_text)),
                None,
                root.value.value().observation_operand,
            );
        }
        Ok(store)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn intern_with_provenance(
        &mut self,
        meaning: MacroMeaning,
        parameter_root: TokenListRef,
        replacement_root: TokenListRef,
        parameter_pattern: MacroParameterPattern,
        parameter_semantic_id: TokenSemanticId,
        replacement_semantic_id: TokenSemanticId,
        provenance: Option<MacroDefinitionProvenance>,
        observation_width: u32,
        domain: Option<&mut PatchAllocationDomain>,
    ) -> MacroDefinitionRef {
        assert_eq!(parameter_root.id(), meaning.parameter_text());
        assert_eq!(replacement_root.id(), meaning.replacement_text());
        let semantic_id = MacroBodySemanticId::new(
            meaning.flags(),
            parameter_semantic_id,
            replacement_semantic_id,
        );
        #[cfg(any(test, feature = "testing"))]
        let semantic_id = if self.force_candidate_collision {
            MacroBodySemanticId::testing_collision()
        } else {
            semantic_id
        };
        let body_value = MacroBodyValue {
            flags: meaning.flags(),
            parameter_text: parameter_root,
            replacement_text: replacement_root,
            parameter_pattern,
        };
        let (body_value, is_new_body) =
            self.bodies
                .intern_with_status(semantic_id, body_value, MacroBodyValue::exact_eq);
        let body_identity = body_value.identity();
        let mut body = MacroBodyRef {
            value: body_value,
            patch_root: self
                .body_patch_leases
                .get(&body_identity)
                .and_then(PatchRootWeak::upgrade),
        };
        let mut domain = domain;
        if is_new_body {
            self.attach_body_patch_allocation(&mut body, domain.as_deref_mut());
        }

        let provenance_cell = OnceLock::new();
        if let Some(provenance) = provenance {
            let _ = provenance_cell.set(provenance);
        }
        let value = MacroDefinitionValue {
            body,
            provenance: provenance_cell,
            observation_operand: self.next_observation_operand,
        };
        self.next_observation_operand = self
            .next_observation_operand
            .checked_sub(i64::from(observation_width))
            .expect("macro observation operand underflow");
        let serial = self.next_definition_serial;
        self.next_definition_serial = self.next_definition_serial.wrapping_add(1);
        let _ = self.definitions.find_exact(&serial, |_| false);
        let value = self.definitions.insert_new(serial, value);
        let mut definition = MacroDefinitionRef {
            value,
            patch_root: None,
        };
        self.attach_definition_patch_allocation(&mut definition, domain);
        self.install_packed_record(
            definition.id(),
            meaning,
            &definition
                .value
                .value()
                .body
                .value
                .value()
                .parameter_pattern,
            definition
                .value
                .value()
                .body
                .value
                .value()
                .parameter_text
                .tokens(),
            definition
                .value
                .value()
                .body
                .value
                .value()
                .replacement_text
                .tokens(),
            Some((
                &definition.value.value().body.value.value().parameter_text,
                &definition.value.value().body.value.value().replacement_text,
            )),
            definition.value.value().provenance.get().cloned(),
            definition.value.value().observation_operand,
        );
        definition
    }

    #[allow(clippy::too_many_arguments)]
    fn install_packed_record(
        &mut self,
        definition: MacroDefinitionId,
        meaning: MacroMeaning,
        pattern: &MacroParameterPattern,
        parameter: &[Token],
        replacement: &[Token],
        token_roots: Option<(&TokenListRef, &TokenListRef)>,
        provenance: Option<MacroDefinitionProvenance>,
        observation_operand: i64,
    ) {
        let chunk_index = definition.raw() as usize / PACKED_MACRO_CHUNK_RECORDS;
        while self.packed_chunks.len() <= chunk_index {
            self.packed_chunks.push(Arc::new(PackedMacroChunk {
                records: vec![None; PACKED_MACRO_CHUNK_RECORDS],
                words: Vec::new(),
                token_ids: Vec::new(),
            }));
        }
        let chunk = Arc::make_mut(&mut self.packed_chunks[chunk_index]);
        let record_index = definition.raw() as usize % PACKED_MACRO_CHUNK_RECORDS;
        let existing = chunk.records[record_index].clone();
        let allocation_serial = existing
            .as_ref()
            .filter(|record| record.definition == definition)
            .map_or_else(
                || {
                    let serial = self.next_packed_serial;
                    self.next_packed_serial = self.next_packed_serial.wrapping_add(1);
                    serial
                },
                |record| record.allocation_serial,
            );
        let roots = token_roots.expect("packed macro records require live token-list roots");
        let parameter_root = retain_or_replace_token_id(
            chunk,
            record_index,
            existing.as_ref().map(|record| record.parameter_root),
            roots.0.id(),
        );
        let replacement_root = retain_or_replace_token_id(
            chunk,
            record_index,
            existing.as_ref().map(|record| record.replacement_root),
            roots.1.id(),
        );
        let parameter_origins = provenance
            .as_ref()
            .map(MacroDefinitionProvenance::parameter_ref);
        let parameter_words = || {
            parameter.iter().copied().enumerate().map(|(index, token)| {
                TracedTokenWord::pack(
                    token,
                    parameter_origins
                        .and_then(|origins| origins.root(index))
                        .map_or(crate::token::OriginId::UNKNOWN, |root| root.id()),
                )
            })
        };
        let replacement_origins = provenance
            .as_ref()
            .map(MacroDefinitionProvenance::replacement_ref);
        let replacement_words = || {
            replacement
                .iter()
                .copied()
                .enumerate()
                .map(|(index, token)| {
                    TracedTokenWord::pack(
                        token,
                        replacement_origins
                            .and_then(|origins| origins.root(index))
                            .map_or(crate::token::OriginId::UNKNOWN, |root| root.id()),
                    )
                })
        };
        let required_len = parameter.len().saturating_add(replacement.len());
        let (parameter_start, allocation_len) = if existing
            .as_ref()
            .is_some_and(|record| required_len <= record.allocation_len as usize)
        {
            let existing = existing.as_ref().expect("checked packed record");
            for (slot, word) in chunk.words[existing.parameter_start as usize
                ..existing.parameter_start as usize + parameter.len()]
                .iter_mut()
                .zip(parameter_words())
            {
                *slot = word;
            }
            let replacement_start = existing.parameter_start as usize + parameter.len();
            for (slot, word) in chunk.words
                [replacement_start..replacement_start + replacement.len()]
                .iter_mut()
                .zip(replacement_words())
            {
                *slot = word;
            }
            (existing.parameter_start, existing.allocation_len)
        } else {
            let parameter_start =
                u32::try_from(chunk.words.len()).expect("macro chunk exceeds u32");
            chunk.words.extend(parameter_words());
            chunk.words.extend(replacement_words());
            (
                parameter_start,
                u32::try_from(required_len).expect("macro text exceeds u32"),
            )
        };
        chunk.records[record_index] = Some(PackedMacroRecord {
            definition,
            flags: meaning.flags(),
            parameter_root,
            replacement_root,
            pattern: pattern.packed(),
            parameter_start,
            parameter_len: u32::try_from(parameter.len())
                .expect("macro parameter text exceeds u32"),
            allocation_len,
            replacement_len: u32::try_from(replacement.len())
                .expect("macro replacement text exceeds u32"),
            observation_operand,
            allocation_serial,
        });
    }

    #[must_use]
    pub(crate) fn packed_owner(
        &self,
        definition: MacroDefinitionId,
    ) -> Option<PackedMacroChunkOwner> {
        self.resolved_value(definition)?;
        let chunk = self
            .packed_chunks
            .get(definition.raw() as usize / PACKED_MACRO_CHUNK_RECORDS)?;
        let roots = chunk
            .records
            .iter()
            .filter_map(|record| record.as_ref())
            .filter_map(|record| {
                let value = self.resolved_value(record.definition)?;
                let body = value.value().body.value.value();
                Some(PackedMacroDefinitionRoots {
                    definition: record.definition,
                    parameter_text: body.parameter_text.clone(),
                    replacement_text: body.replacement_text.clone(),
                    provenance: value.value().provenance.get().cloned(),
                })
            })
            .collect::<Vec<_>>();
        let owner = PackedMacroChunkOwner {
            chunk: Arc::clone(chunk),
            roots: roots.into(),
        };
        owner.contains(definition).then_some(owner)
    }

    fn packed_record(&self, definition: MacroDefinitionId) -> Option<&PackedMacroRecord> {
        self.packed_chunks
            .get(definition.raw() as usize / PACKED_MACRO_CHUNK_RECORDS)?
            .records
            .get(definition.raw() as usize % PACKED_MACRO_CHUNK_RECORDS)?
            .as_ref()
            .filter(|record| record.definition == definition)
    }

    #[must_use]
    pub(crate) fn get(&self, id: MacroDefinitionId) -> MacroMeaning {
        if self.resolved_value(id).is_some()
            && let Some(record) = self.packed_record(id)
        {
            let chunk = &self.packed_chunks[id.raw() as usize / PACKED_MACRO_CHUNK_RECORDS];
            return MacroMeaning::new(
                record.flags,
                chunk.token_ids[record.parameter_root as usize],
                chunk.token_ids[record.replacement_root as usize],
            );
        }
        self.resolved_value(id)
            .expect("macro definition id is not live")
            .value()
            .body
            .value
            .value()
            .meaning()
    }

    #[must_use]
    pub(crate) fn owner(&self, id: MacroDefinitionId) -> Option<MacroDefinitionRef> {
        self.frozen_root(id).cloned().or_else(|| {
            self.definitions
                .resolve(id.identity())
                .map(|value| MacroDefinitionRef {
                    value,
                    patch_root: self
                        .definition_patch_leases
                        .get(&id)
                        .and_then(PatchRootWeak::upgrade),
                })
        })
    }

    /// Clones the owner named either by a live identity or a compact stored
    /// coordinate without first attempting the reserved stored identity.
    pub(crate) fn resolved_owner(&self, id: MacroDefinitionId) -> Option<MacroDefinitionRef> {
        if !id.is_stored() {
            return self.owner(id);
        }
        self.frozen_roots
            .get(id.raw() as usize)
            .cloned()
            .or_else(|| {
                self.definitions.resolve_slot(id.raw()).map(|value| {
                    let resolved = MacroDefinitionId::from_identity(value.identity());
                    MacroDefinitionRef {
                        value,
                        patch_root: self
                            .definition_patch_leases
                            .get(&resolved)
                            .and_then(PatchRootWeak::upgrade),
                    }
                })
            })
    }

    fn resolved_value(
        &self,
        id: MacroDefinitionId,
    ) -> Option<crate::reachable_value::ReachableValueRef<MacroDefinitionValue>> {
        if !id.is_stored() {
            if let Some(root) = self.frozen_root(id) {
                return Some(root.value.clone());
            }
            return self.definitions.resolve(id.identity());
        }
        self.frozen_roots
            .get(id.raw() as usize)
            .map(|root| root.value.clone())
            .or_else(|| self.definitions.resolve_slot(id.raw()))
    }

    fn frozen_root(&self, id: MacroDefinitionId) -> Option<&MacroDefinitionRef> {
        self.frozen_roots
            .get(id.raw() as usize)
            .filter(|root| root.id() == id)
    }

    #[must_use]
    pub(crate) fn stored_slot(&self, raw: u32) -> Option<MacroDefinitionRef> {
        self.frozen_roots.get(raw as usize).cloned().or_else(|| {
            self.definitions
                .resolve_slot(raw)
                .map(|value| MacroDefinitionRef {
                    value,
                    patch_root: None,
                })
        })
    }

    #[must_use]
    pub(crate) fn parameter_pattern(&self, id: MacroDefinitionId) -> MacroParameterPattern {
        if self.resolved_value(id).is_some()
            && let Some(record) = self.packed_record(id)
        {
            let chunk = &self.packed_chunks[id.raw() as usize / PACKED_MACRO_CHUNK_RECORDS];
            let start = record.parameter_start as usize;
            let tokens = chunk.words[start..start + record.parameter_len as usize]
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>();
            return MacroParameterPattern::from_tokens(&tokens);
        }
        self.resolved_value(id)
            .expect("macro definition id is not live")
            .value()
            .body
            .value
            .value()
            .parameter_pattern
            .clone()
    }

    #[must_use]
    pub(crate) fn provenance(&self, id: MacroDefinitionId) -> Option<MacroDefinitionProvenance> {
        self.resolved_value(id)?.value().provenance.get().cloned()
    }

    pub(crate) fn set_provenance(
        &mut self,
        id: MacroDefinitionId,
        provenance: MacroDefinitionProvenance,
    ) {
        let root = self
            .resolved_value(id)
            .expect("macro definition id is not live");
        if let Err(existing) = root.value().provenance.set(provenance.clone()) {
            assert_eq!(
                existing, provenance,
                "macro provenance changed after publication"
            );
        }
        let body = root.value().body.value.value();
        self.install_packed_record(
            id,
            body.meaning(),
            &body.parameter_pattern,
            body.parameter_text.tokens(),
            body.replacement_text.tokens(),
            Some((&body.parameter_text, &body.replacement_text)),
            Some(provenance),
            root.value().observation_operand,
        );
    }

    #[must_use]
    pub(crate) fn observation_operand(&self, id: MacroDefinitionId) -> i64 {
        if self.resolved_value(id).is_some()
            && let Some(record) = self.packed_record(id)
        {
            return record.observation_operand;
        }
        self.resolved_value(id)
            .expect("macro definition id is not live")
            .value()
            .observation_operand
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn contains(&self, id: MacroDefinitionId) -> bool {
        self.owner(id).is_some()
    }

    #[must_use]
    pub(crate) fn resolve_stored(&self, id: MacroDefinitionId) -> Option<MacroDefinitionId> {
        self.resolved_value(id)
            .map(|value| MacroDefinitionId::from_identity(value.identity()))
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> MacroStoreMark {
        MacroStoreMark {
            bodies: u32::try_from(self.bodies.slot_len()).expect("macro body slots exceed u32"),
            definitions: u32::try_from(self.definitions.slot_len())
                .expect("macro definition slots exceed u32 entries"),
            packed_serial: self.next_packed_serial,
            patch_events: u32::try_from(self.patch_order.len())
                .expect("macro patch events exceed u32 entries"),
            next_observation_operand: self.next_observation_operand,
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: MacroStoreMark) {
        if self.next_packed_serial != mark.packed_serial {
            for chunk in &mut self.packed_chunks {
                let chunk = Arc::make_mut(chunk);
                for record in &mut chunk.records {
                    if record
                        .as_ref()
                        .is_some_and(|record| record.allocation_serial >= mark.packed_serial)
                    {
                        *record = None;
                    }
                }
            }
            self.next_packed_serial = mark.packed_serial;
        }
        while self.patch_order.len() > mark.patch_events as usize {
            match self
                .patch_order
                .pop()
                .expect("macro patch order is nonempty")
            {
                PatchEvent::Body(id) => {
                    assert!(self.body_patch_handles.remove(&id).is_some());
                    assert!(self.body_patch_leases.remove(&id).is_some());
                }
                PatchEvent::Definition(id) => {
                    assert!(self.definition_patch_handles.remove(&id).is_some());
                    assert!(self.definition_patch_leases.remove(&id).is_some());
                }
            }
        }
        self.bodies
            .prioritize_reclamation_from(mark.bodies as usize);
        self.definitions
            .prioritize_reclamation_from(mark.definitions as usize);
        self.next_observation_operand = mark.next_observation_operand;
    }

    pub(crate) fn selected_patch_roots(&self, domain: &PatchAllocationDomain) -> Vec<PatchRoot> {
        self.patch_order
            .iter()
            .filter_map(|event| match *event {
                PatchEvent::Body(id) => self
                    .body_patch_handles
                    .get(&id)
                    .map(|handle| domain.root_if_typed(handle)),
                PatchEvent::Definition(id) => self
                    .definition_patch_handles
                    .get(&id)
                    .map(|handle| domain.root_if_typed(handle)),
            })
            .filter_map(|root| root.expect("typed macro root belongs to private domain"))
            .collect()
    }

    pub(crate) fn patch_allocation_count(&self) -> usize {
        self.patch_order.len()
    }

    pub(crate) fn clear_patch_allocations(&mut self) {
        self.body_patch_handles.clear();
        self.body_patch_leases.clear();
        self.definition_patch_handles.clear();
        self.definition_patch_leases.clear();
        self.patch_order.clear();
    }

    fn attach_body_patch_allocation(
        &mut self,
        root: &mut MacroBodyRef,
        domain: Option<&mut PatchAllocationDomain>,
    ) {
        let Some(domain) = domain else { return };
        let id = root.value.identity();
        let handle = domain
            .allocate_shared(root.shared(), root.value.value().logical_bytes())
            .expect("private macro-body allocation belongs to active operation");
        let lease = domain
            .install_root_lease(&handle)
            .expect("new private macro body belongs to active domain");
        assert!(self.body_patch_handles.insert(id, handle).is_none());
        assert!(
            self.body_patch_leases
                .insert(id, lease.downgrade())
                .is_none()
        );
        root.patch_root = Some(lease);
        self.patch_order.push(PatchEvent::Body(id));
    }

    fn attach_definition_patch_allocation(
        &mut self,
        root: &mut MacroDefinitionRef,
        domain: Option<&mut PatchAllocationDomain>,
    ) {
        let Some(domain) = domain else { return };
        let id = root.id();
        let handle = domain
            .allocate_shared(root.shared(), root.value.value().logical_bytes())
            .expect("private macro-definition allocation belongs to active operation");
        let lease = domain
            .install_root_lease(&handle)
            .expect("new private macro definition belongs to active domain");
        assert!(self.definition_patch_handles.insert(id, handle).is_none());
        assert!(
            self.definition_patch_leases
                .insert(id, lease.downgrade())
                .is_none()
        );
        root.patch_root = Some(lease);
        self.patch_order.push(PatchEvent::Definition(id));
    }

    #[cfg(test)]
    pub(crate) fn testing_token_roots(
        &self,
        id: MacroDefinitionId,
    ) -> (TokenListRef, TokenListRef) {
        let owner = self.owner(id).expect("macro definition id is not live");
        let body = owner.value.value().body.value.value();
        (body.parameter_text.clone(), body.replacement_text.clone())
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_live_totals(&self) -> (usize, usize, usize, usize) {
        let (bodies, body_bytes) = self
            .bodies
            .testing_live_totals(MacroBodyValue::logical_bytes);
        let (definitions, definition_bytes) = self
            .definitions
            .testing_live_totals(MacroDefinitionValue::logical_bytes);
        (bodies, body_bytes, definitions, definition_bytes)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_pool_shapes(&self) -> (PoolShape, PoolShape) {
        (
            self.bodies.testing_shape(),
            self.definitions.testing_shape(),
        )
    }

    #[cfg(test)]
    pub(crate) fn testing_packed_shape(&self) -> (usize, usize, usize, usize) {
        (
            self.packed_chunks.len(),
            self.packed_chunks
                .iter()
                .map(|chunk| chunk.records.iter().flatten().count())
                .sum(),
            self.packed_chunks
                .iter()
                .map(|chunk| chunk.words.len())
                .sum(),
            self.packed_chunks
                .iter()
                .map(|chunk| chunk.token_ids.len())
                .sum(),
        )
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_force_candidate_collision(&mut self) {
        self.force_candidate_collision = true;
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_resolved_value(
        &self,
        id: MacroDefinitionId,
    ) -> (Option<MacroMeaning>, LookupWork) {
        let mut work = LookupWork {
            fixed_root_probes: 1,
            ..LookupWork::default()
        };
        if let Some(root) = self
            .frozen_roots
            .get(id.raw() as usize)
            .filter(|root| id.is_stored() || root.id() == id)
        {
            return (Some(root.meaning()), work);
        }
        let (value, pool_work) = if id.is_stored() {
            self.definitions.testing_resolve_slot(id.raw())
        } else {
            self.definitions.testing_resolve(id.identity())
        };
        work.generation_checks += pool_work.generation_checks;
        work.slot_probes += pool_work.slot_probes;
        work.weak_upgrades += pool_work.weak_upgrades;
        let meaning = value.map(|value| value.value().body.value.value().meaning());
        (meaning, work)
    }

    #[cfg(any(test, feature = "testing"))]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn testing_body_collision_lookup(
        &self,
        meaning: MacroMeaning,
        parameter_root: TokenListRef,
        replacement_root: TokenListRef,
        parameter_pattern: MacroParameterPattern,
        parameter_semantic_id: TokenSemanticId,
        replacement_semantic_id: TokenSemanticId,
    ) -> (Option<MacroMeaning>, LookupWork) {
        let semantic_id = if self.force_candidate_collision {
            MacroBodySemanticId::testing_collision()
        } else {
            MacroBodySemanticId::new(
                meaning.flags(),
                parameter_semantic_id,
                replacement_semantic_id,
            )
        };
        let candidate = MacroBodyValue {
            flags: meaning.flags(),
            parameter_text: parameter_root,
            replacement_text: replacement_root,
            parameter_pattern,
        };
        let (found, work) = self
            .bodies
            .testing_find_exact(&semantic_id, |value| value.exact_eq(&candidate));
        (found.map(|body| body.value().meaning()), work)
    }
}

fn observation_operands(widths: &[u32]) -> Result<Vec<i64>, &'static str> {
    let mut next = 249_985_i64;
    let mut operands = Vec::with_capacity(widths.len());
    for width in widths {
        operands.push(next);
        next = next
            .checked_sub(i64::from(*width))
            .ok_or("macro observation operand underflow")?;
    }
    Ok(operands)
}

fn retain_token_id(ids: &mut Vec<TokenListId>, id: TokenListId) -> u32 {
    if let Some(index) = ids.iter().position(|candidate| *candidate == id) {
        return u32::try_from(index).expect("macro token ids exceed u32");
    }
    ids.push(id);
    u32::try_from(ids.len() - 1).expect("macro token ids exceed u32")
}

fn retain_or_replace_token_id(
    chunk: &mut PackedMacroChunk,
    record_index: usize,
    existing: Option<u32>,
    id: TokenListId,
) -> u32 {
    let Some(existing) = existing else {
        return retain_token_id(&mut chunk.token_ids, id);
    };
    if chunk.token_ids[existing as usize] == id {
        return existing;
    }
    let shared = chunk.records.iter().enumerate().any(|(index, record)| {
        index != record_index
            && record.as_ref().is_some_and(|record| {
                record.parameter_root == existing || record.replacement_root == existing
            })
    });
    if shared {
        retain_token_id(&mut chunk.token_ids, id)
    } else {
        chunk.token_ids[existing as usize] = id;
        existing
    }
}

#[cfg(test)]
mod tests;
