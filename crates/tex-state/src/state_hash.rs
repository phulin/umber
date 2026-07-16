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

use crate::ContentHash;
use sha2::{Digest, Sha256};

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

/// SHA-256 identity for exact-comparison data that is already canonically encoded.
#[must_use]
pub(crate) fn strong_identity_bytes(domain: &[u8], bytes: &[u8]) -> ContentHash {
    let mut hasher = Sha256::new();
    hasher.update(b"umber-exact-identity-v1");
    hasher.update((domain.len() as u64).to_le_bytes());
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    ContentHash::new(hasher.finalize().into())
}

/// A deterministic field-by-field state hasher.
#[derive(Clone, Debug)]
pub(crate) struct StateHasher {
    states: [u64; 4],
    lanes: u8,
    strong: Sha256,
}

/// Domain-separated fingerprint for semantic data that is immutable after
/// publication.
///
/// A fragment is derived state rather than a durable identity. Its own domain
/// keeps equal field sequences used for different semantic purposes distinct.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StateHashFragment {
    fingerprint: u64,
    identity: ContentHash,
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
    pub(crate) const fn from_parts(fingerprint: u64, identity: ContentHash) -> Self {
        Self {
            fingerprint,
            identity,
        }
    }

    #[must_use]
    pub(crate) fn from_builder(domain: u64, build: impl FnOnce(&mut StateHasher)) -> Self {
        let mut hasher = StateHasher::new(domain);
        build(&mut hasher);
        hasher.finish_fragment()
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
        let fragment = hasher.finish_fragment();
        #[cfg(feature = "profiling-stats")]
        crate::measurement::record_state_hash_component(component, visits, started.elapsed());
        #[cfg(not(feature = "profiling-stats"))]
        let _ = (component, visits);
        fragment
    }

    pub(crate) fn apply(&self, hasher: &mut StateHasher) {
        hasher.u64(self.fingerprint);
        hasher.strong_identity(self.identity);
    }

    #[must_use]
    pub(crate) const fn fingerprint(self) -> u64 {
        self.fingerprint
    }

    /// Collision-resistant identity over the fragment's canonical field stream.
    #[must_use]
    pub(crate) const fn identity(self) -> ContentHash {
        self.identity
    }

    #[must_use]
    pub(crate) const fn bytes(self) -> [u8; 32] {
        self.identity.bytes()
    }
}

impl StateHasher {
    #[must_use]
    pub(crate) fn new(domain: u64) -> Self {
        let mut strong = Sha256::new();
        strong.update(b"umber-state-fragment-v1");
        strong.update(domain.to_le_bytes());
        Self {
            states: [INITIAL_STATE ^ domain, 0, 0, 0],
            lanes: 1,
            strong,
        }
    }

    #[must_use]
    pub(crate) fn new_quad(domains: [u64; 4]) -> Self {
        let mut strong = Sha256::new();
        strong.update(b"umber-state-fragment-quad-v1");
        for domain in domains {
            strong.update(domain.to_le_bytes());
        }
        Self {
            states: [
                INITIAL_STATE ^ domains[0],
                INITIAL_STATE ^ domains[1],
                INITIAL_STATE ^ domains[2],
                INITIAL_STATE ^ domains[3],
            ],
            lanes: 4,
            strong,
        }
    }

    pub(crate) fn tag(&mut self, tag: u8) {
        self.u8(tag);
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.strong.update([value]);
        self.mix(u64::from(value));
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.strong.update(value.to_le_bytes());
        self.mix(u64::from(value));
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.strong.update(value.to_le_bytes());
        self.mix(u64::from(value));
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.strong.update(value.to_le_bytes());
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
        self.strong.update(bytes);
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

    /// Finishes only the collision-resistant canonical identity.
    #[must_use]
    pub(crate) fn finish_identity(self) -> ContentHash {
        ContentHash::new(self.strong.finalize().into())
    }

    pub(crate) fn finish_fragment(self) -> StateHashFragment {
        StateHashFragment {
            fingerprint: splitmix64(self.states[0]),
            identity: ContentHash::new(self.strong.finalize().into()),
        }
    }

    /// Frames a child strong identity without narrowing it through the rolling lane.
    pub(crate) fn strong_identity(&mut self, identity: ContentHash) {
        self.strong.update(identity.bytes());
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
    use super::{StateHashFragment, StateHasher, strong_identity_bytes};

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

    #[test]
    fn equal_rolling_fingerprints_do_not_alias_strong_child_identities() {
        let left = StateHashFragment {
            fingerprint: 0xdead_beef_dead_beef,
            identity: strong_identity_bytes(b"collision-shape", b"left"),
        };
        let right = StateHashFragment {
            fingerprint: left.fingerprint,
            identity: strong_identity_bytes(b"collision-shape", b"right"),
        };
        let compose = |child: StateHashFragment| {
            StateHashFragment::from_builder(0x636f_6c6c_6973_696f, |hasher| child.apply(hasher))
        };

        let left = compose(left);
        let right = compose(right);
        assert_eq!(left.fingerprint(), right.fingerprint());
        assert_ne!(left.identity(), right.identity());
    }
}
