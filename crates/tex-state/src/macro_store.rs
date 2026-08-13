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
use crate::reachable_value::ReachableValuePool;
use crate::token::{OriginId, Token};
use crate::token_store::{TokenListRef, TokenSemanticId};

const MACRO_PARAMETER_SLOTS: usize = 9;
mod owned;

pub use owned::MacroDefinitionRef;
use owned::{
    MacroBodyRef, MacroBodySemanticId, MacroBodyValue, MacroDefinitionProvenanceRoots,
    MacroDefinitionValue,
};

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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroDefinitionProvenance {
    definition_origin: OriginId,
    parameter_origins: OriginListId,
    replacement_origins: OriginListId,
}

impl MacroDefinitionProvenance {
    #[must_use]
    pub const fn new(
        definition_origin: OriginId,
        parameter_origins: OriginListId,
        replacement_origins: OriginListId,
    ) -> Self {
        Self {
            definition_origin,
            parameter_origins,
            replacement_origins,
        }
    }

    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            definition_origin: OriginId::UNKNOWN,
            parameter_origins: OriginListId::EMPTY,
            replacement_origins: OriginListId::EMPTY,
        }
    }

    #[must_use]
    pub const fn definition_origin(self) -> OriginId {
        self.definition_origin
    }

    #[must_use]
    pub const fn parameter_origins(self) -> OriginListId {
        self.parameter_origins
    }

    #[must_use]
    pub const fn replacement_origins(self) -> OriginListId {
        self.replacement_origins
    }
}

/// Rollback state for private macro allocations and compatibility operands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MacroStoreMark {
    pub(crate) definitions: u32,
    patch_events: u32,
    next_observation_operand: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PatchEvent {
    Body(HandleIdentity),
    Definition(MacroDefinitionId),
}

#[cfg(test)]
type PoolShape = (usize, usize, usize, usize, usize, usize);

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
    #[cfg(test)]
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
            #[cfg(test)]
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
            #[cfg(test)]
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
                provenance_roots: OnceLock::new(),
                observation_operand: operand,
            });
        }
        let (definitions, roots) = ReachableValuePool::from_fixed_values(values, 0);
        Ok(Self {
            bodies,
            definitions,
            frozen_roots: roots
                .into_iter()
                .map(|value| MacroDefinitionRef {
                    value,
                    patch_root: None,
                })
                .collect::<Vec<_>>()
                .into(),
            next_definition_serial: len as u64,
            next_observation_operand,
            body_patch_handles: HashMap::new(),
            body_patch_leases: HashMap::new(),
            definition_patch_handles: HashMap::new(),
            definition_patch_leases: HashMap::new(),
            patch_order: Vec::new(),
            #[cfg(test)]
            force_candidate_collision: false,
        })
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
        #[cfg(test)]
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
            provenance_roots: OnceLock::new(),
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
        definition
    }

    #[must_use]
    pub(crate) fn get(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.owner(id)
            .expect("macro definition id is not live")
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
        self.owner(id)
            .expect("macro definition id is not live")
            .value
            .value()
            .body
            .value
            .value()
            .parameter_pattern
            .clone()
    }

    #[must_use]
    pub(crate) fn provenance(&self, id: MacroDefinitionId) -> Option<MacroDefinitionProvenance> {
        self.owner(id)?.value.value().provenance.get().copied()
    }

    pub(crate) fn set_provenance(
        &mut self,
        id: MacroDefinitionId,
        provenance: MacroDefinitionProvenance,
    ) {
        let root = self.owner(id).expect("macro definition id is not live");
        if let Err(existing) = root.value.value().provenance.set(provenance) {
            assert_eq!(
                existing, provenance,
                "macro provenance changed after publication"
            );
        }
    }

    pub(crate) fn set_provenance_roots(
        &mut self,
        id: MacroDefinitionId,
        definition: crate::provenance::OriginRef,
        parameters: crate::provenance::OriginListRef,
        replacement: crate::provenance::OriginListRef,
    ) {
        let root = self.owner(id).expect("macro definition id is not live");
        let roots = MacroDefinitionProvenanceRoots {
            definition,
            parameters,
            replacement,
        };
        if root.value.value().provenance_roots.set(roots).is_err() {
            panic!("macro provenance roots changed after publication");
        }
    }

    pub(crate) fn provenance_roots(
        &self,
        id: MacroDefinitionId,
    ) -> Option<(
        crate::provenance::OriginRef,
        crate::provenance::OriginListRef,
        crate::provenance::OriginListRef,
    )> {
        let root = self.owner(id)?;
        let roots = root.value.value().provenance_roots.get()?;
        Some((
            roots.definition.clone(),
            roots.parameters.clone(),
            roots.replacement.clone(),
        ))
    }

    #[must_use]
    pub(crate) fn observation_operand(&self, id: MacroDefinitionId) -> i64 {
        self.owner(id)
            .expect("macro definition id is not live")
            .value
            .value()
            .observation_operand
    }

    #[must_use]
    pub(crate) fn contains(&self, id: MacroDefinitionId) -> bool {
        self.owner(id).is_some()
    }

    #[must_use]
    pub(crate) fn resolve_stored(&self, id: MacroDefinitionId) -> Option<MacroDefinitionId> {
        if self.contains(id) {
            return Some(id);
        }
        if !id.is_stored() {
            return None;
        }
        self.stored_slot(id.raw()).map(|root| root.id())
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> MacroStoreMark {
        MacroStoreMark {
            definitions: u32::try_from(self.definitions.slot_len())
                .expect("macro definition slots exceed u32 entries"),
            patch_events: u32::try_from(self.patch_order.len())
                .expect("macro patch events exceed u32 entries"),
            next_observation_operand: self.next_observation_operand,
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: MacroStoreMark) {
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

    #[cfg(test)]
    pub(crate) fn testing_live_totals(&self) -> (usize, usize, usize, usize) {
        let (bodies, body_bytes) = self
            .bodies
            .testing_live_totals(MacroBodyValue::logical_bytes);
        let (definitions, definition_bytes) = self
            .definitions
            .testing_live_totals(MacroDefinitionValue::logical_bytes);
        (bodies, body_bytes, definitions, definition_bytes)
    }

    #[cfg(test)]
    pub(crate) fn testing_pool_shapes(&self) -> (PoolShape, PoolShape) {
        (
            self.bodies.testing_shape(),
            self.definitions.testing_shape(),
        )
    }

    #[cfg(test)]
    pub(crate) fn testing_force_candidate_collision(&mut self) {
        self.force_candidate_collision = true;
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

#[cfg(test)]
mod tests;
