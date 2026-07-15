//! Deterministic semantic state hashing.
//!
//! This module intentionally does not expose a generic `Hash` adapter. Callers
//! feed only fields that are known to be semantic so raw handles and allocation
//! identities do not accidentally become part of convergence hashes.
//!
//! Checkpoint hashes are folds of per-slice hashes via [`combine`], so they
//! depend on where checkpoint boundaries fell, not only on the final semantic
//! state: splitting one slice into two changes the fold. Convergence
//! comparison is valid only between runs with identical checkpoint schedules;
//! see `docs/core_state.md` §9.

const MIX_INCREMENT: u64 = 0x9e37_79b9_7f4a_7c15;
// Schema-v8 streaming constants. The odd multiplier keeps the recurrence
// invertible for each framed word; `finish` supplies the full avalanche once.
const STREAM_MULTIPLIER: u64 = 0x9e37_79b1_85eb_ca87;
const STREAM_INCREMENT: u64 = 0x632b_e59b_d9b4_e019;
const INITIAL_STATE: u64 = 0x6a09_e667_f3bc_c909;

/// Initial checkpoint hash before any semantic slice is combined.
pub(crate) const INITIAL_STATE_HASH: u64 = INITIAL_STATE;

/// Performance-owner categories for discardable checkpoint projections.
///
/// These labels are never part of semantic state. They exist so feature-gated
/// profiling can attribute both traversal and elapsed time without changing
/// the canonical projection bytes.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(feature = "profiling-stats"), allow(dead_code))]
pub(crate) enum StateHashComponent {
    Journal,
    CodeTables,
    Hyphenation,
    PreparedMag,
    FontSelection,
    WorldEffects,
    WorldShellEscapes,
    WorldStreams,
    WorldScalars,
    InputFrames,
    Interaction,
    PageScalars,
    PageInsertions,
    PageMarks,
    PageContribution,
    PageCurrent,
    PageDiscards,
    Mode,
}

impl StateHashComponent {
    #[cfg_attr(not(feature = "profiling-stats"), allow(dead_code))]
    pub(crate) const COUNT: usize = 18;

    #[cfg_attr(not(feature = "profiling-stats"), allow(dead_code))]
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// Combines a previous checkpoint hash with the next semantic slice hash.
#[must_use]
pub(crate) fn combine(prev: u64, slice: u64) -> u64 {
    splitmix64(prev ^ slice.wrapping_add(MIX_INCREMENT))
}

/// A deterministic field-by-field state hasher.
#[derive(Clone, Debug)]
pub(crate) struct StateHasher {
    states: [u64; 4],
    lanes: u8,
}

/// Domain-separated fingerprint for semantic data that is immutable after
/// publication.
///
/// A fragment is derived state rather than a durable identity. Its own domain
/// keeps equal field sequences used for different semantic purposes distinct.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StateHashFragment {
    fingerprint: u64,
}

/// One discardable canonical projection paired with its private reuse key.
///
/// The key may use allocation identity, but it is consulted only through the
/// caller-supplied predicate and is never incorporated into the fingerprint.
#[derive(Clone, Debug)]
pub(crate) struct CachedProjection<K> {
    key: K,
    fragment: StateHashFragment,
}

impl<K> CachedProjection<K> {
    pub(crate) const fn new(key: K, fragment: StateHashFragment) -> Self {
        Self { key, fragment }
    }

    pub(crate) fn fragment_if(
        &self,
        matches: impl FnOnce(&K) -> bool,
    ) -> Option<StateHashFragment> {
        matches(&self.key).then_some(self.fragment)
    }
}

impl StateHashFragment {
    #[must_use]
    pub(crate) fn from_builder(domain: u64, build: impl FnOnce(&mut StateHasher)) -> Self {
        let mut hasher = StateHasher::new(domain);
        build(&mut hasher);
        Self {
            fingerprint: hasher.finish(),
        }
    }

    #[must_use]
    pub(crate) fn from_measured_builder(
        domain: u64,
        component: StateHashComponent,
        visits: usize,
        build: impl FnOnce(&mut StateHasher),
    ) -> Self {
        Self::from_measured_builder_counted(domain, component, |hasher| {
            build(hasher);
            visits
        })
    }

