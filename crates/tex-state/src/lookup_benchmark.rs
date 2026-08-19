//! Focused lookup fixtures for reachability-owned token lists and macro bodies.
//!
//! This module is available only to crate tests and the standalone benchmark's
//! `testing` feature. It exposes semantic cases, not raw store mutation APIs.

use crate::ids::{MacroDefinitionId, TokenListId};
use crate::macro_store::{MacroDefinitionRef, MacroMeaning, MacroParameterPattern, MacroStore};
use crate::meaning::MeaningFlags;
use crate::token::{Catcode, Token};
use crate::token_store::{TokenListRef, TokenSemanticId, TokenStore};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupFamily {
    TokenList,
    MacroBody,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupCase {
    LiveHit,
    RetiredRegionSlot,
    GenerationMismatch,
    FormatHole,
    CollisionSafe,
}

impl LookupCase {
    pub const ALL: [Self; 5] = [
        Self::LiveHit,
        Self::RetiredRegionSlot,
        Self::GenerationMismatch,
        Self::FormatHole,
        Self::CollisionSafe,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LiveHit => "live_hit",
            Self::RetiredRegionSlot => "retired_region_slot",
            Self::GenerationMismatch => "generation_mismatch",
            Self::FormatHole => "format_hole",
            Self::CollisionSafe => "collision_safe",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LookupMeasurement {
    pub found: bool,
    pub coordinate: u32,
    pub work: usize,
}

struct TokenCase {
    store: TokenStore,
    query: TokenListId,
    expected: Option<TokenListId>,
    collision_tokens: Option<[Token; 1]>,
    collision_key: Option<TokenSemanticId>,
    _roots: Vec<TokenListRef>,
}

impl TokenCase {
    fn measure(&self) -> LookupMeasurement {
        let (root, work) = if let (Some(tokens), Some(key)) =
            (self.collision_tokens.as_ref(), self.collision_key)
        {
            self.store.testing_collision_lookup(tokens, key)
        } else {
            self.store.testing_resolved_owner(self.query)
        };
        let coordinate = root
            .as_ref()
            .map(|root| root.id().raw())
            .unwrap_or(u32::MAX);
        LookupMeasurement {
            found: root.as_ref().map(TokenListRef::id) == self.expected,
            coordinate,
            work: work.total(),
        }
    }
}

struct MacroCase {
    store: MacroStore,
    query: MacroDefinitionId,
    expected: Option<MacroDefinitionId>,
    collision: Option<MacroCollisionQuery>,
    _roots: Vec<MacroDefinitionRef>,
}

struct MacroCollisionQuery {
    meaning: MacroMeaning,
    parameter_root: TokenListRef,
    replacement_root: TokenListRef,
    parameter_semantic_id: TokenSemanticId,
    replacement_semantic_id: TokenSemanticId,
}

impl MacroCase {
    fn measure(&self) -> LookupMeasurement {
        let (found, coordinate, work) = if let Some(query) = &self.collision {
            let (meaning, work) = self.store.testing_body_collision_lookup(
                query.meaning,
                query.parameter_root.clone(),
                query.replacement_root.clone(),
                MacroParameterPattern::from_tokens(query.parameter_root.tokens()),
                query.parameter_semantic_id,
                query.replacement_semantic_id,
            );
            (
                meaning.is_some_and(|meaning| meaning.semantic_eq(query.meaning)),
                self.query.raw(),
                work,
            )
        } else {
            let (meaning, work) = self.store.testing_resolved_value(self.query);
            let coordinate = meaning.map_or(u32::MAX, |_| self.query.raw());
            (
                meaning.is_some() == self.expected.is_some(),
                coordinate,
                work,
            )
        };
        LookupMeasurement {
            found,
            coordinate,
            work: work.total(),
        }
    }
}

/// Prepared cases used by both Criterion and the deterministic regression gate.
pub struct ReachabilityLookupBenchmark {
    tokens: [TokenCase; 5],
    macros: [MacroCase; 5],
}

impl ReachabilityLookupBenchmark {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tokens: token_cases(),
            macros: macro_cases(),
        }
    }

    #[must_use]
    pub fn measure(&self, family: LookupFamily, case: LookupCase) -> LookupMeasurement {
        let index = case as usize;
        match family {
            LookupFamily::TokenList => self.tokens[index].measure(),
            LookupFamily::MacroBody => self.macros[index].measure(),
        }
    }
}

impl Default for ReachabilityLookupBenchmark {
    fn default() -> Self {
        Self::new()
    }
}

fn token_cases() -> [TokenCase; 5] {
    let mut live_store = TokenStore::new();
    let live = live_store.testing_owned(&[char_token('l')], TokenSemanticId::testing(1), None);
    let live_id = live.id();

    let mut dead_store = TokenStore::new();
    let dead_mark = dead_store.watermark();
    let dead = dead_store.testing_owned(&[char_token('d')], TokenSemanticId::testing(2), None);
    let dead_id = dead.id();
    drop(dead);
    dead_store.truncate_to(dead_mark);

    let mut stale_store = TokenStore::new();
    let stale = stale_store.testing_owned(&[char_token('s')], TokenSemanticId::testing(3), None);
    let stale_id = stale.id();
    drop(stale);
    let replacement =
        stale_store.testing_owned(&[char_token('r')], TokenSemanticId::testing(4), None);
    assert_eq!(stale_id.raw(), replacement.id().raw());
    assert_ne!(stale_id, replacement.id());

    let mut hole_store = TokenStore::new();
    let hole_mark = hole_store.watermark();
    let hole = hole_store.testing_owned(&[char_token('h')], TokenSemanticId::testing(5), None);
    let hole_id = hole.id();
    drop(hole);
    hole_store.truncate_to(hole_mark);

    let mut collision_store = TokenStore::new();
    let collision_key = TokenSemanticId::testing(6);
    let left = collision_store.testing_owned(&[char_token('a')], collision_key, None);
    let right = collision_store.testing_owned(&[char_token('b')], collision_key, None);
    let right_id = right.id();

    [
        TokenCase {
            store: live_store,
            query: live_id,
            expected: Some(live_id),
            collision_tokens: None,
            collision_key: None,
            _roots: vec![live],
        },
        TokenCase {
            store: dead_store,
            query: dead_id,
            expected: None,
            collision_tokens: None,
            collision_key: None,
            _roots: Vec::new(),
        },
        TokenCase {
            store: stale_store,
            query: stale_id,
            expected: None,
            collision_tokens: None,
            collision_key: None,
            _roots: vec![replacement],
        },
        TokenCase {
            store: hole_store,
            query: TokenListId::testing_new(hole_id.raw()),
            expected: None,
            collision_tokens: None,
            collision_key: None,
            _roots: Vec::new(),
        },
        TokenCase {
            store: collision_store,
            query: right_id,
            expected: Some(right_id),
            collision_tokens: Some([char_token('b')]),
            collision_key: Some(collision_key),
            _roots: vec![left, right],
        },
    ]
}

fn macro_cases() -> [MacroCase; 5] {
    let tokens = macro_tokens();

    let mut live_store = MacroStore::new();
    let live = intern_macro(&mut live_store, &tokens, 0);
    let live_id = live.id();

    let mut dead_store = MacroStore::new();
    let dead_mark = dead_store.watermark();
    let dead = intern_macro(&mut dead_store, &tokens, 1);
    let dead_id = dead.id();
    drop(dead);
    dead_store.truncate_to(dead_mark);

    let mut stale_store = MacroStore::new();
    let stale = intern_macro(&mut stale_store, &tokens, 2);
    let stale_id = stale.id();
    drop(stale);
    let replacement = intern_macro(&mut stale_store, &tokens, 3);
    assert_eq!(stale_id.raw(), replacement.id().raw());
    assert_ne!(stale_id, replacement.id());

    let mut hole_store = MacroStore::new();
    let hole_mark = hole_store.watermark();
    let hole = intern_macro(&mut hole_store, &tokens, 4);
    let hole_id = hole.id();
    drop(hole);
    hole_store.truncate_to(hole_mark);

    let collision_tokens = macro_tokens();
    let mut collision_store = MacroStore::new();
    collision_store.testing_force_candidate_collision();
    let left = intern_macro(&mut collision_store, &collision_tokens, 0);
    let right = intern_macro(&mut collision_store, &collision_tokens, 1);
    let right_id = right.id();
    let query = macro_query(&collision_tokens, 1);

    [
        MacroCase {
            store: live_store,
            query: live_id,
            expected: Some(live_id),
            collision: None,
            _roots: vec![live],
        },
        MacroCase {
            store: dead_store,
            query: dead_id,
            expected: None,
            collision: None,
            _roots: Vec::new(),
        },
        MacroCase {
            store: stale_store,
            query: stale_id,
            expected: None,
            collision: None,
            _roots: vec![replacement],
        },
        MacroCase {
            store: hole_store,
            query: MacroDefinitionId::testing_new(hole_id.raw()),
            expected: None,
            collision: None,
            _roots: Vec::new(),
        },
        MacroCase {
            store: collision_store,
            query: right_id,
            expected: Some(right_id),
            collision: Some(query),
            _roots: vec![left, right],
        },
    ]
}

struct MacroTokens {
    _store: TokenStore,
    empty: TokenListRef,
    replacements: [TokenListRef; 5],
}

fn macro_tokens() -> MacroTokens {
    let mut store = TokenStore::new();
    let empty = store
        .resolved_owner(TokenListId::EMPTY)
        .expect("canonical empty token root");
    let replacements = core::array::from_fn(|index| {
        store.testing_owned(
            &[char_token(char::from(b'a' + index as u8))],
            TokenSemanticId::testing(100 + index as u64),
            None,
        )
    });
    MacroTokens {
        _store: store,
        empty,
        replacements,
    }
}

fn macro_query(tokens: &MacroTokens, index: usize) -> MacroCollisionQuery {
    let replacement = tokens.replacements[index].clone();
    MacroCollisionQuery {
        meaning: MacroMeaning::new(MeaningFlags::EMPTY, tokens.empty.id(), replacement.id()),
        parameter_root: tokens.empty.clone(),
        replacement_semantic_id: replacement.semantic_id(),
        replacement_root: replacement,
        parameter_semantic_id: tokens.empty.semantic_id(),
    }
}

fn intern_macro(store: &mut MacroStore, tokens: &MacroTokens, index: usize) -> MacroDefinitionRef {
    let query = macro_query(tokens, index);
    store.intern_with_provenance(
        query.meaning,
        query.parameter_root.clone(),
        query.replacement_root.clone(),
        MacroParameterPattern::from_tokens(query.parameter_root.tokens()),
        query.parameter_semantic_id,
        query.replacement_semantic_id,
        None,
        2,
        None,
    )
}

fn char_token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_work_gate_covers_safety_branches_without_repeated_resolution() {
        let benchmark = ReachabilityLookupBenchmark::new();
        for (family, expected_work) in [
            (LookupFamily::TokenList, [4, 2, 2, 2, 6]),
            (LookupFamily::MacroBody, [3, 2, 2, 2, 6]),
        ] {
            for (case, expected) in LookupCase::ALL.into_iter().zip(expected_work) {
                let measured = benchmark.measure(family, case);
                assert!(
                    measured.found,
                    "{family:?}/{case:?} returned the wrong semantic result"
                );
                assert_eq!(
                    measured.work, expected,
                    "{family:?}/{case:?} repeated or skipped primitive lookup work"
                );
            }
        }
    }
}
