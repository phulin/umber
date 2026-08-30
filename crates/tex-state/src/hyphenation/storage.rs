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

#[derive(Clone, Debug)]
pub(super) enum HyphenationInverse {
    Exception {
        language: u8,
        word: String,
        value: Option<Vec<usize>>,
        occupied: usize,
    },
    Codes {
        language: u8,
        value: Option<BTreeMap<char, char>>,
    },
    ExceptionCapacity(usize),
    TrieCapacity(usize),
}

pub(crate) struct HyphenationCandidate {
    candidate_mark: usize,
    accepted_journal: Vec<HyphenationInverse>,
    accepted_patterns: PatternOwner,
    accepted_pattern_retained_bytes: usize,
    accepted_trie_capacity: usize,
    accepted_identity: Option<crate::state_hash::SemanticMapIdentity>,
}

/// Checkpoint root for hyphenation's immutable and mutable ownership families.
///
/// Once initialized, cloning `patterns` aliases one coarse owner. The runtime
/// value remains separate so exact rollback and fork isolation cannot mutate
/// that owner or make its cost depend on trie size.
#[derive(Clone)]
pub(crate) struct HyphenationCheckpoint {
    patterns: PatternOwner,
    pattern_retained_bytes: usize,
    journal: usize,
    exception_capacity: usize,
    trie_capacity: usize,
    reachable_state_identity: Option<crate::state_hash::SemanticMapIdentity>,
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
            pattern_retained_bytes: self.pattern_retained_bytes,
            runtime: self.runtime.clone(),
            journal: self.journal.clone(),
            // This is only a derived lookup cache. A checkpoint owns semantic
            // roots, not a duplicate of warmed dependency evidence.
            dependency_fingerprints: OnceLock::new(),
            trie_capacity: self.trie_capacity,
            reachable_state_identity: self.reachable_state_identity,
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
            pattern_retained_bytes: self.pattern_retained_bytes,
            journal: self.journal.len(),
            exception_capacity: self.runtime.exception_capacity,
            trie_capacity: self.trie_capacity,
            reachable_state_identity: self.reachable_state_identity,
        }
    }

    pub(crate) fn validates_checkpoint(&self, checkpoint: &HyphenationCheckpoint) -> bool {
        checkpoint.journal <= self.journal.len()
    }

    pub(crate) fn restore_checkpoint(&mut self, checkpoint: &HyphenationCheckpoint) {
        assert!(self.validates_checkpoint(checkpoint));
        while self.journal.len() > checkpoint.journal {
            let mut inverse = self.journal.pop().expect("checked hyphenation suffix");
            inverse.swap(&mut self.runtime, &mut self.trie_capacity);
        }
        self.patterns = checkpoint.patterns.clone();
        self.pattern_retained_bytes = checkpoint.pattern_retained_bytes;
        self.runtime.exception_capacity = checkpoint.exception_capacity;
        self.dependency_fingerprints = OnceLock::new();
        self.trie_capacity = checkpoint.trie_capacity;
        self.reachable_state_identity = checkpoint.reachable_state_identity;
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        checkpoint: &HyphenationCheckpoint,
    ) -> HyphenationCandidate {
        assert!(self.validates_checkpoint(checkpoint));
        let mut accepted_journal = self.journal.split_off(checkpoint.journal);
        for inverse in accepted_journal.iter_mut().rev() {
            inverse.swap(&mut self.runtime, &mut self.trie_capacity);
        }
        let accepted_patterns = std::mem::replace(&mut self.patterns, checkpoint.patterns.clone());
        let accepted_pattern_retained_bytes = std::mem::replace(
            &mut self.pattern_retained_bytes,
            checkpoint.pattern_retained_bytes,
        );
        let accepted_trie_capacity =
            std::mem::replace(&mut self.trie_capacity, checkpoint.trie_capacity);
        self.runtime.exception_capacity = checkpoint.exception_capacity;
        let accepted_identity = std::mem::replace(
            &mut self.reachable_state_identity,
            checkpoint.reachable_state_identity,
        );
        self.dependency_fingerprints = OnceLock::new();
        HyphenationCandidate {
            candidate_mark: checkpoint.journal,
            accepted_journal,
            accepted_patterns,
            accepted_pattern_retained_bytes,
            accepted_trie_capacity,
            accepted_identity,
        }
    }

    pub(crate) fn reject_checkpoint_candidate(&mut self, mut candidate: HyphenationCandidate) {
        while self.journal.len() > candidate.candidate_mark {
            let mut inverse = self.journal.pop().expect("candidate hyphenation suffix");
            inverse.swap(&mut self.runtime, &mut self.trie_capacity);
        }
        for inverse in &mut candidate.accepted_journal {
            inverse.swap(&mut self.runtime, &mut self.trie_capacity);
        }
        self.journal.append(&mut candidate.accepted_journal);
        self.patterns = candidate.accepted_patterns;
        self.pattern_retained_bytes = candidate.accepted_pattern_retained_bytes;
        self.trie_capacity = candidate.accepted_trie_capacity;
        self.reachable_state_identity = candidate.accepted_identity;
        self.dependency_fingerprints = OnceLock::new();
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self, _candidate: HyphenationCandidate) {}
}

impl HyphenationInverse {
    pub(super) fn swap(&mut self, runtime: &mut HyphenationRuntime, trie_capacity: &mut usize) {
        match self {
            Self::Exception {
                language,
                word,
                value,
                occupied,
            } => {
                let language_map = runtime.exceptions.entry(*language).or_default();
                let current = match value.take() {
                    Some(prior) => language_map.insert(word.clone(), prior),
                    None => language_map.remove(word),
                };
                *value = current;
                if language_map.is_empty() {
                    runtime.exceptions.remove(language);
                }
                std::mem::swap(&mut runtime.exception_occupied, occupied);
            }
            Self::Codes { language, value } => {
                let current = match value.take() {
                    Some(prior) => runtime.hyphen_codes.insert(*language, prior),
                    None => runtime.hyphen_codes.remove(language),
                };
                *value = current;
            }
            Self::ExceptionCapacity(value) => {
                std::mem::swap(&mut runtime.exception_capacity, value)
            }
            Self::TrieCapacity(value) => std::mem::swap(trie_capacity, value),
        }
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
        let pattern_retained_bytes = patterns
            .values()
            .map(|nodes| {
                std::mem::size_of::<(u8, Vec<TrieNode>)>()
                    .saturating_add(nodes.len().saturating_mul(std::mem::size_of::<TrieNode>()))
                    .saturating_add(
                        nodes
                            .iter()
                            .map(|node| {
                                node.edges
                                    .len()
                                    .saturating_mul(std::mem::size_of::<(char, usize)>())
                                    .saturating_add(node.values.len())
                            })
                            .sum::<usize>(),
                    )
            })
            .sum();
        Ok(Self {
            patterns: PatternOwner::Initialized(Arc::new(patterns)),
            pattern_retained_bytes,
            runtime: HyphenationRuntime {
                exceptions,
                hyphen_codes: rows.hyphen_codes,
                exception_occupied: rows.exception_occupied,
                exception_capacity: rows.exception_capacity,
            },
            journal: Vec::new(),
            dependency_fingerprints: OnceLock::new(),
            trie_capacity: default_trie_capacity(),
            reachable_state_identity: None,
        })
    }
}
