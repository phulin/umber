//! Bounded session-local expansion memoization.

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
#[cfg(feature = "profiling-stats")]
use std::time::Instant;

use tex_state::token::{OriginId, Token, TracedTokenWord};
use tex_state::{ChangedAt, DependencyKey, ExpansionState, MemoValidationStamp, TracedTokenList};

use crate::ExpansionContext;

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
struct ExpansionEpisodeKey {
    mode: u8,
    engine: crate::EngineStateSnapshot,
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
    key: ExpansionEpisodeKey,
    stamp: MemoValidationStamp,
    dependencies: Vec<(DependencyKey, ChangedAt)>,
    output: Vec<Token>,
    origins: Vec<EpisodeOriginRecipe>,
    retained_bytes: usize,
}

pub(crate) struct EpisodeAttempt {
    key: ExpansionEpisodeKey,
    input_origins: Vec<OriginId>,
}

pub(crate) enum EpisodeStart {
    Hit(TracedTokenList),
    Miss(EpisodeAttempt),
}

pub(crate) struct ExpansionMemoCache {
    config: ExpansionMemoConfig,
    episodes: HashMap<u64, Vec<ExpansionEpisodeEntry>>,
    insertion_order: VecDeque<(u64, ExpansionEpisodeKey)>,
    stats: ExpansionMemoStats,
    #[cfg(test)]
    forced_candidate: Option<u64>,
}

impl ExpansionMemoCache {
    pub(crate) fn new(config: ExpansionMemoConfig) -> Self {
        Self {
            config,
            episodes: HashMap::new(),
            insertion_order: VecDeque::new(),
            stats: ExpansionMemoStats::default(),
            #[cfg(test)]
            forced_candidate: None,
        }
    }

    pub(crate) fn stats(&self) -> ExpansionMemoStats {
        self.stats
    }

    pub(crate) fn clear(&mut self) {
        self.episodes.clear();
        self.insertion_order.clear();
        self.stats.retained_entries = 0;
        self.stats.retained_bytes = 0;
    }

    fn can_retain(&self, bytes: usize) -> bool {
        self.config.max_entries > 0 && bytes <= self.config.max_retained_bytes
    }

    fn make_room(&mut self, bytes: usize) {
        while self.stats.retained_entries >= self.config.max_entries
            || self.stats.retained_bytes.saturating_add(bytes) > self.config.max_retained_bytes
        {
            let Some((candidate, key)) = self.insertion_order.pop_front() else {
                break;
            };
            let Some(bucket) = self.episodes.get_mut(&candidate) else {
                continue;
            };
            let Some(index) = bucket.iter().position(|entry| entry.key == key) else {
                continue;
            };
            let removed = bucket.swap_remove(index).retained_bytes;
            if bucket.is_empty() {
                self.episodes.remove(&candidate);
            }
            self.stats.retained_bytes = self.stats.retained_bytes.saturating_sub(removed);
            self.stats.retained_entries = self.stats.retained_entries.saturating_sub(1);
            self.stats.evictions = self.stats.evictions.saturating_add(1);
        }
    }

