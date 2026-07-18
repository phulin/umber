use crate::state_hash::exact_identity_bytes;
use ahash::{AHashSet, RandomState};

const COLLECTION_DOMAIN: &[u8] = b"umber-exact-canonical-collection-v2";
const MIX_SALT_A: u64 = 0x6a09_e667_f3bc_c909;
const MIX_SALT_B: u64 = 0xbb67_ae85_84ca_a73b;

/// Order-independent probabilistic identity for a set of content identities.
///
/// Membership suppresses duplicate content, while the commutative accumulators
/// make each distinct insertion expected O(1) and independent of allocation or
/// insertion order. The cache is derived acceleration state: callers rebuild it
/// from canonical leaves whenever allocator ancestry cannot prove extension.
#[derive(Debug)]
pub(super) struct CanonicalCollectionIdentity {
    members: AHashSet<u64>,
    sum_a: u64,
    sum_b: u64,
    xor: u64,
    cached_identity: Option<u64>,
}

impl Default for CanonicalCollectionIdentity {
    fn default() -> Self {
        Self {
            members: AHashSet::with_hasher(RandomState::with_seeds(
                0x756d_6265_725f_636f,
                0x6c6c_6563_7469_6f6e,
                0x5f6d_656d_6265_7273,
                0x5f76_325f_6669_7865,
            )),
            sum_a: 0,
            sum_b: 0,
            xor: 0,
            cached_identity: None,
        }
    }
}

impl CanonicalCollectionIdentity {
    pub(super) fn reserve(&mut self, additional: usize) {
        self.members.reserve(additional);
    }

    pub(super) fn insert(&mut self, key: u64) {
        if !self.members.insert(key) {
            return;
        }
        self.sum_a = self.sum_a.wrapping_add(mix(key ^ MIX_SALT_A));
        self.sum_b = self.sum_b.wrapping_add(mix(key ^ MIX_SALT_B));
        self.xor ^= mix(key);
        self.cached_identity = None;
    }

    pub(super) fn identity(&mut self) -> u64 {
        *self.cached_identity.get_or_insert_with(|| {
            let mut framed = [0; 32];
            framed[..8].copy_from_slice(&(self.members.len() as u64).to_le_bytes());
            framed[8..16].copy_from_slice(&self.sum_a.to_le_bytes());
            framed[16..24].copy_from_slice(&self.sum_b.to_le_bytes());
            framed[24..].copy_from_slice(&self.xor.to_le_bytes());
            exact_identity_bytes(COLLECTION_DOMAIN, &framed)
        })
    }
}

fn mix(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: u8) -> u64 {
        exact_identity_bytes(b"test", &[value])
    }

    #[test]
    fn insertion_order_does_not_change_identity() {
        let mut forward = CanonicalCollectionIdentity::default();
        let mut reverse = CanonicalCollectionIdentity::default();
        for value in 1..=32 {
            forward.insert(hash(value));
        }
        for value in (1..=32).rev() {
            reverse.insert(hash(value));
        }
        assert_eq!(forward.identity(), reverse.identity());
    }

    #[test]
    fn duplicate_content_does_not_change_set_identity() {
        let mut identity = CanonicalCollectionIdentity::default();
        identity.insert(hash(1));
        let once = identity.identity();
        identity.insert(hash(1));
        assert_eq!(identity.identity(), once);
    }

    #[test]
    fn different_sets_have_different_test_identities() {
        let mut first = CanonicalCollectionIdentity::default();
        let mut second = CanonicalCollectionIdentity::default();
        for value in 1..=32 {
            first.insert(hash(value));
            second.insert(hash(value + 1));
        }
        assert_ne!(first.identity(), second.identity());
    }
}
