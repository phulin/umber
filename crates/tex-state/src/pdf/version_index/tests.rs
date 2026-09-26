use super::*;

#[test]
fn sparse_keys_and_old_roots_survive_updates() {
    let keys = [0, u64::MAX, 1, 1 << 32, 1 << 63, (1 << 55) | 7];
    let mut index = PdfVersionIndex::default();
    let mut root = PdfVersionRoot::default();
    for (value, key) in keys.into_iter().enumerate() {
        let old = root;
        root = index.insert(root, key, value as u32, false);
        assert_eq!(index.get(old, key), None);
        assert_eq!(index.get(root, key), Some(value as u32));
    }
    let old = root;
    root = index.insert(root, 1 << 32, 77, false);
    assert_eq!(index.get(old, 1 << 32), Some(3));
    assert_eq!(index.get(root, 1 << 32), Some(77));
    for (value, key) in keys.into_iter().enumerate() {
        if key != 1 << 32 {
            assert_eq!(index.get(root, key), Some(value as u32));
        }
    }
}

#[test]
fn candidate_reject_and_accept_preserve_accepted_roots() {
    let mut index = PdfVersionIndex::default();
    let accepted = index.insert(PdfVersionRoot::default(), 0, 1, false);
    let rejected = index.insert(accepted, u64::MAX, 2, true);
    assert_eq!(index.get(rejected, u64::MAX), Some(2));
    index.reject_candidate();
    assert_eq!(index.get(accepted, 0), Some(1));
    let candidate = index.insert(accepted, u64::MAX, 3, true);
    index.accept_candidate();
    assert_eq!(index.get(candidate, u64::MAX), Some(3));
    assert_eq!(index.get(accepted, u64::MAX), None);
    let fork = index.insert(accepted, 1 << 63, 4, false);
    assert_eq!(index.get(fork, 1 << 63), Some(4));
    assert_eq!(index.get(candidate, 1 << 63), None);
}

#[test]
fn node_growth_tracks_real_branches() {
    let mut index = PdfVersionIndex::default();
    let mut root = PdfVersionRoot::default();
    for key in 0..1024 {
        root = index.insert(root, key, key as u32, false);
    }
    assert!(
        index.accepted.len() < 16_000,
        "{} nodes",
        index.accepted.len()
    );
    assert!(
        index.allocated_bytes() < 25_000 * std::mem::size_of::<Node>(),
        "{} reserved bytes for 1,024 distinct keys",
        index.allocated_bytes()
    );
    assert_eq!(index.get(root, 1023), Some(1023));
    assert_eq!(index.get(root, 1024), None);

    let warmed_nodes = index.accepted.len();
    for value in 0..10_000 {
        root = index.insert(root, 777, value, false);
    }
    assert!(
        index.accepted.len() - warmed_nodes < 160_000,
        "{} nodes for 10,000 warmed updates",
        index.accepted.len() - warmed_nodes
    );
    assert_eq!(index.get(root, 777), Some(9_999));
    assert!(
        index.allocated_bytes() < 200_000 * std::mem::size_of::<Node>(),
        "{} reserved bytes after warmed updates",
        index.allocated_bytes()
    );
}

#[test]
fn retained_roots_match_ordered_map_for_clustered_and_high_keys() {
    use std::collections::BTreeMap;

    let mut index = PdfVersionIndex::default();
    let mut root = PdfVersionRoot::default();
    let mut expected = BTreeMap::new();
    let mut roots = Vec::new();
    let mut random = 0x6e32_1b7d_a9f4_c853u64;
    for step in 0..400u32 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let row = (random as u32) & 63;
        let key = match step % 4 {
            0 => row as u64,
            1 => (7_u64 << 56) | row as u64,
            2 => (5_u64 << 56) | (1_u64 << 55) | row as u64,
            _ => [0, u64::MAX, 1 << 63, 1 << 32][row as usize % 4],
        };
        root = index.insert(root, key, step, false);
        expected.insert(key, step);
        roots.push((root, expected.clone()));
    }
    for (root, expected) in roots {
        for (key, value) in &expected {
            assert_eq!(index.get(root, *key), Some(*value));
        }
        for key in [u64::MAX - 1, 1 << 62, (6_u64 << 56) | 999] {
            assert_eq!(index.get(root, key), expected.get(&key).copied());
        }
    }
}
