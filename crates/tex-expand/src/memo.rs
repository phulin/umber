//! Bounded session-local expansion memoization.

use std::hash::{DefaultHasher, Hash, Hasher};
#[cfg(feature = "profiling-stats")]
use std::time::Instant;

use tex_lex::{InputStack, MacroArguments};
use tex_state::ExpansionState;
use tex_state::ids::MacroDefinitionId;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::args::MatchedArguments;
use crate::{Dispatch, ExpansionContext, ExpansionReplayKind};

/// Capacity policy for one expansion session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpansionMemoConfig {
    pub max_entries: usize,
    pub max_retained_bytes: usize,
}

impl Default for ExpansionMemoConfig {
    fn default() -> Self {
        Self {
            max_entries: 1_024,
            max_retained_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Observable work and ownership counters for the expansion memo layer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExpansionMemoStats {
    pub substitution_lookups: u64,
    pub substitution_hits: u64,
    pub substitution_misses: u64,
    pub substituted_tokens_reused: u64,
    pub lookup_nanos: u64,
    pub retained_entries: usize,
    pub retained_bytes: usize,
    pub evictions: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MacroSubstitutionKey {
    flags: u8,
    parameter_text: Vec<Token>,
    replacement_text: Vec<Token>,
    arguments: [Vec<Token>; 9],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OriginRecipe {
    Replacement(usize),
    Argument { slot: u8, index: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PlannedToken {
    token: Token,
    origin: OriginRecipe,
}

#[derive(Clone, Debug)]
struct MacroSubstitutionEntry {
    candidate: u64,
    key: MacroSubstitutionKey,
    plan: Vec<PlannedToken>,
    retained_bytes: usize,
}

pub(crate) struct ExpansionMemoCache {
    config: ExpansionMemoConfig,
    entries: Vec<MacroSubstitutionEntry>,
    stats: ExpansionMemoStats,
    #[cfg(test)]
    forced_candidate: Option<u64>,
}

impl ExpansionMemoCache {
    pub(crate) fn new(config: ExpansionMemoConfig) -> Self {
        Self {
            config,
            entries: Vec::new(),
            stats: ExpansionMemoStats::default(),
            #[cfg(test)]
            forced_candidate: None,
        }
    }

    pub(crate) fn stats(&self) -> ExpansionMemoStats {
        self.stats
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.stats.retained_entries = 0;
        self.stats.retained_bytes = 0;
    }

    fn candidate(&self, key: &MacroSubstitutionKey) -> u64 {
        #[cfg(test)]
        if let Some(candidate) = self.forced_candidate {
            return candidate;
        }
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    fn plan(&mut self, key: MacroSubstitutionKey) -> Vec<PlannedToken> {
        #[cfg(feature = "profiling-stats")]
        let started = Instant::now();
        self.stats.substitution_lookups = self.stats.substitution_lookups.saturating_add(1);
        let candidate = self.candidate(&key);
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.candidate == candidate && entry.key == key)
        {
            self.stats.substitution_hits = self.stats.substitution_hits.saturating_add(1);
            self.stats.substituted_tokens_reused = self
                .stats
                .substituted_tokens_reused
                .saturating_add(entry.plan.len() as u64);
            #[cfg(feature = "profiling-stats")]
            self.record_lookup_elapsed(started);
            return entry.plan.clone();
        }

        self.stats.substitution_misses = self.stats.substitution_misses.saturating_add(1);
        let plan = build_plan(&key);
        let retained_bytes = retained_bytes(&key, &plan);
        if self.config.max_entries > 0 && retained_bytes <= self.config.max_retained_bytes {
            while self.entries.len() >= self.config.max_entries
                || self.stats.retained_bytes.saturating_add(retained_bytes)
                    > self.config.max_retained_bytes
            {
                let removed = self.entries.remove(0);
                self.stats.retained_bytes = self
                    .stats
                    .retained_bytes
                    .saturating_sub(removed.retained_bytes);
                self.stats.evictions = self.stats.evictions.saturating_add(1);
            }
            self.entries.push(MacroSubstitutionEntry {
                candidate,
                key,
                plan: plan.clone(),
                retained_bytes,
            });
            self.stats.retained_entries = self.entries.len();
            self.stats.retained_bytes = self.stats.retained_bytes.saturating_add(retained_bytes);
        }
        #[cfg(feature = "profiling-stats")]
        self.record_lookup_elapsed(started);
        plan
    }

    #[cfg(feature = "profiling-stats")]
    fn record_lookup_elapsed(&mut self, started: Instant) {
        self.stats.lookup_nanos = self
            .stats
            .lookup_nanos
            .saturating_add(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn memoized_macro_dispatch(
    input: &InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    definition: MacroDefinitionId,
    meaning: MacroMeaning,
    provenance: MacroDefinitionProvenance,
    arguments: MatchedArguments,
    call_origin: OriginId,
) -> Dispatch {
    let key = capture_key(stores, meaning, &arguments);
    let plan = expansion
        .memo
        .as_mut()
        .expect("memoized dispatch requires an enabled cache")
        .plan(key);
    let traced = instantiate_plan(stores, provenance, &arguments, &plan);
    let substituted = stores.finish_traced_token_list(&traced);
    Dispatch::Push {
        replay_kind: ExpansionReplayKind::MacroBody,
        token_list: substituted.token_list(),
        origin_list: substituted.origin_list(),
        macro_arguments: MacroArguments::new(),
        macro_invocation: stores.macro_invocation_origin(
            definition,
            call_origin,
            provenance.definition_origin(),
            input.active_macro_invocation(),
        ),
    }
}

fn capture_key(
    stores: &impl ExpansionState,
    meaning: MacroMeaning,
    arguments: &MatchedArguments,
) -> MacroSubstitutionKey {
    let arguments = std::array::from_fn(|index| {
        arguments
            .get(index as u8 + 1)
            .unwrap_or_default()
            .iter()
            .map(|word| semantic_token(*word))
            .collect()
    });
    MacroSubstitutionKey {
        flags: meaning.flags().bits(),
        parameter_text: stores.tokens(meaning.parameter_text()).to_vec(),
        replacement_text: stores.tokens(meaning.replacement_text()).to_vec(),
        arguments,
    }
}

fn build_plan(key: &MacroSubstitutionKey) -> Vec<PlannedToken> {
    let mut plan = Vec::new();
    for (replacement_index, &token) in key.replacement_text.iter().enumerate() {
        if let Token::Param(slot @ 1..=9) = token {
            for (index, &token) in key.arguments[usize::from(slot - 1)].iter().enumerate() {
                plan.push(PlannedToken {
                    token,
                    origin: OriginRecipe::Argument { slot, index },
                });
            }
        } else {
            plan.push(PlannedToken {
                token,
                origin: OriginRecipe::Replacement(replacement_index),
            });
        }
    }
    plan
}

fn instantiate_plan(
    stores: &impl ExpansionState,
    provenance: MacroDefinitionProvenance,
    arguments: &MatchedArguments,
    plan: &[PlannedToken],
) -> Vec<TracedTokenWord> {
    let replacement_origins = stores.origin_list_if_live(provenance.replacement_origins());
    plan.iter()
        .map(|planned| {
            let origin = match planned.origin {
                OriginRecipe::Replacement(index) => replacement_origins
                    .and_then(|origins| origins.get(index))
                    .copied()
                    .unwrap_or(OriginId::UNKNOWN),
                OriginRecipe::Argument { slot, index } => arguments
                    .get(slot)
                    .and_then(|argument| argument.get(index))
                    .map_or(OriginId::UNKNOWN, |word| word.origin()),
            };
            TracedTokenWord::pack(planned.token, origin)
        })
        .collect()
}

fn retained_bytes(key: &MacroSubstitutionKey, plan: &[PlannedToken]) -> usize {
    let key_tokens = key
        .parameter_text
        .len()
        .saturating_add(key.replacement_text.len())
        .saturating_add(key.arguments.iter().map(Vec::len).sum::<usize>());
    std::mem::size_of::<MacroSubstitutionEntry>()
        .saturating_add(key_tokens.saturating_mul(std::mem::size_of::<Token>()))
        .saturating_add(
            plan.len()
                .saturating_mul(std::mem::size_of::<PlannedToken>()),
        )
}

fn semantic_token(word: TracedTokenWord) -> Token {
    word.token()
        .expect("macro memoization received an invalid traced token")
}

#[cfg(test)]
mod tests;
