use super::*;

fn entries() -> Vec<(Vec<u8>, u32)> {
    vec![
        (b"gamma".to_vec(), 2),
        (b"alpha".to_vec(), 0),
        (b"beta".to_vec(), 1),
    ]
}

#[test]
fn generation_is_deterministic_and_lookup_equivalent() {
    let first = encode(entries()).expect("encode lookup");
    let mut reversed = entries();
    reversed.reverse();
    assert_eq!(encode(reversed).expect("encode reordered lookup"), first);

    let lookup = decode(&first, 3).expect("decode lookup");
    assert_eq!(lookup.get(b"alpha"), Some(0));
    assert_eq!(lookup.get(b"beta"), Some(1));
    assert_eq!(lookup.get(b"gamma"), Some(2));
    assert_eq!(lookup.get(b"missing"), None);
    lookup
        .spot_check(0x1234_5678_9abc_def0)
        .expect("spot checks");
}

#[test]
fn complete_structure_and_bounds_are_validated() {
    let valid = encode(entries()).expect("encode lookup");
    for offset in [0, 4, 8, 16, 20, 24, 28, HEADER_LEN] {
        let mut corrupt = valid.clone();
        corrupt[offset] ^= 1;
        assert!(decode(&corrupt, 3).is_err(), "offset {offset} accepted");
    }

    let mut bad_target = valid;
    let entries_offset = HEADER_LEN + read_u32(&bad_target, 16) as usize * 4;
    put_u32(&mut bad_target, entries_offset + 8, 3);
    assert!(decode(&bad_target, 3).is_err());
}

#[test]
fn duplicate_complete_keys_are_rejected() {
    assert_eq!(
        encode([(b"same".to_vec(), 0), (b"same".to_vec(), 1)]),
        Err("duplicate frozen lookup key")
    );
}
