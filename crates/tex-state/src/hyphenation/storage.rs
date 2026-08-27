use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use super::{
    HyphenationTable, LanguageHyphenation, TrieNode, default_exception_capacity,
    default_trie_capacity,
};

/// The pattern builder moves into exactly one coarse immutable owner when
/// TeX82 §919/§1335 initializes the trie. Checkpoints clone only that owner's
/// small handle; no trie node or edge is copied after initialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PatternOwner {
    Building(BTreeMap<u8, Vec<TrieNode>>),
    Initialized(Arc<BTreeMap<u8, Vec<TrieNode>>>),
}

impl PatternOwner {
    pub(super) fn languages(&self) -> &BTreeMap<u8, Vec<TrieNode>> {
        match self {
            Self::Building(languages) => languages,
            Self::Initialized(languages) => languages,
        }
    }

    pub(super) fn building_languages(&mut self) -> &mut BTreeMap<u8, Vec<TrieNode>> {
        match self {
            Self::Building(languages) => languages,
            Self::Initialized(_) => panic!("hyphenation patterns are already initialized"),
        }
    }

    pub(super) const fn is_building(&self) -> bool {
        matches!(self, Self::Building(_))
    }

    pub(super) fn initialize(&mut self) {
        if matches!(self, Self::Initialized(_)) {
            return;
        }
        let Self::Building(languages) = std::mem::replace(self, Self::Building(BTreeMap::new()))
        else {
            unreachable!("initialized patterns returned above");
        };
        *self = Self::Initialized(Arc::new(languages));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HyphenationRuntime {
    pub(super) exceptions: BTreeMap<u8, BTreeMap<String, Vec<usize>>>,
    pub(super) hyphen_codes: BTreeMap<u8, BTreeMap<char, char>>,
    /// TeX82 §1334's `hyph_count` and §934's configured `hyph_size`.
    pub(super) exception_occupied: usize,
    pub(super) exception_capacity: usize,
}

/// Checkpoint root for hyphenation's immutable and mutable ownership families.
///
/// Once initialized, cloning `patterns` aliases one coarse owner. The runtime
/// value remains separate so exact rollback and fork isolation cannot mutate
/// that owner or make its cost depend on trie size.
#[derive(Clone)]
pub(crate) struct HyphenationCheckpoint {
    patterns: PatternOwner,
    runtime: HyphenationRuntime,
    trie_capacity: usize,
}

impl Default for HyphenationRuntime {
    fn default() -> Self {
        Self {
            exceptions: BTreeMap::new(),
            hyphen_codes: BTreeMap::new(),
            exception_occupied: 0,
            exception_capacity: default_exception_capacity(),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct HyphenationFormatRows {
    languages: BTreeMap<u8, LanguageHyphenation>,
    hyphen_codes: BTreeMap<u8, BTreeMap<char, char>>,
    exception_occupied: usize,
    #[serde(default = "default_exception_capacity")]
    exception_capacity: usize,
}

impl Clone for HyphenationTable {
    fn clone(&self) -> Self {
        Self {
            patterns: self.patterns.clone(),
            runtime: self.runtime.clone(),
            // This is only a derived lookup cache. A checkpoint owns semantic
            // roots, not a duplicate of warmed dependency evidence.
            dependency_fingerprints: OnceLock::new(),
            trie_capacity: self.trie_capacity,
        }
    }
}

impl Default for HyphenationTable {
    fn default() -> Self {
        Self::new()
    }
}

impl HyphenationTable {
    pub(crate) fn checkpoint(&self) -> HyphenationCheckpoint {
        HyphenationCheckpoint {
            patterns: self.patterns.clone(),
            runtime: self.runtime.clone(),
            trie_capacity: self.trie_capacity,
        }
    }

    pub(crate) fn from_checkpoint(checkpoint: &HyphenationCheckpoint) -> Self {
        Self {
            patterns: checkpoint.patterns.clone(),
            runtime: checkpoint.runtime.clone(),
            dependency_fingerprints: OnceLock::new(),
            trie_capacity: checkpoint.trie_capacity,
        }
    }

    pub(crate) fn restore_checkpoint(&mut self, checkpoint: &HyphenationCheckpoint) {
        self.patterns = checkpoint.patterns.clone();
        self.runtime = checkpoint.runtime.clone();
        self.dependency_fingerprints = OnceLock::new();
        self.trie_capacity = checkpoint.trie_capacity;
    }
}

impl Serialize for HyphenationTable {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut languages = self
            .patterns
            .languages()
            .iter()
            .map(|(&language, nodes)| {
                (
                    language,
                    LanguageHyphenation {
                        nodes: nodes.clone(),
                        exceptions: BTreeMap::new(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        for (&language, exceptions) in &self.runtime.exceptions {
            languages.entry(language).or_default().exceptions = exceptions.clone();
        }
        HyphenationFormatRows {
            languages,
            hyphen_codes: self.runtime.hyphen_codes.clone(),
            exception_occupied: self.runtime.exception_occupied,
            exception_capacity: self.runtime.exception_capacity,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HyphenationTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let rows = HyphenationFormatRows::deserialize(deserializer)?;
        let mut patterns = BTreeMap::new();
        let mut exceptions = BTreeMap::new();
        for (language, table) in rows.languages {
            patterns.insert(language, table.nodes);
            if !table.exceptions.is_empty() {
                exceptions.insert(language, table.exceptions);
            }
        }
        Ok(Self {
            patterns: PatternOwner::Initialized(Arc::new(patterns)),
            runtime: HyphenationRuntime {
                exceptions,
                hyphen_codes: rows.hyphen_codes,
                exception_occupied: rows.exception_occupied,
                exception_capacity: rows.exception_capacity,
            },
            dependency_fingerprints: OnceLock::new(),
            trie_capacity: default_trie_capacity(),
        })
    }
}
