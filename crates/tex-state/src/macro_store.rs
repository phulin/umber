//! Immutable macro-definition storage.
//!
//! Macro meanings keep one compact operand in Env. The operand names a frozen
//! macro definition here, and the definition names separately frozen parameter
//! text and replacement-body token lists.

use crate::ids::{MacroDefinitionId, OriginListId, TokenListId};
use crate::meaning::MeaningFlags;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Public macro meaning aggregate used at the Universe boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroMeaning {
    flags: MeaningFlags,
    parameter_text: TokenListId,
    parameter_origins: OriginListId,
    replacement_text: TokenListId,
    replacement_origins: OriginListId,
}

impl MacroMeaning {
    /// Creates a macro meaning over already-frozen token lists.
    #[must_use]
    pub const fn new(
        flags: MeaningFlags,
        parameter_text: TokenListId,
        replacement_text: TokenListId,
    ) -> Self {
        Self::with_origins(
            flags,
            parameter_text,
            OriginListId::EMPTY,
            replacement_text,
            OriginListId::EMPTY,
        )
    }

    /// Creates a macro meaning over frozen token lists and their origin lists.
    #[must_use]
    pub const fn with_origins(
        flags: MeaningFlags,
        parameter_text: TokenListId,
        parameter_origins: OriginListId,
        replacement_text: TokenListId,
        replacement_origins: OriginListId,
    ) -> Self {
        Self {
            flags,
            parameter_text,
            parameter_origins,
            replacement_text,
            replacement_origins,
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
    pub const fn parameter_origins(self) -> OriginListId {
        self.parameter_origins
    }

    #[must_use]
    pub const fn replacement_text(self) -> TokenListId {
        self.replacement_text
    }

    #[must_use]
    pub const fn replacement_origins(self) -> OriginListId {
        self.replacement_origins
    }

    #[must_use]
    pub const fn semantic_eq(self, other: Self) -> bool {
        self.flags.bits() == other.flags.bits()
            && self.parameter_text.raw() == other.parameter_text.raw()
            && self.replacement_text.raw() == other.replacement_text.raw()
    }
}

/// A rollback watermark for the macro-definition store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MacroStoreMark {
    definitions: u32,
}

/// Hash-consed immutable macro-definition table.
#[derive(Clone, Debug)]
pub struct MacroStore {
    definitions: Vec<MacroMeaning>,
    index: HashMap<u64, Vec<MacroDefinitionId>>,
    index_dirty: bool,
}

impl MacroStore {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            definitions: Vec::new(),
            index: HashMap::new(),
            index_dirty: false,
        }
    }

    pub(crate) fn intern(&mut self, meaning: MacroMeaning) -> MacroDefinitionId {
        if self.index_dirty {
            self.rebuild_index();
        }

        let hash = content_hash(meaning);
        if let Some(candidates) = self.index.get(&hash) {
            for &id in candidates {
                if self.get(id) == meaning {
                    return id;
                }
            }
        }

        let id = MacroDefinitionId::new(u32_len(
            self.definitions.len(),
            "macro definition table exceeds u32 entries",
        ));
        self.definitions.push(meaning);
        self.index.entry(hash).or_default().push(id);
        id
    }

    #[must_use]
    pub(crate) fn get(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.definitions
            .get(id.raw() as usize)
            .copied()
            .expect("macro definition id is not live")
    }

    #[must_use]
    pub(crate) fn contains(&self, id: MacroDefinitionId) -> bool {
        (id.raw() as usize) < self.definitions.len()
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> MacroStoreMark {
        MacroStoreMark {
            definitions: u32_len(
                self.definitions.len(),
                "macro definition table exceeds u32 entries",
            ),
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: MacroStoreMark) {
        let definitions = mark.definitions as usize;
        assert!(
            definitions <= self.definitions.len(),
            "macro-store mark has too many definitions"
        );
        self.definitions.truncate(definitions);
        self.index_dirty = true;
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for raw in 0..self.definitions.len() {
            let id =
                MacroDefinitionId::new(u32_len(raw, "macro definition table exceeds u32 entries"));
            let hash = content_hash(self.get(id));
            self.index.entry(hash).or_default().push(id);
        }
        self.index_dirty = false;
    }
}

fn content_hash(meaning: MacroMeaning) -> u64 {
    let mut hasher = DefaultHasher::new();
    meaning.hash(&mut hasher);
    hasher.finish()
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}
