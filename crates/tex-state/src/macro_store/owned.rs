use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use crate::ContentHash;
use crate::ids::MacroDefinitionId;
use crate::meaning::MeaningFlags;
use crate::patch_domain::PatchRootLease;
#[cfg(any(test, feature = "testing"))]
use crate::reachable_value::ReachableValuePool;
use crate::reachable_value::ReachableValueRef;
use crate::state_hash::StateHasher;
use crate::token_store::{TokenListRef, TokenSemanticId};

use super::{
    MacroDefinitionProvenance, MacroMeaning, MacroParameterPattern, PackedMacroChunkOwner,
};

const MACRO_BODY_ID_DOMAIN: u64 = 0x6d61_6372_6f5f_626f;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MacroBodySemanticId {
    fingerprint: u64,
    identity: ContentHash,
}

impl Hash for MacroBodySemanticId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.fingerprint);
    }
}

impl MacroBodySemanticId {
    pub(super) fn new(
        flags: MeaningFlags,
        parameter: TokenSemanticId,
        replacement: TokenSemanticId,
    ) -> Self {
        let mut hasher = StateHasher::new(MACRO_BODY_ID_DOMAIN);
        hasher.u8(1);
        hasher.u8(flags.bits());
        parameter.apply(&mut hasher);
        replacement.apply(&mut hasher);
        let fragment = hasher.finish_fragment();
        Self {
            fingerprint: fragment.fingerprint(),
            identity: fragment.identity(),
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub(super) fn testing_collision() -> Self {
        Self {
            fingerprint: 0,
            identity: crate::state_hash::semantic_identity_bytes(b"macro-body-test-collision", &[]),
        }
    }
}

#[derive(Debug)]
pub(super) struct MacroBodyValue {
    pub(super) flags: MeaningFlags,
    pub(super) parameter_text: TokenListRef,
    pub(super) replacement_text: TokenListRef,
    pub(super) parameter_pattern: MacroParameterPattern,
}

impl MacroBodyValue {
    pub(super) fn meaning(&self) -> MacroMeaning {
        MacroMeaning::new(
            self.flags,
            self.parameter_text.id(),
            self.replacement_text.id(),
        )
    }

    pub(super) fn exact_eq(&self, other: &Self) -> bool {
        self.flags == other.flags
            && self.parameter_text.tokens() == other.parameter_text.tokens()
            && self.replacement_text.tokens() == other.replacement_text.tokens()
    }

    pub(super) fn logical_bytes(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

#[derive(Clone, Debug)]
pub(super) struct MacroBodyRef {
    pub(super) value: ReachableValueRef<MacroBodyValue>,
    pub(super) patch_root: Option<PatchRootLease>,
}

impl MacroBodyRef {
    pub(super) fn shared(&self) -> Arc<MacroBodyValue> {
        self.value.shared()
    }
}

#[derive(Debug)]
pub(super) struct MacroDefinitionValue {
    pub(super) body: MacroBodyRef,
    pub(super) provenance: OnceLock<MacroDefinitionProvenance>,
    pub(super) observation_operand: i64,
}

impl MacroDefinitionValue {
    pub(super) fn logical_bytes(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

/// One strong definition occurrence, paired with its compact coordinate.
#[derive(Clone, Debug)]
pub struct MacroDefinitionRef {
    pub(super) value: Option<ReachableValueRef<MacroDefinitionValue>>,
    pub(super) packed: Option<(MacroDefinitionId, PackedMacroChunkOwner, Arc<()>)>,
    pub(super) patch_root: Option<PatchRootLease>,
}

impl MacroDefinitionRef {
    /// Creates an isolated definition owner for downstream state-machine
    /// tests that do not own a Universe.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_new(raw: u32) -> Self {
        let empty = crate::token_store::testing_empty_token_list_ref();
        let body_id = MacroBodySemanticId::new(
            MeaningFlags::EMPTY,
            empty.semantic_id(),
            empty.semantic_id(),
        );
        let (mut bodies, _) = ReachableValuePool::from_fixed_values(Vec::new(), 0);
        let body = MacroBodyRef {
            value: bodies.insert_new(
                body_id,
                MacroBodyValue {
                    flags: MeaningFlags::EMPTY,
                    parameter_text: empty.clone(),
                    replacement_text: empty,
                    parameter_pattern: MacroParameterPattern::from_tokens(&[]),
                },
            ),
            patch_root: None,
        };
        let values = (0..=raw)
            .map(|index| MacroDefinitionValue {
                body: body.clone(),
                provenance: OnceLock::new(),
                observation_operand: i64::from(index),
            })
            .collect();
        let (_, roots) =
            ReachableValuePool::<u64, MacroDefinitionValue>::from_fixed_values(values, 0);
        Self {
            value: Some(roots[raw as usize].clone()),
            packed: None,
            patch_root: None,
        }
    }

    pub(super) fn packed(
        id: MacroDefinitionId,
        owner: PackedMacroChunkOwner,
    ) -> Self {
        debug_assert!(owner.contains(id));
        let liveness = owner
            .definition_liveness(id)
            .expect("packed macro definition has no liveness root");
        Self {
            value: None,
            packed: Some((id, owner, liveness)),
            patch_root: None,
        }
    }

    pub(super) fn exact_value(&self) -> &ReachableValueRef<MacroDefinitionValue> {
        self.value
            .as_ref()
            .expect("arena-backed macro definition has no exact value")
    }

    #[must_use]
    pub fn id(&self) -> MacroDefinitionId {
        self.packed.as_ref().map_or_else(
            || MacroDefinitionId::from_identity(self.exact_value().identity()),
            |(id, _, _)| *id,
        )
    }

    /// Returns the compact slot coordinate carried by this owner.
    #[must_use]
    pub fn raw(&self) -> u32 {
        self.id().raw()
    }

    #[must_use]
    pub fn meaning(&self) -> MacroMeaning {
        self.packed.as_ref().map_or_else(
            || self.exact_value().value().body.value.value().meaning(),
            |(id, owner, _)| {
                owner
                    .meaning(*id)
                    .expect("packed macro owner lost its definition")
            },
        )
    }

    pub(super) fn shared(&self) -> Arc<MacroDefinitionValue> {
        self.exact_value().shared()
    }

    #[cfg(test)]
    pub(crate) fn strong_count(&self) -> usize {
        self.exact_value().strong_count()
    }

    #[cfg(test)]
    pub(crate) fn body_ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(
            &self.exact_value().value().body.shared(),
            &other.exact_value().value().body.shared(),
        )
    }
}

impl PartialEq for MacroDefinitionRef {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for MacroDefinitionRef {}

impl Hash for MacroDefinitionRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}