    fn lookup_episode(
        &mut self,
        key: &ExpansionEpisodeKey,
        owner_nonce: u64,
        stores: &tex_state::ExpansionContext<'_>,
    ) -> Option<(Vec<Token>, Vec<EpisodeOriginRecipe>)> {
        self.stats.episode_lookups = self.stats.episode_lookups.saturating_add(1);
        let candidate = self.episode_candidate(key);
        let Some(bucket) = self.episodes.get_mut(&candidate) else {
            self.stats.episode_misses = self.stats.episode_misses.saturating_add(1);
            return None;
        };
        let Some(index) = bucket.iter().position(|entry| entry.key == *key) else {
            self.stats.episode_misses = self.stats.episode_misses.saturating_add(1);
            return None;
        };
        let valid = if bucket[index]
            .stamp
            .same_universe(MemoValidationStamp::new_for_owner(owner_nonce))
        {
            bucket[index]
                .dependencies
                .iter()
                .all(|(key, changed)| stores.dependency_changed_at(*key) == *changed)
        } else {
            bucket[index].stamp.state_hash() == stores.memo_validation_stamp().state_hash()
        };
        if !valid || bucket[index].output.len() != bucket[index].origins.len() {
            let removed = bucket.swap_remove(index);
            if bucket.is_empty() {
                self.episodes.remove(&candidate);
            }
            self.stats.retained_bytes = self
                .stats
                .retained_bytes
                .saturating_sub(removed.retained_bytes);
            self.stats.retained_entries = self.stats.retained_entries.saturating_sub(1);
            self.stats.episode_invalidations = self.stats.episode_invalidations.saturating_add(1);
            self.stats.episode_misses = self.stats.episode_misses.saturating_add(1);
            return None;
        }
        let entry = &bucket[index];
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
        stamp: MemoValidationStamp,
        dependencies: Vec<(DependencyKey, ChangedAt)>,
        output: Vec<Token>,
        origins: Vec<EpisodeOriginRecipe>,
    ) {
        let retained_bytes = std::mem::size_of::<ExpansionEpisodeEntry>()
            .saturating_add(std::mem::size_of::<(u64, ExpansionEpisodeKey)>())
            .saturating_add(std::mem::size_of::<u64>())
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
        let key = attempt.key;
        self.episodes
            .entry(candidate)
            .or_default()
            .push(ExpansionEpisodeEntry {
                key: key.clone(),
                stamp,
                dependencies,
                output,
                origins,
                retained_bytes,
            });
        self.insertion_order.push_back((candidate, key));
        self.stats.retained_entries = self.stats.retained_entries.saturating_add(1);
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
    #[cfg(feature = "profiling-stats")]
    let started = Instant::now();
    let key = ExpansionEpisodeKey {
        mode,
        engine: expansion.engine,
        input: stores.tokens(input.token_list()).to_vec(),
        job_name: expansion.job_name.to_owned(),
        job_clock: expansion.job_clock,
    };
    let input_origins = live_origins(stores, input);
    let owner_nonce = stores.memo_owner_nonce();
    let hit = expansion
        .memo
        .as_mut()
        .expect("episode lookup requires enabled memoization")
        .lookup_episode(&key, owner_nonce, stores);
    if let Some((tokens, recipes)) = hit {
        let traced = tokens
            .into_iter()
            .zip(recipes)
            .map(|(token, recipe)| {
                let origin = instantiate_episode_origin(stores, &input_origins, &recipe);
                TracedTokenWord::pack(token, origin)
            })
            .collect::<Vec<_>>();
        let hit = EpisodeStart::Hit(stores.finish_traced_token_list(&traced));
        #[cfg(feature = "profiling-stats")]
        expansion
            .memo
            .as_mut()
            .expect("episode cache enabled")
            .record_lookup_elapsed(started);
        return hit;
    }
    let miss = EpisodeStart::Miss(EpisodeAttempt { key, input_origins });
    #[cfg(feature = "profiling-stats")]
    expansion
        .memo
        .as_mut()
        .expect("episode cache enabled")
        .record_lookup_elapsed(started);
    miss
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
    let mut input_ordinals = HashMap::with_capacity(attempt.input_origins.len());
    for (index, origin) in attempt.input_origins.iter().copied().enumerate() {
        input_ordinals.entry(origin).or_insert(index);
    }
    let recipes = output_origins
        .iter()
        .map(|origin| episode_origin_recipe(stores, &input_ordinals, *origin, 0))
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
        .insert_episode(
            attempt,
            stores.memo_validation_stamp(),
            dependencies,
            output_tokens,
            recipes,
        );
}

fn episode_origin_recipe(
    stores: &tex_state::ExpansionContext<'_>,
    input_ordinals: &HashMap<OriginId, usize>,
    origin: OriginId,
    depth: u8,
) -> Option<EpisodeOriginRecipe> {
    if origin == OriginId::UNKNOWN {
        return Some(EpisodeOriginRecipe::Unknown);
    }
    if let Some(index) = input_ordinals.get(&origin) {
        return Some(EpisodeOriginRecipe::InputOrdinal(*index));
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
                    input_ordinals,
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
                    input_ordinals,
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

#[cfg(test)]
mod tests;
