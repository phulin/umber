//! Bounded session-local expansion memoization.

use std::hash::{DefaultHasher, Hash, Hasher};
#[cfg(feature = "profiling-stats")]
use std::time::Instant;

use tex_lex::{InputStack, MacroArguments};
use tex_state::ids::MacroDefinitionId;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::token::{OriginId, Token, TracedTokenWord};
use tex_state::{ChangedAt, DependencyKey, ExpansionState, MemoValidationStamp, TracedTokenList};

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
    pub episode_lookups: u64,
    pub episode_hits: u64,
    pub episode_misses: u64,
    pub episode_invalidations: u64,
    pub episode_barrier_rejections: u64,
    pub expanded_tokens_reused: u64,
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ExpansionEpisodeKey {
    mode: u8,
    input: Vec<Token>,
    job_name: String,
    job_clock: tex_state::JobClock,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EpisodeOriginRecipe {
    Unknown,
    InputOrdinal(usize),
    Inserted {
        kind: tex_state::provenance::InsertedOriginKind,
        token: Token,
        parent: Box<EpisodeOriginRecipe>,
    },
    Synthesized {
        kind: tex_state::provenance::SynthesizedOriginKind,
        parent: Box<EpisodeOriginRecipe>,
    },
}

#[derive(Clone, Debug)]
struct ExpansionEpisodeEntry {
    candidate: u64,
    key: ExpansionEpisodeKey,
    stamp: MemoValidationStamp,
    dependencies: Vec<(DependencyKey, ChangedAt)>,
    output: Vec<Token>,
    origins: Vec<EpisodeOriginRecipe>,
    retained_bytes: usize,
}

pub(crate) struct EpisodeAttempt {
    key: ExpansionEpisodeKey,
    stamp: MemoValidationStamp,
    input_origins: Vec<OriginId>,
}

pub(crate) enum EpisodeStart {
    Hit(TracedTokenList),
    Miss(EpisodeAttempt),
}

pub(crate) struct ExpansionMemoCache {
    config: ExpansionMemoConfig,
    entries: Vec<MacroSubstitutionEntry>,
    episodes: Vec<ExpansionEpisodeEntry>,
    stats: ExpansionMemoStats,
    #[cfg(test)]
    forced_candidate: Option<u64>,
}

impl ExpansionMemoCache {
    pub(crate) fn new(config: ExpansionMemoConfig) -> Self {
        Self {
            config,
            entries: Vec::new(),
            episodes: Vec::new(),
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
        self.episodes.clear();
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
        if self.can_retain(retained_bytes) {
            self.make_room(retained_bytes);
            self.entries.push(MacroSubstitutionEntry {
                candidate,
                key,
                plan: plan.clone(),
                retained_bytes,
            });
            self.stats.retained_entries = self.entries.len() + self.episodes.len();
            self.stats.retained_bytes = self.stats.retained_bytes.saturating_add(retained_bytes);
        }
        #[cfg(feature = "profiling-stats")]
        self.record_lookup_elapsed(started);
        plan
    }

    fn can_retain(&self, bytes: usize) -> bool {
        self.config.max_entries > 0 && bytes <= self.config.max_retained_bytes
    }

    fn make_room(&mut self, bytes: usize) {
        while self.entries.len() + self.episodes.len() >= self.config.max_entries
            || self.stats.retained_bytes.saturating_add(bytes) > self.config.max_retained_bytes
        {
            let removed = if self.entries.is_empty() {
                self.episodes.remove(0).retained_bytes
            } else {
                self.entries.remove(0).retained_bytes
            };
            self.stats.retained_bytes = self.stats.retained_bytes.saturating_sub(removed);
            self.stats.evictions = self.stats.evictions.saturating_add(1);
        }
    }

    fn lookup_episode(
        &mut self,
        key: &ExpansionEpisodeKey,
        stamp: MemoValidationStamp,
        stores: &tex_state::ExpansionContext<'_>,
    ) -> Option<(Vec<Token>, Vec<EpisodeOriginRecipe>)> {
        self.stats.episode_lookups = self.stats.episode_lookups.saturating_add(1);
        let candidate = self.episode_candidate(key);
        let Some(index) = self
            .episodes
            .iter()
            .position(|entry| entry.candidate == candidate && entry.key == *key)
        else {
            self.stats.episode_misses = self.stats.episode_misses.saturating_add(1);
            return None;
        };
        let valid = if self.episodes[index].stamp.same_universe(stamp) {
            self.episodes[index]
                .dependencies
                .iter()
                .all(|(key, changed)| stores.dependency_changed_at(*key) == *changed)
        } else {
            self.episodes[index].stamp.state_hash() == stamp.state_hash()
        };
        if !valid || self.episodes[index].output.len() != self.episodes[index].origins.len() {
            let removed = self.episodes.remove(index);
            self.stats.retained_bytes = self
                .stats
                .retained_bytes
                .saturating_sub(removed.retained_bytes);
            self.stats.retained_entries = self.entries.len() + self.episodes.len();
            self.stats.episode_invalidations = self.stats.episode_invalidations.saturating_add(1);
            self.stats.episode_misses = self.stats.episode_misses.saturating_add(1);
            return None;
        }
        let entry = &self.episodes[index];
        self.stats.episode_hits = self.stats.episode_hits.saturating_add(1);
        self.stats.expanded_tokens_reused = self
            .stats
            .expanded_tokens_reused
            .saturating_add(entry.output.len() as u64);
        Some((entry.output.clone(), entry.origins.clone()))
    }

    fn insert_episode(
        &mut self,
        attempt: EpisodeAttempt,
        dependencies: Vec<(DependencyKey, ChangedAt)>,
        output: Vec<Token>,
        origins: Vec<EpisodeOriginRecipe>,
    ) {
        let retained_bytes = std::mem::size_of::<ExpansionEpisodeEntry>()
            .saturating_add(
                attempt
                    .key
                    .input
                    .len()
                    .saturating_add(output.len())
                    .saturating_mul(std::mem::size_of::<Token>()),
            )
            .saturating_add(
                dependencies
                    .len()
                    .saturating_mul(std::mem::size_of::<(DependencyKey, ChangedAt)>()),
            )
            .saturating_add(
                origins
                    .len()
                    .saturating_mul(std::mem::size_of::<EpisodeOriginRecipe>()),
            )
            .saturating_add(origins.iter().map(episode_origin_heap_bytes).sum::<usize>())
            .saturating_add(attempt.key.job_name.capacity());
        if !self.can_retain(retained_bytes) {
            return;
        }
        self.make_room(retained_bytes);
        let candidate = self.episode_candidate(&attempt.key);
        self.episodes.push(ExpansionEpisodeEntry {
            candidate,
            key: attempt.key,
            stamp: attempt.stamp,
            dependencies,
            output,
            origins,
            retained_bytes,
        });
        self.stats.retained_entries = self.entries.len() + self.episodes.len();
        self.stats.retained_bytes = self.stats.retained_bytes.saturating_add(retained_bytes);
    }

    fn episode_candidate(&self, key: &ExpansionEpisodeKey) -> u64 {
        #[cfg(test)]
        if let Some(candidate) = self.forced_candidate {
            return candidate;
        }
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    #[cfg(feature = "profiling-stats")]
    fn record_lookup_elapsed(&mut self, started: Instant) {
        self.stats.lookup_nanos = self
            .stats
            .lookup_nanos
            .saturating_add(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
    }
}

fn episode_origin_heap_bytes(recipe: &EpisodeOriginRecipe) -> usize {
    match recipe {
        EpisodeOriginRecipe::Inserted { parent, .. }
        | EpisodeOriginRecipe::Synthesized { parent, .. } => {
            std::mem::size_of::<EpisodeOriginRecipe>()
                .saturating_add(episode_origin_heap_bytes(parent))
        }
        EpisodeOriginRecipe::Unknown | EpisodeOriginRecipe::InputOrdinal(_) => 0,
    }
}

pub(crate) fn start_expansion_episode(
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    input: TracedTokenList,
    mode: u8,
) -> EpisodeStart {
    let key = ExpansionEpisodeKey {
        mode,
        input: stores.tokens(input.token_list()).to_vec(),
        job_name: expansion.job_name.to_owned(),
        job_clock: expansion.job_clock,
    };
    let input_origins = live_origins(stores, input);
    let stamp = stores.memo_validation_stamp();
    let hit = expansion
        .memo
        .as_mut()
        .expect("episode lookup requires enabled memoization")
        .lookup_episode(&key, stamp, stores);
    if let Some((tokens, recipes)) = hit {
        let traced = tokens
            .into_iter()
            .zip(recipes)
            .map(|(token, recipe)| {
                let origin = instantiate_episode_origin(stores, &input_origins, &recipe);
                TracedTokenWord::pack(token, origin)
            })
            .collect::<Vec<_>>();
        return EpisodeStart::Hit(stores.finish_traced_token_list(&traced));
    }
    EpisodeStart::Miss(EpisodeAttempt {
        key,
        stamp,
        input_origins,
    })
}

pub(crate) fn finish_expansion_episode(
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    attempt: EpisodeAttempt,
    output: TracedTokenList,
    dependencies: Vec<DependencyKey>,
    eligible: bool,
) {
    let output_tokens = stores.tokens(output.token_list()).to_vec();
    let output_origins = live_origins(stores, output);
    let recipes = output_origins
        .iter()
        .map(|origin| episode_origin_recipe(stores, &attempt.input_origins, *origin, 0))
        .collect::<Option<Vec<_>>>();
    let Some(recipes) = recipes.filter(|_| eligible && output_tokens.len() == output_origins.len())
    else {
        let memo = expansion.memo.as_mut().expect("episode cache enabled");
        memo.stats.episode_barrier_rejections =
            memo.stats.episode_barrier_rejections.saturating_add(1);
        return;
    };
    let dependencies = dependencies
        .into_iter()
        .map(|key| (key, stores.track_dependency(key)))
        .collect();
    expansion
        .memo
        .as_mut()
        .expect("episode cache enabled")
        .insert_episode(attempt, dependencies, output_tokens, recipes);
}

fn episode_origin_recipe(
    stores: &tex_state::ExpansionContext<'_>,
    input_origins: &[OriginId],
    origin: OriginId,
    depth: u8,
) -> Option<EpisodeOriginRecipe> {
    if origin == OriginId::UNKNOWN {
        return Some(EpisodeOriginRecipe::Unknown);
    }
    if let Some(index) = input_origins
        .iter()
        .position(|candidate| *candidate == origin)
    {
        return Some(EpisodeOriginRecipe::InputOrdinal(index));
    }
    if depth >= 64 {
        return None;
    }
    match stores.origin(origin) {
        tex_state::provenance::OriginRecord::Inserted(inserted) => {
            Some(EpisodeOriginRecipe::Inserted {
                kind: inserted.kind(),
                token: inserted.token(),
                parent: Box::new(episode_origin_recipe(
                    stores,
                    input_origins,
                    inserted.parent(),
                    depth + 1,
                )?),
            })
        }
        tex_state::provenance::OriginRecord::Synthesized(synthesized) => {
            Some(EpisodeOriginRecipe::Synthesized {
                kind: synthesized.kind(),
                parent: Box::new(episode_origin_recipe(
                    stores,
                    input_origins,
                    synthesized.parent(),
                    depth + 1,
                )?),
            })
        }
        tex_state::provenance::OriginRecord::UnknownBootstrap => Some(EpisodeOriginRecipe::Unknown),
        tex_state::provenance::OriginRecord::Source(_)
        | tex_state::provenance::OriginRecord::SourceSpan(_)
        | tex_state::provenance::OriginRecord::MacroInvocation(_)
        | tex_state::provenance::OriginRecord::Synthetic(_) => None,
    }
}

fn instantiate_episode_origin(
    stores: &mut tex_state::ExpansionContext<'_>,
    input_origins: &[OriginId],
    recipe: &EpisodeOriginRecipe,
) -> OriginId {
    match recipe {
        EpisodeOriginRecipe::Unknown => OriginId::UNKNOWN,
        EpisodeOriginRecipe::InputOrdinal(index) => input_origins
            .get(*index)
            .copied()
            .unwrap_or(OriginId::UNKNOWN),
        EpisodeOriginRecipe::Inserted {
            kind,
            token,
            parent,
        } => {
            let parent = instantiate_episode_origin(stores, input_origins, parent);
            stores.inserted_origin(*kind, *token, parent)
        }
        EpisodeOriginRecipe::Synthesized { kind, parent } => {
            let parent = instantiate_episode_origin(stores, input_origins, parent);
            stores.synthesized_origin(*kind, parent)
        }
    }
}

fn live_origins(stores: &impl ExpansionState, value: TracedTokenList) -> Vec<OriginId> {
    if value.origin_list() == tex_state::ids::OriginListId::EMPTY {
        return vec![OriginId::UNKNOWN; stores.tokens(value.token_list()).len()];
    }
    stores.origin_list_if_live(value.origin_list()).map_or_else(
        || vec![OriginId::UNKNOWN; stores.tokens(value.token_list()).len()],
        <[OriginId]>::to_vec,
    )
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