    #[must_use]
    pub(crate) fn from_measured_builder_counted(
        domain: u64,
        component: StateHashComponent,
        build: impl FnOnce(&mut StateHasher) -> usize,
    ) -> Self {
        #[cfg(feature = "profiling-stats")]
        let started = crate::world::World::start_profiling_timer();
        let mut hasher = StateHasher::new(domain);
        let visits = build(&mut hasher);
        let fragment = Self {
            fingerprint: hasher.finish(),
        };
        #[cfg(feature = "profiling-stats")]
        crate::measurement::record_state_hash_component(component, visits, started.elapsed());
        #[cfg(not(feature = "profiling-stats"))]
        let _ = (component, visits);
        fragment
    }

    pub(crate) fn apply(&self, hasher: &mut StateHasher) {
        hasher.u64(self.fingerprint);
    }

    #[must_use]
    pub(crate) const fn fingerprint(self) -> u64 {
        self.fingerprint
    }
}

impl StateHasher {
    #[must_use]
    pub(crate) const fn new(domain: u64) -> Self {
        Self {
            states: [INITIAL_STATE ^ domain, 0, 0, 0],
            lanes: 1,
        }
    }

    #[must_use]
    pub(crate) const fn new_quad(domains: [u64; 4]) -> Self {
        Self {
            states: [
                INITIAL_STATE ^ domains[0],
                INITIAL_STATE ^ domains[1],
                INITIAL_STATE ^ domains[2],
                INITIAL_STATE ^ domains[3],
            ],
            lanes: 4,
        }
    }

    pub(crate) fn tag(&mut self, tag: u8) {
        self.u8(tag);
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.mix(u64::from(value));
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.mix(u64::from(value));
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.mix(u64::from(value));
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.mix(value);
    }

    pub(crate) fn i32(&mut self, value: i32) {
        self.u32(value as u32);
    }

    pub(crate) fn usize(&mut self, value: usize) {
        self.u64(u64::try_from(value).expect("state hash length exceeds u64"));
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.usize(bytes.len());
        for chunk in bytes.chunks(8) {
            let mut word = 0_u64;
            for (offset, byte) in chunk.iter().copied().enumerate() {
                word |= u64::from(byte) << (offset * 8);
            }
            self.mix(word);
        }
    }

    pub(crate) fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    #[must_use]
    pub(crate) fn finish(self) -> u64 {
        splitmix64(self.states[0])
    }

    #[must_use]
    pub(crate) fn finish_quad(self) -> [u64; 4] {
        assert_eq!(self.lanes, 4, "quad finish requires four hash lanes");
        self.states.map(splitmix64)
    }

    fn mix(&mut self, value: u64) {
        if self.lanes == 1 {
            self.states[0] ^= value.wrapping_add(MIX_INCREMENT);
            self.states[0] = self.states[0]
                .rotate_left(27)
                .wrapping_mul(STREAM_MULTIPLIER)
                .wrapping_add(STREAM_INCREMENT);
            return;
        }
        for state in &mut self.states {
            *state ^= value.wrapping_add(MIX_INCREMENT);
            *state = state
                .rotate_left(27)
                .wrapping_mul(STREAM_MULTIPLIER)
                .wrapping_add(STREAM_INCREMENT);
        }
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(MIX_INCREMENT);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::StateHasher;

    #[test]
    fn quad_projection_matches_four_independent_projections() {
        let domains = [11, 22, 33, 44];
        let write = |hasher: &mut StateHasher| {
            hasher.tag(7);
            hasher.bool(true);
            hasher.u16(65_000);
            hasher.i32(-123_456);
            hasher.bytes(b"one semantic traversal");
        };
        let expected = domains.map(|domain| {
            let mut hasher = StateHasher::new(domain);
            write(&mut hasher);
            hasher.finish()
        });
        let mut combined = StateHasher::new_quad(domains);
        write(&mut combined);
        assert_eq!(combined.finish_quad(), expected);
    }
}
